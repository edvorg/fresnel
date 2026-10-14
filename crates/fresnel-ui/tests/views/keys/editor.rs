//! The editor of the keys: the read of the values while the panel is open, show and hide,
//! copy, an edit with its diff before the save, and the save with only the changed key.

use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::Command;
use fresnel_core::cluster::ClusterId;
use fresnel_core::keys::{KeyKind, KeyWrite, Place, Value, WriteState};

use super::scene::{config_map, give, loaded, open, read, saving, secret, target, with_config_maps, with_secrets};
use super::{click, copied, save_command, typed};
use crate::fake::{shows, view, window};
use crate::kinds::in_panel;

const PASSWORD: &[u8] = b"made-up-password\n";

#[test]
fn the_panel_of_a_secret_reads_its_values_and_hides_each_one_until_the_user_shows_it() {
    let secrets = [secret("db", "Opaque", &["password", "username"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    open(&mut window, "Secrets", "db");
    let db = target("ams-prod", KeyKind::Secret, "db");
    let rules = Command::ReadRules { cluster: ClusterId::new("ams-prod"), namespace: "scene".into() };
    let sent = &window.state().sent;
    assert!(sent.contains(&Command::ShowKeys(Some(db.clone()))) && sent.contains(&rules), "{sent:?}");
    assert!(in_panel(&window, "The values are read from ams-prod…"));

    let values = [("password", Place::Data, PASSWORD), ("username", Place::Data, b"user-a".as_slice())];
    give(&mut window, "ams-prod", Some(loaded(read(&db, "4711", &values))));
    assert!(
        in_panel(&window, "password") && in_panel(&window, "username") && in_panel(&window, "version 4711 of ams-prod")
    );
    assert!(in_panel(&window, "type Opaque · 2 keys"));
    assert!(in_panel(&window, "no line feed at the end"), "the mark of `user-a`");
    assert!(window.query_by_label("Value of password").is_none(), "a value of a Secret is hidden");

    click(&mut window, "Show the value of password");
    let shown = window.get_by_label("Value of password").value();
    assert!(shown.as_deref() == Some("made-up-password\n"), "the decoded value");
    click(&mut window, "Hide the value of password");
    assert!(window.query_by_label("Value of password").is_none());

    // The panel closes: the core drops the values (plan section 9.4).
    click(&mut window, "Close the details");
    let sent = &window.state().sent;
    let opened = sent.iter().position(|command| *command == Command::ShowKeys(Some(db.clone())));
    let closed = sent.iter().position(|command| *command == Command::ShowKeys(None));
    assert!(matches!((opened, closed), (Some(opened), Some(closed)) if opened < closed), "{sent:?}");
}

#[test]
fn copy_gives_the_text_of_a_value_and_the_base64_of_a_binary_value() {
    let secrets = [secret("db", "Opaque", &["password"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    open(&mut window, "Secrets", "db");
    let db = target("ams-prod", KeyKind::Secret, "db");
    give(&mut window, "ams-prod", Some(loaded(read(&db, "1", &[("password", Place::Data, PASSWORD)]))));
    window.get_by_label("Copy the value of password").click();
    window.step();
    assert!(copied(&window).as_deref() == Some("made-up-password\n"), "the hidden value goes to the clipboard");
    window.run();
    assert!(in_panel(&window, "Copied the value of `password`."));

    let maps = [config_map("assets", &["color"], &["logo"])];
    let mut window = crate::fake::window(view(vec![with_config_maps("ams-prod", &maps)]));
    open(&mut window, "ConfigMaps", "assets");
    let assets = target("ams-prod", KeyKind::ConfigMap, "assets");
    let png = b"\x89PNG\r\n\x1a\n\x00\x01".as_slice();
    let values = [("color", Place::Data, b"blue".as_slice()), ("logo", Place::BinaryData, png)];
    give(&mut window, "ams-prod", Some(loaded(read(&assets, "1", &values))));
    // The actions of the header put the row low in the panel: it scrolls to it first.
    window.get_by_label("Copy the value of logo").scroll_to_me();
    window.run();
    window.get_by_label("Copy the value of logo").click();
    window.step();
    assert_eq!(copied(&window).as_deref(), Some("iVBORw0KGgoAAQ=="));
    window.run();
    assert!(in_panel(&window, "Copied the value of `logo` as base64."));
}

/// Plan section 12.2 and research 08 section 8.3: the diff shows before the save, also a
/// line feed at the end that the edit removed, and the save names the changed key only.
#[test]
fn an_edit_shows_its_diff_and_the_save_sends_only_that_key_with_the_version_of_the_read() {
    let secrets = [secret("db", "Opaque", &["password", "username"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    open(&mut window, "Secrets", "db");
    let db = target("ams-prod", KeyKind::Secret, "db");
    let values = [("password", Place::Data, PASSWORD), ("username", Place::Data, b"user-a".as_slice())];
    let first = loaded(read(&db, "4711", &values));
    give(&mut window, "ams-prod", Some(first.clone()));
    click(&mut window, "Show the value of password");
    typed(&mut window, "Value of password", "made-up-password-2");
    assert!(in_panel(&window, "changed") && shows(&window, "Save 1 key…"));

    click(&mut window, "Save 1 key…");
    let target_texts = ["Save 1 key of this Secret?", "Cluster", "ams-prod", "Namespace", "scene", "Name", "db"];
    assert!(target_texts.iter().all(|text| shows(&window, text)), "the dialog names its object");
    assert!(super::scene::shows_part(&window, "the line feed at the end goes away"));
    assert!(!shows(&window, "- made-up-password"), "the values of a Secret are hidden in the diff");
    click(&mut window, "Show the values");
    assert!(shows(&window, "- made-up-password") && shows(&window, "+ made-up-password-2"));
    click(&mut window, "Save 1 key");

    let save = save_command(&window).expect("a save");
    assert_eq!((&save.target, &*save.version), (&db, "4711"));
    let new = Value::new(b"made-up-password-2".to_vec());
    let only = KeyWrite { key: "password".to_owned(), new: Some((Place::Data, new)), old: Some(Place::Data) };
    assert!(save.writes == [only], "the one changed key and no other key");

    give(&mut window, "ams-prod", Some(saving(&first, &save, WriteState::Pending)));
    assert!(shows(&window, "Save of 1 key: pending…"));
    assert!(
        window.get_by_label("Save 1 key…").accesskit_node().is_disabled(),
        "no second save while one is on its way"
    );

    let after = [("password", Place::Data, b"made-up-password-2".as_slice()), ("username", Place::Data, b"user-a")];
    let answer = read(&db, "4712", &after);
    let mut done = saving(&first, &save, WriteState::Done(answer.clone()));
    done.object = Some(answer);
    give(&mut window, "ams-prod", Some(done));
    assert!(shows(&window, "Save of 1 key: done. The Secret has version 4712."));
    assert!(!in_panel(&window, "changed") && shows(&window, "Save"), "the answer is the new read, with no change");
}

#[test]
fn a_change_back_to_the_value_of_the_read_is_no_change() {
    let maps = [config_map("web", &["mode"], &[])];
    let mut window = window(view(vec![with_config_maps("ams-prod", &maps)]));
    open(&mut window, "ConfigMaps", "web");
    let web = target("ams-prod", KeyKind::ConfigMap, "web");
    give(&mut window, "ams-prod", Some(loaded(read(&web, "7", &[("mode", Place::Data, b"fast\n")]))));
    assert!(window.query_by_label("Value of mode").is_some(), "a ConfigMap shows its values");
    typed(&mut window, "Value of mode", "slow\n");
    assert!(shows(&window, "Save 1 key…"));
    typed(&mut window, "Value of mode", "fast\n");
    assert!(!in_panel(&window, "changed") && window.get_by_label("Save").accesskit_node().is_disabled());
}

/// Plan section 11.5: the menu of a row runs the actions of the keys. "Add key" opens the
/// panel with the focus in the name of a new key.
#[test]
fn the_menu_of_a_row_adds_a_key_with_the_focus_in_its_name() {
    let maps = [config_map("web", &["mode"], &[])];
    let mut window = window(view(vec![with_config_maps("ams-prod", &maps)]));
    crate::fake::open_page(&mut window, "ConfigMaps");
    crate::fake::until(&mut window, "the row", |window| shows(window, "web"));
    window.get_by_label("web").click_secondary();
    window.run();
    for action in ["Edit keys…", "Add key…", "Remove key…"] {
        assert!(!window.get_by_label_contains(action).accesskit_node().is_disabled(), "{action} runs");
    }
    window.get_by_label_contains("Add key…").click();
    window.run();
    let web = target("ams-prod", KeyKind::ConfigMap, "web");
    give(&mut window, "ams-prod", Some(loaded(read(&web, "7", &[("mode", Place::Data, b"fast\n")]))));
    assert!(window.get_by_label("Name of the new key").is_focused(), "the focus is in the name of a new key");
}
