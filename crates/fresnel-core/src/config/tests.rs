use std::path::PathBuf;

use super::table::Setting;
use super::*;
use crate::commands::{CommandId, Shortcut};
use crate::kinds::Action;

fn key(text: &str) -> Shortcut {
    Shortcut::parse(text).expect("a shortcut")
}

/// The problems as the journal shows them.
fn texts(loaded: &Loaded) -> Vec<String> {
    loaded.problems.iter().map(ToString::to_string).collect()
}

#[test]
fn a_file_gives_the_key_of_each_command_and_the_others_keep_their_default() {
    let text = "version = 1\n\n[shortcuts]\nback = \"Alt+Left\"\nfilter = \"\"\ndelete = \"cmd+backspace\"\n";
    let loaded = parse(text);
    assert_eq!(texts(&loaded), Vec::<String>::new());
    assert!(loaded.used);
    let keys = &loaded.config.keys;
    assert_eq!(keys.of(CommandId::Back), Some(key("Alt+Left")));
    assert_eq!(keys.of(CommandId::Filter), None, "an empty text removes the key");
    assert_eq!(keys.of(CommandId::Object(Action::Delete)), Some(key("Cmd+Backspace")));
    assert_eq!(keys.of(CommandId::QuickSearch), Some(key("Cmd+K")), "the default");
}

#[test]
fn each_unknown_key_and_each_value_that_does_not_fit_is_a_problem_at_its_line_and_column() {
    let text = "version = 1\ntheme = \"dark\"\n\n[shortcuts]\nback = \"Alt+Left\"\n  jump = \"Cmd+J\"\nforward = 7\n\
                quick-search = \"Cmd+Hyper+K\"\n\n[shortcut]\nx = 1\n";
    let loaded = parse(text);
    assert_eq!(
        texts(&loaded),
        [
            "line 2, column 1: unknown key `theme`; the app ignores it",
            "line 6, column 3: unknown command `jump` in `shortcuts`; the app ignores it",
            "line 7, column 11: the key of `forward` is a text, for example \"Cmd+K\"",
            "line 8, column 16: the key of `quick-search`: `Hyper` is not a modifier: use Cmd, Ctrl, Alt or Shift",
            "line 10, column 2: unknown key `shortcut`; the app ignores it",
        ]
    );
    assert!(loaded.used, "the other keys stay in use");
    assert_eq!(loaded.config.keys.of(CommandId::Back), Some(key("Alt+Left")));
    assert_eq!(loaded.config.keys.of(CommandId::QuickSearch), Some(key("Cmd+K")), "a bad key keeps the default");
}

#[test]
fn a_parse_error_has_its_line_and_column_and_the_app_uses_no_setting_of_the_file() {
    let loaded = parse("version = 1\n[shortcuts]\nback = \"Alt+Left\n");
    assert!(!loaded.used);
    assert_eq!(loaded.config, Config::default());
    let [problem] = loaded.problems.as_slice() else { panic!("one problem: {:?}", loaded.problems) };
    // The parser names the place where the end of the text was due.
    assert_eq!((problem.line, problem.column), (3, 17), "{problem}");

    let loaded = parse("version = 1\nversion = 1\n");
    assert_eq!(loaded.problems.iter().map(|problem| problem.line).collect::<Vec<_>>(), [2], "a key two times");
}

#[test]
fn a_file_of_a_newer_version_is_not_used_and_a_file_with_no_version_is_read_as_version_1() {
    let newer = parse("version = 2\n[shortcuts]\nback = \"Alt+Left\"\n");
    assert!(!newer.used);
    assert_eq!(newer.config, Config::default());
    assert_eq!(texts(&newer), ["line 1, column 1: the file has the version 2, and this app reads the version 1 only"]);

    let none = parse("[shortcuts]\nback = \"Alt+Left\"\n");
    assert!(none.used);
    assert_eq!(texts(&none), ["line 1, column 1: the file has no `version`; the app reads it as version 1"]);
    assert_eq!(none.config.keys.of(CommandId::Back), Some(key("Alt+Left")));

    let text = parse("version = \"1\"\n");
    assert_eq!(texts(&text), ["line 1, column 11: `version` is a whole number, for example 1"]);
    assert!(text.used);
}

