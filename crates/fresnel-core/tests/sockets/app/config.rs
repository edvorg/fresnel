//! The settings file `config.toml` (plan section 12.5): the core reads it at its start and
//! after each change, gives its settings to the view and to its requests, and tells each
//! problem with its line and column. Only a save of the settings view writes the file.

use fresnel_core::app::{Command, CoreSettings, CoreView, Health, Startup};
use fresnel_core::cluster::ClusterId;
use fresnel_core::commands::{CommandId, Shortcut};
use fresnel_core::config::{Edit, EditValue, FileRead, Theme};
use fresnel_core::journal::Level;
use fresnel_core::kinds::Kind;
use fresnel_testkit::FakeApiServer;

use super::{App, CONFIG_FILE, KUBECONFIG, home, write_kubeconfig};
use crate::stores::PODS;

fn key(text: &str) -> Shortcut {
    Shortcut::parse(text).expect("a shortcut")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_start_gives_the_keys_of_the_file_and_each_unknown_key_is_a_warning_with_its_place() {
    let home = home("config-keys");
    write_kubeconfig(&home, "a", &[]);
    let text = "version = 1\nzoom = 2\n[shortcuts]\nback = \"Alt+Left\"\n";
    home.write(CONFIG_FILE, text).expect("the settings file");
    let app = App::started(&home, CoreSettings::default());
    let view = app.until("the end of the start", |view| view.startup == Startup::Ready).await;
    assert_eq!(view.config.keys.of(CommandId::Back), Some(key("Alt+Left")));
    assert_eq!(view.config.keys.of(CommandId::QuickSearch), Some(key("Cmd+K")), "a default");

    let file = home.path(CONFIG_FILE).display().to_string();
    let warning = format!("{file}: line 2, column 1: unknown key `zoom`; the app ignores it");
    assert!(app.journal().contains(&(Level::Warning, "app".to_owned(), warning)), "{:#?}", app.journal());
    assert_eq!(home.read(CONFIG_FILE), text, "the core writes the file only for the settings view");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_with_a_parse_error_gives_the_defaults_and_an_error_with_its_place() {
    let home = home("config-broken");
    write_kubeconfig(&home, "a", &[]);
    let text = "version = 1\n[shortcuts\nback = \"Alt+Left\"\n";
    home.write(CONFIG_FILE, text).expect("the settings file");
    let app = App::started(&home, CoreSettings::default());
    let view = app.until("the end of the start", |view| view.startup == Startup::Ready).await;
    assert_eq!(view.config.keys.of(CommandId::Back), Some(key("Cmd+[")), "the default");

    let file = home.path(CONFIG_FILE).display().to_string();
    let errors: Vec<String> = app
        .journal()
        .into_iter()
        .filter(|(level, _, text)| *level == Level::Error && text.starts_with(&file))
        .map(|(_, _, text)| text)
        .collect();
    let [error] = errors.as_slice() else { panic!("one error of the file: {:#?}", app.journal()) };
    assert!(error.starts_with(&format!("{file}: line 2, column 11: ")), "{error}");
    assert!(error.ends_with("; the app uses no setting of the file"), "{error}");
    assert_eq!(home.read(CONFIG_FILE), text);
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_file_each_setting_has_its_default() {
    let home = home("config-absent");
    write_kubeconfig(&home, "a", &[]);
    let app = App::started(&home, CoreSettings::default());
    let view = app.until("the end of the start", |view| view.startup == Startup::Ready).await;
    assert_eq!(view.config.keys.of(CommandId::Back), Some(key("Cmd+[")));
    let file = home.path(CONFIG_FILE).display().to_string();
    let note = format!("{file}: no file; each setting has its default");
    assert!(app.journal().contains(&(Level::Info, "app".to_owned(), note)), "{:#?}", app.journal());
    assert!(!home.path(CONFIG_FILE).exists(), "the core writes no file");
}

/// The numbers of the file reach the requests of the core: a number outside its bounds
/// takes the nearest bound with a note at its place, and a period of 0 is refused.
#[tokio::test(flavor = "multi_thread")]
async fn the_numbers_of_the_file_reach_the_requests_and_each_clamp_has_a_note_at_its_place() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("config-numbers");
    write_kubeconfig(&home, "a", &[("a", server.addr(), &server, false)]);
    let text =
        "version = 1\n[network]\nlist_page_size = 10\n[events]\nmode = \"warnings\"\n[metrics]\nperiod_secs = 0\n";
    home.write(CONFIG_FILE, text).expect("the settings file");
    let app = App::started(&home, CoreSettings::default());
    app.until_health("a", Health::Live).await;
    let lists = |path: &str| -> Vec<String> {
        let seen = server.requests().into_iter().filter(|request| request.uri.path() == path && !request.is_watch());
        seen.map(|request| request.uri.query().unwrap_or_default().to_owned()).collect()
    };
    let events = Kind::Event.info().path();
    app.until("the lists of the pods and the Events", |_| !lists(PODS).is_empty() && !lists(&events).is_empty()).await;
    assert!(lists(PODS).iter().all(|query| query.contains("limit=50")), "{:?}", lists(PODS));
    assert!(lists(&events).iter().all(|query| query.contains("fieldSelector=type%3DWarning")), "{:?}", lists(&events));

    let file = home.path(CONFIG_FILE).display().to_string();
    let warnings: Vec<String> = app
        .journal()
        .into_iter()
        .filter(|(level, _, text)| *level == Level::Warning && text.starts_with(&file))
        .map(|(_, _, text)| text.replacen(&file, "", 1))
        .collect();
    assert_eq!(
        warnings,
        [
            ": line 3, column 18: `network.list_page_size` = 10 is below its lowest value; the app uses 50",
            ": line 7, column 15: `metrics.period_secs` = 0 is refused: a period or a time limit of 0 never ends its \
             wait; the app keeps 15 s",
        ]
    );
}

/// The core follows the kubeconfig files and the settings file: a new context, a file of
/// the settings and a new key act with no restart. A file that does not parse leaves the
/// settings of the last read, so a mistake in an editor changes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_change_of_a_file_acts_with_no_restart_and_a_broken_settings_file_keeps_the_last_settings() {
    let home = home("config-watch");
    write_kubeconfig(&home, "a", &[]);
    let fast = "version = 1\n[kubeconfig]\nwatch_secs = 1\nfiles = [\"~/work/extra.yaml\"]\n";
    home.write(CONFIG_FILE, fast).expect("the settings file");
    let app = App::started(&home, CoreSettings::default());
    app.until("the end of the start", |view| view.startup == Startup::Ready).await;
    let names = |view: &CoreView| view.clusters.iter().map(|row| row.id.to_string()).collect::<Vec<_>>();

    home.write(KUBECONFIG, "contexts:\n- {name: a, context: {}}\n- {name: b, context: {}}\n").expect("a new context");
    let view = app.until("the context b", |view| names(view).contains(&"b".to_owned())).await;
    assert_eq!(names(&view), ["a", "b"]);
    home.write("work/extra.yaml", "contexts:\n- {name: c, context: {}}\n").expect("the extra file");
    app.until("the context of the extra file", |view| names(view) == ["a", "b", "c"]).await;

    home.write(CONFIG_FILE, &format!("{fast}[shortcuts]\nback = \"Alt+Left\"\n")).expect("a new key");
    app.until("the new key", |view| view.config.keys.of(CommandId::Back) == Some(key("Alt+Left"))).await;
    home.write(CONFIG_FILE, &format!("{fast}[shortcuts\nback = \"Alt+Right\"\n")).expect("a broken file");
    let file = home.path(CONFIG_FILE).display().to_string();
    let broken = format!("{file}: line 5, column 11: ");
    let kept = "; the app keeps the settings of its last read";
    let errors = app.until_texts("app", |texts| texts.iter().any(|text| text.starts_with(&broken))).await;
    assert!(errors.iter().any(|text| text.starts_with(&broken) && text.ends_with(kept)), "{errors:#?}");
    let view = app.core.view();
    assert_eq!(view.config.keys.of(CommandId::Back), Some(key("Alt+Left")), "the settings of the last read");
    assert!(matches!(&view.config_file.read, FileRead::Read { used: false, .. }), "{:?}", view.config_file);
    assert!(view.config_file.read.refusal().is_some_and(|refusal| refusal.contains("line 5, column 11")));
}

/// The settings view writes its changes into the file and keeps each comment. The app
/// never writes over a file that it cannot read.
#[tokio::test(flavor = "multi_thread")]
async fn a_save_of_the_settings_view_keeps_the_comments_and_a_file_that_does_not_parse_stays() {
    let home = home("config-edit");
    write_kubeconfig(&home, "a", &[]);
    let text = "# my settings\nversion = 1\n[kubeconfig]\nwatch_secs = 1 # fast\n";
    home.write(CONFIG_FILE, text).expect("the settings file");
    let app = App::started(&home, CoreSettings::default());
    app.until("the end of the start", |view| view.startup == Startup::Ready).await;

    let edits = vec![
        Edit::new(&["appearance", "theme"], EditValue::Text("light".to_owned())),
        Edit::new(&["kubeconfig", "watch_secs"], EditValue::Number(2)),
        Edit::new(&["clusters", "a", "read_only"], EditValue::Flag(true)),
    ];
    app.core.send(Command::EditSettings(edits));
    let saved = |number| {
        move |view: &CoreView| {
            view.config_file.save.as_ref().is_some_and(|save| save.number == number && save.result.is_some())
        }
    };
    let view = app.until("the save", saved(1)).await;
    assert_eq!(view.config_file.save.as_ref().and_then(|save| save.result.clone()), Some(Ok(())));
    assert_eq!(
        home.read(CONFIG_FILE),
        "# my settings\nversion = 1\n[kubeconfig]\nwatch_secs = 2 # fast\n\n[appearance]\ntheme = \"light\"\n\n\
         [clusters.a]\nread_only = true\n"
    );
    assert_eq!(view.config.theme(), Theme::Light, "the view has the new settings at once");
    assert!(view.config.read_only(&ClusterId::new("a")));

    let broken = "version = 1\n[appearance\n";
    home.write(CONFIG_FILE, broken).expect("a broken file");
    app.until("the read of the broken file", |view| {
        matches!(view.config_file.read, FileRead::Read { used: false, .. })
    })
    .await;
    app.core.send(Command::EditSettings(vec![Edit::new(&["appearance", "theme"], EditValue::Text("dark".to_owned()))]));
    let view = app.until("the refused save", saved(2)).await;
    let result = view.config_file.save.as_ref().and_then(|save| save.result.clone());
    assert!(matches!(&result, Some(Err(error)) if error.contains("line 2, column 12")), "{result:?}");
    assert_eq!(home.read(CONFIG_FILE), broken, "the file stays as it is");
    assert_eq!(view.config.theme(), Theme::Light, "the settings of the last read stay");
}
