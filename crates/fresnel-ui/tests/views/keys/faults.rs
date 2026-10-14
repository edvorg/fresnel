//! The key view and the New Secret dialog when the object, the core or the user moves at the
//! worst moment, and with bad data from the cluster. Each value is made up.

use std::sync::Arc;

use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::CoreError;
use fresnel_core::app::{Command, CreateJob};
use fresnel_core::keys::{KeyKind, KeyObject, Place, SecretType, WriteState};
use fresnel_core::net::ErrorClass;

use super::scene::{Values, give, loaded, open, read, saving, secret, target, with_secrets};
use super::{click, save_command, typed};
use crate::fake::{Window, shows, view, window};
use crate::kinds::in_panel;

const VALUES: Values<'static> =
    &[("password", Place::Data, b"made-up-password\n"), ("username", Place::Data, b"user-a\n")];

/// The panel of the Secret `db` of ams-prod at version 4711, with a change of `password`.
fn changed() -> Window {
    let secrets = [secret("db", "Opaque", &["password", "username"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    open(&mut window, "Secrets", "db");
    let db = target("ams-prod", KeyKind::Secret, "db");
    give(&mut window, "ams-prod", Some(loaded(read(&db, "4711", VALUES))));
    click(&mut window, "Show the value of password");
    typed(&mut window, "Value of password", "made-up-password-2\n");
    window
}

/// Each command of this type that the views sent.
fn count(window: &Window, of_type: fn(&Command) -> bool) -> usize {
    window.state().sent.iter().filter(|command| of_type(command)).count()
}

/// Plan section 11.2: a deleted object has its actions off. The Secret leaves its store
/// while the dialog of a save is open, and the save of the dialog does not go.
#[test]
fn a_secret_that_leaves_its_store_while_the_save_dialog_is_open_gets_no_save() {
    let mut window = changed();
    click(&mut window, "Save 1 key…");
    let keys = window.state().view.clusters[0].data.as_ref().and_then(|data| data.keys.clone());
    window.state_mut().view.clusters[0] = with_secrets("ams-prod", &[]);
    give(&mut window, "ams-prod", keys.map(|keys| (*keys).clone()));
    let reason = "You cannot change this Secret. The Secret is not in its store now: its actions are off.";
    assert!(shows(&window, reason));
    assert!(window.get_by_label("Save 1 key").accesskit_node().is_disabled(), "the save of the dialog is off");
    window.get_by_label("Save 1 key").click();
    window.run();
    assert_eq!(count(&window, |command| matches!(command, Command::SaveKeys(_))), 0, "no save went");

    click(&mut window, "Cancel");
    assert!(in_panel(&window, "This Secret is deleted. The panel shows its last state."));
    for label in ["Save 1 key…", "Name of the new key", "Remove the key username"] {
        assert!(window.get_by_label(label).accesskit_node().is_disabled(), "{label}");
    }
}

/// A key of the shell while the dialog of a save is open: the quick search would open over
/// the dialog, and Back would close the panel below it.
#[test]
fn the_keys_of_the_shell_wait_while_the_save_dialog_is_open() {
    let mut window = changed();
    click(&mut window, "Save 1 key…");
    window.key_press_modifiers(Modifiers::COMMAND, Key::K);
    window.run();
    assert!(window.query_by_label("Search text").is_none(), "no search over the dialog");
    assert!(shows(&window, "Save 1 key"), "the dialog stays");
    click(&mut window, "Cancel");
    window.key_press_modifiers(Modifiers::COMMAND, Key::K);
    window.run();
    assert!(window.query_by_label("Search text").is_some(), "the key acts again after the dialog");
}

/// Fast input during a save: an edit then would meet the answer of the save as a change of
/// the server, and the own save of the user would come back as a conflict. The keys wait
/// for the end of the save, and the panel marks the change until the watch brings it.
#[test]
fn the_keys_wait_for_a_save_and_its_answer_is_no_conflict() {
    let mut window = changed();
    let db = target("ams-prod", KeyKind::Secret, "db");
    let first = loaded(read(&db, "4711", VALUES));
    click(&mut window, "Save 1 key…");
    click(&mut window, "Save 1 key");
    let sent = save_command(&window).expect("a save");
    give(&mut window, "ams-prod", Some(saving(&first, &sent, WriteState::Pending)));
    for label in ["Name of the new key", "Remove the key username", "Undo the change of password", "Discard"] {
        assert!(window.get_by_label(label).accesskit_node().is_disabled(), "{label} waits for the save");
    }
    typed(&mut window, "Value of password", "made-up-password-3\n");

    let after: Values<'_> =
        &[("password", Place::Data, b"made-up-password-2\n"), ("username", Place::Data, b"user-a\n")];
    let answer = read(&db, "4712", after);
    let mut done = saving(&first, &sent, WriteState::Done(Arc::clone(&answer)));
    done.object = Some(answer);
    give(&mut window, "ams-prod", Some(done.clone()));
    assert!(!in_panel(&window, "CONFLICTS") && !in_panel(&window, "changed"), "the answer is the new read");
    assert!(!window.get_by_label("Name of the new key").accesskit_node().is_disabled(), "the keys after the save");
    let pending = "Change pending: the server stored a change, and the watch did not bring it yet.";
    assert!(in_panel(&window, pending), "the store has version 100 of the Secret");

    let mut stored = secret("db", "Opaque", &["password", "username"]);
    Arc::make_mut(&mut stored).metadata.resource_version = Some("4712".to_owned());
    window.state_mut().view.clusters[0] = with_secrets("ams-prod", &[stored]);
    give(&mut window, "ams-prod", Some(done));
    assert!(!in_panel(&window, pending), "the watch brought the version of the save");
}

/// Fast input: a second click on Create before the core shows the job of the first one.
/// It would go again and come back as "already exists" for the Secret of the first one.
/// The view still shows an earlier create of the same values, which is not this one.
#[test]
fn a_second_click_on_create_before_the_core_has_the_job_sends_nothing() {
    let mut window = window(view(vec![with_secrets("ams-prod", &[])]));
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    window.run();
    typed(&mut window, "Name of the new Secret", "api");
    typed(&mut window, "Key 1", "token");
    typed(&mut window, "Value 1", "made-up");
    click(&mut window, "Create the Secret");
    let sent = window.state().sent.iter().rev().find_map(|command| match command {
        Command::CreateSecret(secret) => Some(secret.clone()),
        _ => None,
    });
    let refused = WriteState::Failed(CoreError::new(ErrorClass::Server, "the server is shutting down"));
    let job = CreateJob { number: 1, secret: sent.expect("a create"), state: refused };
    window.state_mut().view.create = Some(Arc::new(job));
    window.run();
    assert!(shows(&window, "Create: failed: the server is shutting down"));

    let creates = |window: &Window| count(window, |command| matches!(command, Command::CreateSecret(_)));
    click(&mut window, "Create the Secret");
    click(&mut window, "Create the Secret");
    assert_eq!(creates(&window), 2, "the try after the refusal, and no third one");
    assert!(shows(&window, "Create: pending…"), "pending from the click, not the earlier refusal");
    assert!(window.get_by_label("Create the Secret").accesskit_node().is_disabled());
}

/// Bad input: two rows with the name of one key. The Secret would keep one of the values
/// with no word of the server, so the create waits for a change of a name.
#[test]
fn two_keys_of_one_name_stop_the_create() {
    let mut window = window(view(vec![with_secrets("ams-prod", &[])]));
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    window.run();
    typed(&mut window, "Name of the new Secret", "api");
    typed(&mut window, "Key 1", "token");
    click(&mut window, "Add key");
    typed(&mut window, "Key 2", "token");
    assert!(shows(&window, "`token` is the name of two keys"));
    assert!(window.get_by_label("Create the Secret").accesskit_node().is_disabled());
    typed(&mut window, "Key 2", "token-2");
    assert!(!window.get_by_label("Create the Secret").accesskit_node().is_disabled());
}

/// Bad data from the cluster: an image pull Secret of 150 registries. Each registry is a
/// layout in each frame, so the panel shows the first 100 and the count of the others.
#[test]
fn a_pull_secret_of_many_registries_shows_the_first_ones_and_their_count() {
    let secrets = [secret("registry-key", "kubernetes.io/dockerconfigjson", &[".dockerconfigjson"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    open(&mut window, "Secrets", "registry-key");
    let auths: Vec<String> =
        (0..150).map(|number| format!(r#""r{number:03}.example":{{"username":"robot"}}"#)).collect();
    let json = format!(r#"{{"auths":{{{}}}}}"#, auths.join(","));
    let pull = target("ams-prod", KeyKind::Secret, "registry-key");
    let object = read(&pull, "1", &[(".dockerconfigjson", Place::Data, json.as_bytes())]);
    let object = Arc::new(KeyObject { secret_type: Some(SecretType::DockerConfigJson), ..KeyObject::clone(&object) });
    give(&mut window, "ams-prod", Some(loaded(object)));
    assert!(in_panel(&window, "r000.example") && in_panel(&window, "r099.example"));
    assert!(!shows(&window, "r100.example") && in_panel(&window, "and 50 more"));
}