#[test]
fn two_commands_with_one_key_give_the_key_to_the_first_one_in_the_file() {
    let text = "version = 1\n[shortcuts]\nforward = \"Cmd+J\"\nback = \"Cmd+J\"\n";
    let loaded = parse(text);
    assert_eq!(texts(&loaded), ["line 4, column 1: Cmd+J is the key of `forward` already, so `back` has no key"]);
    assert_eq!(loaded.config.keys.of(CommandId::Forward), Some(key("Cmd+J")));
    assert_eq!(loaded.config.keys.of(CommandId::Back), None);
}

#[test]
fn an_absent_file_gives_no_settings_and_a_folder_in_its_place_is_an_error() {
    let folder = std::env::temp_dir().join(format!("fresnel-core-test-config-{:08x}", fastrand::u32(..)));
    std::fs::create_dir_all(&folder).expect("a folder");
    let path: PathBuf = folder.join(FILE_NAME);
    assert_eq!(load(&path), Ok(None));
    std::fs::create_dir(&path).expect("a folder with the name of the file");
    assert!(load(&path).is_err());
    std::fs::remove_dir(&path).expect("the folder goes");
    std::fs::write(&path, "version = 1\n").expect("a file");
    assert_eq!(load(&path).map(|loaded| loaded.map(|loaded| loaded.used)), Ok(Some(true)));
    std::fs::remove_dir_all(&folder).expect("the folder of the test goes");
}

/// A file of thousands of unknown keys gives the journal a bounded count of entries.
#[test]
fn the_problems_of_a_file_have_a_limit_and_a_count_of_the_rest() {
    let keys: String = (0..1_000).map(|number| format!("key-{number} = 1\n")).collect();
    let loaded = parse(&format!("version = 1\n{keys}"));
    let texts = texts(&loaded);
    assert_eq!(texts.len(), 21);
    assert_eq!(texts.first().map(String::as_str), Some("line 2, column 1: unknown key `key-0`; the app ignores it"));
    let rest = "line 22, column 1: 980 more problems from here; the journal shows the first 20 only";
    assert_eq!(texts.last().map(String::as_str), Some(rest));
    assert!(loaded.used);
}

#[test]
fn a_number_outside_its_bounds_takes_the_nearest_bound_and_a_period_of_0_is_refused() {
    let text = "version = 1\n[metrics]\nperiod_secs = 0\nmemory_mib = 4096\n[network]\nlist_page_size = 10\n\
                keepalive_secs = \"ten\"\nwatch_timeout_spread_secs = 0\nhedge_min_ms = 0x64\n";
    let loaded = parse(text);
    assert_eq!(
        texts(&loaded),
        [
            "line 3, column 15: `metrics.period_secs` = 0 is refused: a period or a time limit of 0 never ends its \
             wait; the app keeps 15 s",
            "line 4, column 14: `metrics.memory_mib` = 4096 is above its highest value; the app uses 1024 MiB",
            "line 6, column 18: `network.list_page_size` = 10 is below its lowest value; the app uses 50",
            "line 7, column 18: `network.keepalive_secs` is a whole number, from 1 s to 60 s",
        ]
    );
    let config = &loaded.config;
    assert_eq!((config.set(Setting::MetricsPeriod), config.number(Setting::MetricsPeriod)), (None, 15));
    assert_eq!((config.number(Setting::MetricsMemory), config.number(Setting::PageSize)), (1024, 50));
    assert_eq!(config.set(Setting::WatchSpread), Some(&Value::Number(0)), "0 is a value of a number that is no period");
    assert_eq!(config.number(Setting::HedgeMin), 100, "a number in hex");
}

