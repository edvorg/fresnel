//! Add and remove a key, each state of a save, and a conflict with the server (plan section
//! 12.2, acceptance of M4: add key and remove key work, and a conflict shows as a conflict).

use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::CoreError;
use fresnel_core::keys::{KeyKind, Place, WriteState};
use fresnel_core::net::ErrorClass;

use super::scene::{Values, give, loaded, open, read, saving, secret, shows_part, target, with_secrets};
use super::{click, save_command, typed};
use crate::fake::{Window, shows, view, window};
use crate::kinds::in_panel;

const VALUES: Values<'static> =
    &[("password", Place::Data, b"made-up-password\n"), ("username", Place::Data, b"user-a\n")];

/// The panel of the Secret `db` of ams-prod with the read of `VALUES` at version 4711.
fn opened() -> Window {
    let secrets = [secret("db", "Opaque", &["password", "username"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    open(&mut window, "Secrets", "db");
    let db = target("ams-prod", KeyKind::Secret, "db");
    give(&mut window, "ams-prod", Some(loaded(read(&db, "4711", VALUES))));
    window
}

/// Saves the changes through the dialog.
fn save(window: &mut Window, keys: &str) {
    click(window, &format!("Save {keys}…"));
    click(window, &format!("Save {keys}"));
}

#[test]
fn a_new_key_with_a_bad_name_shows_why_and_add_and_remove_go_into_one_save() {
    let mut window = opened();
    typed(&mut window, "Name of the new key", "a/b");
    click(&mut window, "Add the key");
    assert!(shows(&window, "`a/b`: a key has only letters, digits, `-`, `_` and `.`"));
    assert!(!in_panel(&window, "new"), "no key");

    typed(&mut window, "Name of the new key", "api-token");
    typed(&mut window, "Value of the new key", "made-up-token");
    click(&mut window, "Add the key");
    assert!(in_panel(&window, "api-token") && in_panel(&window, "new"));
    assert!(window.get_by_label("Value of api-token").value().as_deref() == Some("made-up-token"));
    assert!(window.get_by_label("Name of the new key").value().unwrap_or_default().is_empty(), "the row is empty");
    typed(&mut window, "Name of the new key", "password");
    click(&mut window, "Add the key");
    assert!(shows(&window, "the key `password` exists"));

    click(&mut window, "Remove the key username");
    assert!(in_panel(&window, "removed") && window.query_by_label("Undo the change of username").is_some());
    assert!(shows(&window, "Save 2 keys…"));
    save(&mut window, "2 keys");
    let sent = save_command(&window).expect("a save");
    let writes: Vec<(&str, bool, bool)> =
        sent.writes.iter().map(|write| (write.key.as_str(), write.old.is_some(), write.new.is_some())).collect();
    assert_eq!(writes, [("api-token", false, true), ("username", true, false)]);
}

#[test]
fn undo_gives_a_key_the_value_of_the_read_and_discard_drops_each_change() {
    let mut window = opened();
    click(&mut window, "Remove the key username");
    click(&mut window, "Undo the change of username");
    assert!(!in_panel(&window, "removed") && window.query_by_label("Remove the key username").is_some());
    click(&mut window, "Remove the key password");
    typed(&mut window, "Name of the new key", "extra");
    click(&mut window, "Add the key");
    assert!(shows(&window, "Save 2 keys…"));
    click(&mut window, "Discard");
    assert!(!in_panel(&window, "extra") && !in_panel(&window, "removed"));
    assert!(window.get_by_label("Save").accesskit_node().is_disabled());
}

#[test]
fn each_state_of_a_save_shows_in_the_words_of_a_job() {
    let mut window = opened();
    let db = target("ams-prod", KeyKind::Secret, "db");
    let first = loaded(read(&db, "4711", VALUES));
    click(&mut window, "Remove the key username");
    save(&mut window, "1 key");
    let sent = save_command(&window).expect("a save");
    let error = |text: &str| CoreError::new(ErrorClass::Server, text);
    let states = [
        (
            WriteState::Failed(error("admission webhook denied the request")),
            "Save of 1 key: failed. The server refused it, and nothing changed: admission webhook denied the request",
        ),
        (WriteState::NotSent(error("ams-prod is not connected")), "Save of 1 key: not sent: ams-prod is not connected"),
        (
            WriteState::Unknown(error("no answer in 15s")),
            "Save of 1 key: result not known: no answer in 15s. The app does not send it again; the panel reads the Secret to show it.",
        ),
    ];
    for (state, text) in states {
        give(&mut window, "ams-prod", Some(saving(&first, &sent, state)));
        assert!(shows(&window, text), "{text}");
        assert!(!window.get_by_label("Save 1 key…").accesskit_node().is_disabled(), "the change stays for a new save");
    }
}

/// Research 08 section 8.3, step 6: after HTTP 409 the panel reads the object again. A key
/// that the server changed too is a conflict with both values, and the save waits for a
/// choice. The next save goes with the version of the new read.
#[test]
fn a_conflict_shows_the_value_of_the_server_and_the_save_waits_for_a_choice() {
    let mut window = opened();
    let db = target("ams-prod", KeyKind::Secret, "db");
    let first = loaded(read(&db, "4711", VALUES));
    click(&mut window, "Show the value of password");
    click(&mut window, "Show the value of username");
    typed(&mut window, "Value of password", "made-up-mine\n");
    typed(&mut window, "Value of username", "user-b\n");
    save(&mut window, "2 keys");
    let sent = save_command(&window).expect("a save");
    give(
        &mut window,
        "ams-prod",
        Some(saving(
            &first,
            &sent,
            WriteState::Conflict(CoreError::new(ErrorClass::Conflict, "the object has been modified")),
        )),
    );
    assert!(shows_part(&window, "Save of 2 keys: conflict. The Secret changed on the server after the read"));

    let server: Values<'_> = &[("password", Place::Data, b"made-up-server\n"), ("username", Place::Data, b"user-a\n")];
    let mut again = saving(&first, &sent, WriteState::Conflict(CoreError::new(ErrorClass::Conflict, "409")));
    again.object = Some(read(&db, "4713", server));
    give(&mut window, "ams-prod", Some(again.clone()));
    assert!(in_panel(&window, "CONFLICTS") && in_panel(&window, "Server:") && in_panel(&window, "Yours:"));
    assert!(window.query_by_label("Keep my value of username").is_none(), "the server did not change username");
    assert!(shows_part(&window, "made-up-server") && shows_part(&window, "made-up-mine"), "both values");
    assert!(shows(&window, "Choose a value for each conflict first."));
    assert!(window.get_by_label("Save 2 keys…").accesskit_node().is_disabled());

    click(&mut window, "Keep my value of password");
    assert!(!in_panel(&window, "CONFLICTS"));
    save(&mut window, "2 keys");
    let resent = save_command(&window).expect("a second save");
    assert_eq!(&*resent.version, "4713", "the version of the new read");
    assert_eq!(resent.writes.len(), 2);

    // The other choice: the value of the server, and the key has no change.
    let mut window = opened();
    click(&mut window, "Show the value of password");
    typed(&mut window, "Value of password", "made-up-mine\n");
    give(&mut window, "ams-prod", Some(again));
    click(&mut window, "Take the server value of password");
    assert!(!in_panel(&window, "CONFLICTS") && !in_panel(&window, "changed"));
    assert!(window.get_by_label("Value of password").value().as_deref() == Some("made-up-server\n"));
}
