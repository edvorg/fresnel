//! The settings file `config.toml` (plan section 12.5): the core reads it at its start,
//! gives its keys to the view and tells each problem with its line and column. It never
//! writes the file.

use fresnel_core::app::{CoreSettings, Startup};
use fresnel_core::commands::{CommandId, Shortcut};
use fresnel_core::journal::Level;

use super::{App, CONFIG_FILE, home, write_kubeconfig};

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
    assert_eq!(home.read(CONFIG_FILE), text, "the core never writes the file");
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