#[test]
fn each_form_of_a_group_has_its_problem_and_a_good_value_its_place() {
    let text = "version = 1\n[appearance]\ntheme = \"blue\"\ntime_zone = \"Mars/Base\"\ntime_format = \"12h\"\n\
                [kubeconfig]\nfiles = [\"~/work.yaml\", 3]\n[plugins]\nfolders = [\"~/bin\"]\nshell = \"zsh\"\n\
                [safety]\nread_only = \"yes\"\n[events]\nmode = \"warnings\"\n[table]\nrow_height = 30\n";
    let loaded = parse(text);
    assert_eq!(
        texts(&loaded),
        [
            "line 3, column 9: `appearance.theme` is one of \"system\", \"dark\", \"light\"",
            "line 4, column 13: `appearance.time_zone`: no time zone `Mars/Base`; use local, UTC or a name such as \
             Europe/Amsterdam",
            "line 7, column 9: `kubeconfig.files` is a list of texts, for example [\"a\", \"b\"]",
            "line 10, column 1: unknown key `shell` in `plugins`; the app ignores it",
            "line 12, column 13: `safety.read_only` is true or false",
        ]
    );
    let config = &loaded.config;
    assert_eq!((config.text(Setting::TimeFormat), config.text(Setting::EventsMode)), ("12h", "warnings"));
    assert_eq!(
        (config.list(Setting::PluginFolders), config.number(Setting::RowHeight)),
        (vec!["~/bin".to_owned()], 30)
    );
    assert_eq!(config.text(Setting::TimeZone), "local", "a bad zone keeps the default");
    assert!(parse("version = 1\n[appearance]\ntime_zone = \"Europe/Amsterdam\"\n").problems.is_empty());
}

#[test]
fn the_options_of_each_cluster_and_the_groups_have_their_problems_at_their_places() {
    let text = "version = 1\n[clusters.\"do-ams3-prod\"]\nalias = \"AMS\"\ncolor = \"#f85149\"\nread_only = true\n\
                default_namespace = \"shop\"\nfavorite_namespaces = [\"shop\", \"Kube_System\"]\nconfirm = \"type-name\"\n\
                [clusters.kind]\ncolor = \"red\"\nzoom = 2\nalias = \"\"\n[groups]\neurope = [\"do-ams3-prod\", \"fra\"]\n\
                bad = \"fra\"\n";
    let loaded = parse(text);
    assert_eq!(
        texts(&loaded),
        [
            "line 7, column 23: `clusters.\"do-ams3-prod\".favorite_namespaces`: `Kube_System` is no name of a namespace",
            "line 10, column 9: `clusters.\"kind\".color` is a color such as \"#3fb950\"",
            "line 11, column 1: unknown option `zoom` of a cluster; the options are alias, color, read_only, \
             default_namespace, favorite_namespaces, confirm",
            "line 12, column 9: `clusters.\"kind\".alias` has 1 to 40 characters",
            "line 15, column 7: `groups.\"bad\"` is a list of texts, for example [\"a\", \"b\"]",
        ]
    );
    let prod = loaded.config.clusters.get("do-ams3-prod").expect("the options");
    assert_eq!((prod.alias.as_deref(), prod.color.map(|rgb| rgb.0)), (Some("AMS"), Some([0xf8, 0x51, 0x49])));
    assert_eq!((prod.read_only, prod.confirm), (Some(true), Some(ConfirmRule::TypeName)));
    assert_eq!(
        (prod.default_namespace.as_deref(), prod.favorite_namespaces.clone()),
        (Some("shop"), vec!["shop".to_owned()])
    );
    assert_eq!(loaded.config.clusters.get("kind"), Some(&ClusterOptions::default()));
    let groups: Vec<(&str, usize)> =
        loaded.config.groups.iter().map(|group| (group.name.as_str(), group.clusters.len())).collect();
    assert_eq!(groups, [("europe", 2)]);
}
