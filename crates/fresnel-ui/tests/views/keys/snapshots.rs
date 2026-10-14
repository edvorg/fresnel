//! PNG files of the key view in the dark theme, for a look by a person. Each value is made up.

use std::sync::Arc;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::CoreError;
use fresnel_core::keys::{KeyKind, KeyObject, Place, SecretType, WriteState};
use fresnel_core::net::ErrorClass;

use super::scene::{
    Values, config_map, give, loaded, open, read, saving, secret, target, with_config_maps, with_secrets,
};
use super::{click, save_command, typed};
use crate::fake::{Window, view, window_with};
use crate::snapshots::{SCALE, save};

/// Saves the frame after the panel scrolled to the widget with the name `at`.
fn picture(window: &mut Window, at: &str, name: &str) {
    window.get_by_label(at).scroll_to_me();
    window.run();
    window.remove_cursor();
    window.run();
    save(window, name);
}

const DB: Values<'static> = &[
    ("password", Place::Data, b"made-up-password\n"),
    ("url", Place::Data, b"postgres://db.scene.svc:5432/shop?sslmode=require"),
    ("username", Place::Data, b"shop-app\n"),
];

/// The panel of the Secret `db` with the values of `DB` at version 4711.
fn db_window() -> Window {
    let secrets =
        [secret("db", "Opaque", &["password", "url", "username"]), secret("web-secret", "Opaque", &["token"])];
    let mut window = window_with(view(vec![with_secrets("ams-prod", &secrets)]), egui::Theme::Dark, SCALE);
    open(&mut window, "Secrets", "db");
    give(&mut window, "ams-prod", Some(loaded(read(&target("ams-prod", KeyKind::Secret, "db"), "4711", DB))));
    window
}

#[test]
fn a_secret_with_its_values_hidden_and_then_an_edit_with_its_dialog() {
    let mut window = db_window();
    picture(&mut window, "Copy the value of username", "keys-secret");

    click(&mut window, "Show the value of password");
    typed(&mut window, "Value of password", "made-up-password-2");
    click(&mut window, "Remove the key username");
    typed(&mut window, "Name of the new key", "api-token");
    typed(&mut window, "Value of the new key", "made-up-token\n");
    click(&mut window, "Add the key");
    picture(&mut window, "Add the key", "keys-secret-edit");

    click(&mut window, "Save 3 keys…");
    click(&mut window, "Show the values");
    picture(&mut window, "Save 3 keys", "keys-save-dialog");
}

#[test]
fn a_conflict_after_a_save() {
    let mut window = db_window();
    let db = target("ams-prod", KeyKind::Secret, "db");
    let first = loaded(read(&db, "4711", DB));
    click(&mut window, "Show the value of password");
    typed(&mut window, "Value of password", "made-up-mine\n");
    click(&mut window, "Save 1 key…");
    click(&mut window, "Save 1 key");
    let sent = save_command(&window).expect("a save");
    let server: Values<'_> =
        &[("password", Place::Data, b"made-up-server\n"), ("username", Place::Data, b"shop-app\n")];
    let mut conflict = saving(&first, &sent, WriteState::Conflict(CoreError::new(ErrorClass::Conflict, "409")));
    conflict.object = Some(read(&db, "4713", server));
    give(&mut window, "ams-prod", Some(conflict));
    picture(&mut window, "Take the server value of password", "keys-conflict");
}

#[test]
fn a_config_map_with_a_binary_value() {
    let maps = [config_map("assets", &["nginx.conf", "mode"], &["logo"])];
    let mut window = window_with(view(vec![with_config_maps("ams-prod", &maps)]), egui::Theme::Dark, SCALE);
    open(&mut window, "ConfigMaps", "assets");
    let conf = b"server {\n  listen 8080;\n  root /srv/www;\n}\n";
    let values: Values<'_> = &[
        ("logo", Place::BinaryData, b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x10"),
        ("mode", Place::Data, b"production"),
        ("nginx.conf", Place::Data, conf),
    ];
    give(&mut window, "ams-prod", Some(loaded(read(&target("ams-prod", KeyKind::ConfigMap, "assets"), "88", values))));
    picture(&mut window, "Copy the value of nginx.conf", "keys-config-map");
}

#[test]
fn the_registries_of_an_image_pull_secret_and_an_immutable_secret() {
    let secrets = [secret("registry-key", "kubernetes.io/dockerconfigjson", &[".dockerconfigjson"])];
    let mut window = window_with(view(vec![with_secrets("ams-prod", &secrets)]), egui::Theme::Dark, SCALE);
    open(&mut window, "Secrets", "registry-key");
    let json = br#"{"auths":{"ghcr.io":{"username":"octo","password":"made-up"},"registry.example.com":{"auth":"cm9ib3Q6bWFkZS11cA=="}}}"#;
    let object = read(
        &target("ams-prod", KeyKind::Secret, "registry-key"),
        "5",
        &[(".dockerconfigjson", Place::Data, json.as_slice())],
    );
    let object =
        KeyObject { secret_type: Some(SecretType::DockerConfigJson), immutable: true, ..KeyObject::clone(&object) };
    give(&mut window, "ams-prod", Some(loaded(Arc::new(object))));
    picture(&mut window, "registry.example.com", "keys-pull-secret");
}

#[test]
fn the_certificates_of_a_tls_secret() {
    let secrets = [secret("web-tls", "kubernetes.io/tls", &["tls.crt", "tls.key"])];
    let mut window = window_with(view(vec![with_secrets("ams-prod", &secrets)]), egui::Theme::Dark, SCALE);
    open(&mut window, "Secrets", "web-tls");
    let chain = super::marks::certificate();
    let values: Values<'_> = &[("tls.crt", Place::Data, chain.as_bytes()), ("tls.key", Place::Data, b"made-up-key")];
    let object = read(&target("ams-prod", KeyKind::Secret, "web-tls"), "12", values);
    let object = KeyObject { secret_type: Some(SecretType::Tls), ..KeyObject::clone(&object) };
    give(&mut window, "ams-prod", Some(loaded(Arc::new(object))));
    picture(&mut window, "EC 256 bits", "keys-tls-secret");
}

#[test]
fn the_dialog_of_a_new_secret() {
    let mut window = window_with(view(vec![with_secrets("ams-prod", &[])]), egui::Theme::Dark, SCALE);
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    window.run();
    typed(&mut window, "Name of the new Secret", "web-tls");
    click(&mut window, "Type of the new Secret");
    click(&mut window, "Type kubernetes.io/tls");
    typed(&mut window, "Value 2", "made-up-key");
    picture(&mut window, "Create the Secret", "keys-new-secret");
}
