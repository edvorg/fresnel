//! The dialog of a new Secret (plan section 12.2): its cluster and its namespace, a type
//! with the keys that it needs, the hints of the core, the create, its state and "Open it".

use std::sync::Arc;

use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::CoreError;
use fresnel_core::app::{Command, CreateJob};
use fresnel_core::keys::{KeyKind, NewSecret, Place, SecretType, Value, WriteState};
use fresnel_core::net::ErrorClass;

use super::scene::{read, secret, shows_part, target, with_secrets};
use super::{click, typed};
use crate::fake::{Window, open_page, shows, until, view, window};
use crate::kinds::in_panel;

/// The create that the views sent last.
fn created(window: &Window) -> Option<NewSecret> {
    window.state().sent.iter().rev().find_map(|command| match command {
        Command::CreateSecret(secret) => Some(secret.clone()),
        _ => None,
    })
}

fn give(window: &mut Window, secret: &NewSecret, state: WriteState) {
    window.state_mut().view.create = Some(Arc::new(CreateJob { number: 1, secret: secret.clone(), state }));
    window.run();
}

#[test]
fn the_dialog_creates_a_tls_secret_in_the_chosen_cluster_and_opens_it() {
    let secrets = [secret("db", "Opaque", &["password"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets), with_secrets("fra-stage", &secrets)]));
    open_page(&mut window, "Secrets");
    click(&mut window, "New Secret");
    assert!(window.get_by_label("Create the Secret").accesskit_node().is_disabled(), "no name yet");
    click(&mut window, "Cluster of the new Secret");
    click(&mut window, "Create in fra-stage");
    typed(&mut window, "Namespace of the new Secret", "shop");
    typed(&mut window, "Name of the new Secret", "api");
    click(&mut window, "Type of the new Secret");
    click(&mut window, "Type kubernetes.io/tls");
    let keys = |window: &Window| [window.get_by_label("Key 1").value(), window.get_by_label("Key 2").value()];
    assert_eq!(keys(&window), [Some("tls.crt".to_owned()), Some("tls.key".to_owned())], "the keys of the type");
    assert!(shows(&window, "the type kubernetes.io/tls needs a value of `tls.crt`"));

    typed(&mut window, "Value 1", "not a certificate");
    assert!(shows(&window, "`tls.crt`: the value has no PEM certificate"), "a hint of the core");
    typed(&mut window, "Value 2", "made-up-key");
    click(&mut window, "Create the Secret");
    let sent = created(&window).expect("a create");
    assert_eq!((sent.cluster.as_str(), sent.namespace.as_str(), sent.name.as_str()), ("fra-stage", "shop", "api"));
    assert_eq!(sent.secret_type, SecretType::Tls);
    assert!(sent.entries.get("tls.key") == Some(&Value::new(b"made-up-key".to_vec())), "the value as bytes");

    give(&mut window, &sent, WriteState::Pending);
    assert!(shows(&window, "Create: pending…"));
    assert!(window.get_by_label("Create the Secret").accesskit_node().is_disabled(), "one create at a time");
    let api = target("fra-stage", KeyKind::Secret, "api");
    give(&mut window, &sent, WriteState::Done(read(&api, "1", &[("tls.key", Place::Data, b"made-up-key")])));
    assert!(shows(&window, "Create: done. fra-stage has the Secret now."));

    // The watch of the Secrets of fra-stage has the new Secret, and "Open it" shows it.
    let mut new = secret("api", "kubernetes.io/tls", &["tls.crt", "tls.key"]);
    Arc::make_mut(&mut new).metadata.namespace = Some("shop".to_owned());
    window.state_mut().view.clusters[1] = with_secrets("fra-stage", &[new]);
    click(&mut window, "Open it");
    until(&mut window, "the panel of the new Secret", |window| in_panel(window, "api") && in_panel(window, "DATA"));
    assert!(window.query_by_label("Create the Secret").is_none(), "the dialog closed");
}

#[test]
fn a_key_opens_the_dialog_from_each_page_and_a_close_drops_its_values() {
    let mut window = window(view(vec![with_secrets("ams-prod", &[])]));
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    window.run();
    assert!(shows(&window, "New Secret") && shows(&window, "Create the Secret"), "the dialog over the Pods");
    typed(&mut window, "Name of the new Secret", "api");
    typed(&mut window, "Value 1", "made-up");
    window.key_press(Key::Escape);
    window.run();
    assert!(window.query_by_label("Create the Secret").is_none(), "Escape closes the dialog");
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    window.run();
    assert!(window.get_by_label("Name of the new Secret").value().unwrap_or_default().is_empty());
    assert!(window.get_by_label("Value 1").value().unwrap_or_default().is_empty(), "no value stays");
    assert!(window.get_by_label("Namespace of the new Secret").value().as_deref() == Some("default"));
}

#[test]
fn a_refused_create_shows_the_reason_and_keeps_the_form_for_a_new_try() {
    let mut window = window(view(vec![with_secrets("ams-prod", &[])]));
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    window.run();
    typed(&mut window, "Name of the new Secret", "api");
    typed(&mut window, "Key 1", "token");
    typed(&mut window, "Value 1", "made-up");
    click(&mut window, "Create the Secret");
    let sent = created(&window).expect("a create");
    assert_eq!((sent.cluster.as_str(), sent.namespace.as_str()), ("ams-prod", "default"), "the defaults");
    let refused = CoreError::new(ErrorClass::Conflict, "secrets \"api\" already exists");
    give(&mut window, &sent, WriteState::Failed(refused));
    assert!(shows_part(&window, "Create: failed: secrets \"api\" already exists"));
    assert!(!window.get_by_label("Create the Secret").accesskit_node().is_disabled(), "a new try");
    assert!(window.get_by_label("Value 1").value().as_deref() == Some("made-up"), "the form stays");
}
