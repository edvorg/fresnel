use super::*;
use crate::config::table::Setting;
use crate::config::values::Value;

fn number(path: &[&str], value: i64) -> Edit {
    Edit::new(path, EditValue::Number(value))
}

#[test]
fn a_change_keeps_each_comment_and_each_other_line_of_the_file() {
    let text = "# The settings of my Mac.\nversion = 1\n\n# Slow hotel network.\n[network]\nkeepalive_secs = 10 # measured\nlist_page_size = 200\n\n[shortcuts]\nback = \"Alt+Left\" # like a browser\n";
    let edits = [
        number(&["network", "keepalive_secs"], 20),
        number(&["metrics", "period_secs"], 30),
        Edit::new(&["shortcuts", "forward"], EditValue::Text("Alt+Right".to_owned())),
    ];
    let changed = apply(text, &edits).expect("a change");
    assert_eq!(
        changed,
        "# The settings of my Mac.\nversion = 1\n\n# Slow hotel network.\n[network]\nkeepalive_secs = 20 # measured\nlist_page_size = 200\n\n[shortcuts]\nback = \"Alt+Left\" # like a browser\nforward = \"Alt+Right\"\n\n[metrics]\nperiod_secs = 30\n"
    );
    let loaded = parse(&changed);
    assert!(loaded.used && loaded.problems.is_empty(), "{:?}", loaded.problems);
    assert_eq!(loaded.config.number(Setting::Keepalive), 20);
    assert_eq!(loaded.config.set(Setting::PageSize), Some(&Value::Number(200)));
}

#[test]
fn a_new_file_gets_its_version_and_a_cluster_gets_a_table_with_no_empty_header() {
    let edits = [
        Edit::new(&["clusters", "ams prod", "alias"], EditValue::Text("AMS".to_owned())),
        Edit::new(&["clusters", "ams prod", "read_only"], EditValue::Flag(true)),
        Edit::new(&["groups", "europe"], EditValue::List(vec!["ams prod".to_owned(), "fra".to_owned()])),
    ];
    let changed = apply("", &edits).expect("a change");
    assert_eq!(
        changed,
        "version = 1\n\n[clusters.\"ams prod\"]\nalias = \"AMS\"\nread_only = true\n\n[groups]\neurope = [\"ams prod\", \"fra\"]\n"
    );
    let config = parse(&changed).config;
    let options = config.clusters.get("ams prod").expect("the options");
    assert_eq!((options.alias.as_deref(), options.read_only), (Some("AMS"), Some(true)));
    assert_eq!(config.groups.first().map(|group| group.clusters.len()), Some(2));
}

#[test]
fn a_removed_key_gives_the_default_and_a_key_that_is_absent_changes_nothing() {
    let text = "version = 1\n[appearance]\ntheme = \"light\" # bright office\ntime_format = \"12h\"\n";
    let removed = [Edit::new(&["appearance", "theme"], EditValue::Remove)];
    let changed = apply(text, &removed).expect("a change");
    assert_eq!(changed, "version = 1\n[appearance]\ntime_format = \"12h\"\n");
    assert_eq!(parse(&changed).config.text(Setting::Theme), "system");
    let absent = [Edit::new(&["clusters", "none", "alias"], EditValue::Remove)];
    assert_eq!(apply(text, &absent).as_deref(), Ok(text), "no table for a remove");
}

#[test]
fn the_app_writes_no_file_that_it_cannot_read_and_no_change_that_the_reader_refuses() {
    let edit = [number(&["metrics", "period_secs"], 30)];
    let broken = apply("version = 1\n[metrics\nperiod_secs = 10\n", &edit).expect_err("a parse error");
    assert!(broken.starts_with("the file has a problem at line 2, column 9: "), "{broken}");
    assert!(broken.ends_with("; the app writes no file that it cannot read"), "{broken}");
    let newer = apply("version = 2\n", &edit).expect_err("an other version");
    assert!(newer.contains("this app reads the version 1 only"), "{newer}");
    let not_a_table = apply("version = 1\nmetrics = 3\n", &edit).expect_err("a value in place of a table");
    assert_eq!(not_a_table, "`metrics.period_secs`: `metrics` is no table in the file");
    // A change that the reader of the new text refuses.
    let refused = apply("version = 1\n", &[number(&["version"], 2)]).expect_err("an other version");
    assert!(
        refused.starts_with("the change has a problem at line 1, column 1: the file has the version 2"),
        "{refused}"
    );
}

#[test]
fn a_write_replaces_the_file_in_one_step_and_a_refused_write_leaves_it() {
    let folder = std::env::temp_dir().join(format!("fresnel-config-edit-{}", std::process::id()));
    let path = folder.join("config.toml");
    write(&path, &[number(&["network", "list_page_size"], 100)]).expect("a write");
    assert_eq!(fs::read_to_string(&path).expect("the file"), "version = 1\n\n[network]\nlist_page_size = 100\n");
    assert!(!path.with_extension("toml.new").exists(), "the second file took the name");
    fs::write(&path, "version = 1\n[network\n").expect("a broken file");
    let refused = write(&path, &[number(&["network", "list_page_size"], 200)]);
    assert!(refused.is_err());
    assert_eq!(fs::read_to_string(&path).expect("the file"), "version = 1\n[network\n", "the file stays");
    fs::remove_dir_all(&folder).expect("the test folder");
}
