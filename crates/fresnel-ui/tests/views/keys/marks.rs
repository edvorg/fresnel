//! The marks of the key view: an immutable object, a user who cannot change the object, a
//! binary value, the certificates of a TLS Secret, the registries of an image pull Secret,
//! the pods that use a Secret, and the cluster of each request (weakness 9 of plan section 6).

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{AccessView, CheckState, Command};
use fresnel_core::keys::{KeyKind, KeyObject, Place, SecretType};
use fresnel_core::kinds::{Action, Kind};
use fresnel_core::ops::access::{Decision, needs};
use rcgen::{CertificateParams, DnType, KeyPair, date_time_ymd};

use super::scene::{
    NAMESPACE, config_map, give, loaded, open, read, secret, shows_part, target, with_config_maps, with_secrets,
};
use super::{click, save_command, typed};
use crate::data::pods;
use crate::fake::{shows, view, window};
use crate::kinds::in_panel;

#[test]
fn an_immutable_secret_has_its_mark_and_no_change_starts() {
    let secrets = [secret("db", "Opaque", &["password"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    open(&mut window, "Secrets", "db");
    let db = target("ams-prod", KeyKind::Secret, "db");
    let object = read(&db, "1", &[("password", Place::Data, b"made-up\n")]);
    let immutable = Arc::new(KeyObject { immutable: true, ..KeyObject::clone(&object) });
    give(&mut window, "ams-prod", Some(loaded(immutable)));
    assert!(in_panel(&window, "type Opaque · 1 key · immutable"));
    assert!(shows_part(&window, "This Secret is immutable: the server refuses each change of a value."));
    let off = ["Remove the key password", "Name of the new key", "Save"];
    for label in off {
        assert!(window.get_by_label(label).accesskit_node().is_disabled(), "{label}");
    }
    click(&mut window, "Show the value of password");
    let editor = window.get_by_label("Value of password");
    assert!(editor.value().as_deref() == Some("made-up\n"), "the value shows, read-only");
}

/// Plan section 10: an action that the rules of the user forbid is off, with the reason.
#[test]
fn a_user_who_cannot_patch_the_secret_sees_why_and_cannot_change_a_value() {
    let secrets = [secret("db", "Opaque", &["password"])];
    let mut row = with_secrets("ams-prod", &secrets);
    let key = target("ams-prod", KeyKind::Secret, "db").key;
    let mut access = AccessView::default();
    for request in needs(Action::EditKeys, Kind::Secret, &key) {
        let decision = match request.verb {
            "patch" => Decision::Denied("no rule allows patch secrets in scene".to_owned()),
            _ => Decision::Allowed,
        };
        access.checks.insert(request, CheckState::Answered(decision));
    }
    if let Some(data) = row.data.as_mut() {
        data.access = Arc::new(access);
    }
    let mut window = window(view(vec![row]));
    open(&mut window, "Secrets", "db");
    let db = target("ams-prod", KeyKind::Secret, "db");
    give(&mut window, "ams-prod", Some(loaded(read(&db, "1", &[("password", Place::Data, b"made-up")]))));
    assert!(shows(&window, "You cannot change this Secret: no rule allows patch secrets in scene"));
    assert!(window.get_by_label("Remove the key password").accesskit_node().is_disabled());
    assert!(!window.get_by_label("Copy the value of password").accesskit_node().is_disabled(), "a copy reads only");
}

#[test]
fn a_binary_value_shows_its_map_type_size_and_first_bytes_and_no_editor() {
    let maps = [config_map("assets", &["color"], &["logo"])];
    let mut window = window(view(vec![with_config_maps("ams-prod", &maps)]));
    open(&mut window, "ConfigMaps", "assets");
    let assets = target("ams-prod", KeyKind::ConfigMap, "assets");
    let png = b"\x89PNG\r\n\x1a\n\x00\x01".as_slice();
    let values = [("color", Place::Data, b"blue\n".as_slice()), ("logo", Place::BinaryData, png)];
    give(&mut window, "ams-prod", Some(loaded(read(&assets, "1", &values))));
    let texts = ["binaryData", "binary · PNG image · 10 bytes", "89 50 4e 47 0d 0a 1a 0a 00 01"];
    assert!(texts.iter().all(|text| in_panel(&window, text)), "the marks of the binary value");
    assert!(window.query_by_label("Value of logo").is_none() && window.query_by_label("Value of color").is_some());
    assert!(window.query_by_label("Show the value of color").is_none(), "a ConfigMap hides no value");
}

/// A made-up chain of one certificate, valid from 2026-10-01 to 2027-02-01: 16 whole days
/// after the time of the view tests.
pub(super) fn certificate() -> String {
    let mut params = CertificateParams::new(vec!["web.shop.example".to_owned()]).expect("parameters");
    params.distinguished_name.push(DnType::CommonName, "web");
    params.distinguished_name.push(DnType::OrganizationName, "shop");
    params.not_before = date_time_ymd(2026, 10, 1);
    params.not_after = date_time_ymd(2027, 2, 1);
    let key = KeyPair::generate().expect("a key");
    let der = params.self_signed(&key).expect("a certificate");
    format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", STANDARD.encode(der.der()))
}

#[test]
fn a_tls_secret_shows_its_certificates_and_a_pull_secret_its_registries_with_no_password() {
    let secrets = [
        secret("web-tls", "kubernetes.io/tls", &["tls.crt", "tls.key"]),
        secret("registry-key", "kubernetes.io/dockerconfigjson", &[".dockerconfigjson"]),
    ];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    open(&mut window, "Secrets", "web-tls");
    let tls = target("ams-prod", KeyKind::Secret, "web-tls");
    let chain = certificate();
    let values = [("tls.crt", Place::Data, chain.as_bytes()), ("tls.key", Place::Data, b"made-up-key".as_slice())];
    let object =
        Arc::new(KeyObject { secret_type: Some(SecretType::Tls), ..KeyObject::clone(&read(&tls, "1", &values)) });
    give(&mut window, "ams-prod", Some(loaded(object)));
    let texts =
        ["CERTIFICATES", "Certificate 1", "CN=web, O=shop", "in 16 days", "DNS:web.shop.example", "EC 256 bits"];
    assert!(texts.iter().all(|text| in_panel(&window, text)), "the facts of the certificate");
    assert!(!shows_part(&window, "made-up-key"), "the key stays hidden");

    open(&mut window, "Secrets", "registry-key");
    let pull = target("ams-prod", KeyKind::Secret, "registry-key");
    let json = br#"{"auths":{"registry.example.com":{"username":"robot","password":"made-up-pass"}}}"#;
    let object = read(&pull, "1", &[(".dockerconfigjson", Place::Data, json.as_slice())]);
    let object = Arc::new(KeyObject { secret_type: Some(SecretType::DockerConfigJson), ..KeyObject::clone(&object) });
    give(&mut window, "ams-prod", Some(loaded(object)));
    let texts = ["REGISTRIES", "registry.example.com", "robot", "set"];
    assert!(texts.iter().all(|text| in_panel(&window, text)), "the registry and its user");
    assert!(!shows_part(&window, "made-up-pass"), "no password");
}

/// Plan section 9.9: the reference index gives the pods that use a Secret.
#[test]
fn the_panel_of_a_secret_shows_the_pods_that_use_it() {
    let secrets = [secret("web-secret", "Opaque", &["token"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    open(&mut window, "Secrets", "web-secret");
    assert!(in_panel(&window, "USED BY"));
    let names = pods().into_iter().filter_map(|pod| pod.metadata.name.clone());
    for pod in names.filter(|name| name.starts_with("web-6888b778b6-")) {
        assert!(in_panel(&window, &pod), "the pod {pod}");
    }
    assert_eq!(NAMESPACE, "scene");
}

/// Weakness 9 of plan section 6: a Secret of the same name in two clusters. The panel of
/// the second one reads and saves the Secret of the second cluster.
#[test]
fn the_secret_of_the_second_cluster_reads_and_saves_in_that_cluster_only() {
    let secrets = [secret("db", "Opaque", &["password"])];
    let clusters = vec![with_secrets("ams-prod", &secrets), with_secrets("fra-stage", &secrets)];
    let mut window = window(view(clusters));
    crate::fake::open_page(&mut window, "Secrets");
    crate::fake::until(&mut window, "two rows", |window| window.query_all_by_label("db").count() == 2);
    window.query_all_by_label("db").nth(1).expect("the row of fra-stage").click();
    window.run();
    let fra = target("fra-stage", KeyKind::Secret, "db");
    assert!(window.state().sent.contains(&Command::ShowKeys(Some(fra.clone()))));
    give(&mut window, "fra-stage", Some(loaded(read(&fra, "9", &[("password", Place::Data, b"made-up")]))));
    click(&mut window, "Show the value of password");
    typed(&mut window, "Value of password", "made-up-2");
    click(&mut window, "Save 1 key…");
    click(&mut window, "Save 1 key");
    let save = save_command(&window).expect("a save");
    assert_eq!(save.target.cluster.as_str(), "fra-stage");
}
