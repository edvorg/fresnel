//! The keys of Secrets and ConfigMaps on a real kube-apiserver (plan section 15, M4): an
//! edit of one key keeps each other key byte for byte, a binary value does not change, a
//! conflict shows as a conflict, and add key and remove key work. kubectl in the container
//! reads the state of the cluster. The text of the editor of a Secret has its values decoded
//! (M11).
//!
//! An assertion names keys only: a value never goes to the output of a test.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_core::cluster::{ClusterId, Gateway};
use fresnel_core::keys::{
    self, Draft, Form, KeyKind, KeyObject, NewSecret, Refusal, SecretType, Special, Target, Value, WriteState,
};
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::check::{self, Identity};
use fresnel_core::manifest::{self, Decoded};
use fresnel_core::net::NetSettings;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::K3s;
use kube::core::Request;
use kube::core::params::PostParams;
use serde_json::{Value as Json, json};
use tokio::runtime::Handle;

use common::{TestTasks, connect, in_time, registry, until_live};

const NAMESPACE: &str = "default";
/// Bytes that are not UTF-8: a zero, 0xff, and each other byte value.
const BLOB: [u8; 6] = [0x00, 0xff, 0x10, 0x80, 0xfe, 0x7f];

/// One map of an object as the server stores it, as kubectl reads it: base64 for bytes.
type Stored = BTreeMap<String, String>;

/// The maps `data` and `binaryData` of an object, as kubectl reads them.
async fn stored(k3s: &K3s, kind: &str, name: &str) -> (Stored, Stored) {
    let text = k3s.kubectl(&["get", kind, name, "--namespace", NAMESPACE, "--output", "json"]).await;
    let object: Json = serde_json::from_str(&text.expect("kubectl works")).expect("JSON");
    let map = |field: &str| -> Stored {
        let entries = object.get(field).and_then(Json::as_object).into_iter().flatten();
        entries.map(|(key, value)| (key.clone(), value.as_str().unwrap_or_default().to_owned())).collect()
    };
    (map("data"), map("binaryData"))
}

/// Asserts that each key of `before` but `changed` has the same text after, and that no
/// other key came. The message names the key, never its value.
fn same_except(before: &Stored, after: &Stored, changed: &[&str]) {
    for (key, value) in before.iter().filter(|(key, _)| !changed.contains(&key.as_str())) {
        assert!(after.get(key) == Some(value), "the key {key} changed");
    }
    let came = after.keys().find(|key| !before.contains_key(*key) && !changed.contains(&key.as_str()));
    assert!(came.is_none(), "the key {came:?} came");
}

/// Asserts that the server stores these bytes for a key of a map of base64 texts.
fn holds(map: &Stored, key: &str, bytes: &[u8]) {
    assert!(map.get(key) == Some(&STANDARD.encode(bytes)), "the key {key} does not have its bytes");
}

fn target(kind: KeyKind, name: &str) -> Target {
    Target { cluster: ClusterId::new("default"), kind, key: ObjectKey::new(NAMESPACE, name), uid: "".into() }
}

async fn read(gateway: &Gateway, kind: KeyKind, name: &str) -> Arc<KeyObject> {
    Arc::new(keys::read(gateway, &target(kind, name)).await.expect("the read"))
}

/// Saves a draft and gives the state of the save.
async fn save(gateway: &Gateway, draft: &Draft) -> WriteState {
    WriteState::of_save(keys::save(gateway, &draft.save().expect("a save")).await)
}

/// The answer of a write that the server stored. Each other state ends the test.
fn done(state: &WriteState) -> Arc<KeyObject> {
    match state {
        WriteState::Done(object) => Arc::clone(object),
        other => panic!("the write did not end well: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn an_edit_keeps_each_other_key_byte_for_byte_and_a_conflict_shows_as_a_conflict() {
    let k3s = K3s::start("m4-keys").await.expect("k3s starts");
    let tasks = TestTasks::new(&Handle::current());
    let registry = registry(&tasks, &Handle::current(), &k3s.kubeconfig(), NetSettings::default());
    let link = connect(&registry, "default");
    until_live(&link).await;
    let gateway = &link.gateway;

    // A create with values in `data`, as text, as bytes, with a carriage return, and with no line feed.
    let mut new = NewSecret::new(ClusterId::new("default"), NAMESPACE, "db", SecretType::Opaque);
    for (key, bytes) in [("user", &b"admin\n"[..]), ("blob", &BLOB), ("crlf", b"a\r\nb\r\n"), ("nolf", b"x")] {
        new.entries.insert(key.to_owned(), Value::new(bytes.to_vec()));
    }
    let created = WriteState::of_create(in_time("the create", keys::create(gateway, &new)).await);
    assert!(matches!(created, WriteState::Done(_)), "the create: {created:?}");
    let (first, _) = stored(&k3s, "secret", "db").await;
    for (key, bytes) in [("user", &b"admin\n"[..]), ("blob", &BLOB), ("crlf", b"a\r\nb\r\n"), ("nolf", b"x")] {
        holds(&first, key, bytes);
    }

    // The read decodes each text and keeps the bytes of the binary value.
    let object = read(gateway, KeyKind::Secret, "db").await;
    let form = |key: &str| object.value(key).map(Value::form);
    assert_eq!((form("user"), form("crlf"), form("nolf")), (Some(Form::Text), Some(Form::Text), Some(Form::Text)));
    assert!(matches!(form("blob"), Some(Form::Binary(_))), "a value that is not UTF-8 is binary");
    assert!(object.value("blob").is_some_and(|value| value.bytes() == BLOB));
    let marks = object.value("crlf").map(|value| (value.has_carriage_return(), value.ends_without_line_feed()));
    assert_eq!(marks, Some((true, false)));
    assert!(object.value("nolf").is_some_and(Value::ends_without_line_feed));

    // An edit of one key: each other key, the binary one too, stays byte for byte.
    let mut draft = Draft::new(Arc::clone(&object));
    draft.set("user", b"root\n".to_vec()).expect("a change");
    let after_edit = done(&save(gateway, &draft).await);
    let (second, _) = stored(&k3s, "secret", "db").await;
    same_except(&first, &second, &["user"]);
    holds(&second, "user", b"root\n");
    assert!(after_edit.value("user").is_some_and(|value| value.bytes() == b"root\n"), "the answer is the new object");
    let managers = k3s.kubectl(&[
        "get",
        "secret",
        "db",
        "--show-managed-fields",
        "--output",
        "jsonpath={.metadata.managedFields[*].manager}",
    ]);
    assert!(managers.await.expect("kubectl works").split(' ').any(|manager| manager == "fresnel"), "the field manager");

    // Add key and remove key, in one save.
    let mut draft = Draft::new(after_edit);
    draft.add("api-key", b"k-1".to_vec()).expect("a new key");
    draft.remove("nolf").expect("a removal");
    let after_keys = done(&save(gateway, &draft).await);
    let (third, _) = stored(&k3s, "secret", "db").await;
    same_except(&second, &third, &["api-key", "nolf"]);
    holds(&third, "api-key", b"k-1");
    assert!(!third.contains_key("nolf"), "the removed key is gone");

    // A conflict: kubectl changes the key of the edit after the read. The server refuses the
    // save, changes nothing, and the new read shows the value of each side.
    let user_of_kubectl = r#"{"data":{"user":"b3RoZXIK"}}"#; // "other\n"
    k3s.kubectl(&["patch", "secret", "db", "--namespace", NAMESPACE, "--patch", user_of_kubectl])
        .await
        .expect("kubectl");
    let mut draft = Draft::new(Arc::clone(&after_keys));
    draft.set("user", b"mine\n".to_vec()).expect("a change");
    let refused = save(gateway, &draft).await;
    let WriteState::Conflict(error) = &refused else { panic!("a conflict: {refused:?}") };
    assert_eq!(error.status(), Some(409));
    let (fourth, _) = stored(&k3s, "secret", "db").await;
    holds(&fourth, "user", b"other\n");
    same_except(&third, &fourth, &["user"]);
    let conflicts = draft.rebase(read(gateway, KeyKind::Secret, "db").await).expect("the same object");
    let sides: Vec<_> = conflicts
        .iter()
        .map(|conflict| {
            let bytes = |value: &Option<Value>| value.as_ref().map(|value| value.bytes().to_vec());
            (conflict.key.as_str(), bytes(&conflict.base), bytes(&conflict.mine), bytes(&conflict.server))
        })
        .collect();
    let expected = [("user", Some(b"root\n".to_vec()), Some(b"mine\n".to_vec()), Some(b"other\n".to_vec()))];
    assert!(sides == expected, "the conflict of `user` with the value of each side");
    let resolved = done(&save(gateway, &draft).await);
    holds(&stored(&k3s, "secret", "db").await.0, "user", b"mine\n");

    // A change of kubectl to an other key is no overlap: the save after the new read keeps it.
    k3s.kubectl(&["patch", "secret", "db", "--namespace", NAMESPACE, "--patch", r#"{"data":{"api-key":"ay0y"}}"#])
        .await
        .expect("kubectl");
    let mut draft = Draft::new(resolved);
    draft.set("crlf", b"c\r\n".to_vec()).expect("a change");
    assert!(matches!(save(gateway, &draft).await, WriteState::Conflict(_)), "the version is old");
    let fresh = read(gateway, KeyKind::Secret, "db").await;
    assert!(draft.rebase(fresh).expect("the same object").is_empty(), "no key of both sides");
    done(&save(gateway, &draft).await);
    let (sixth, _) = stored(&k3s, "secret", "db").await;
    holds(&sixth, "api-key", b"k-2");
    holds(&sixth, "crlf", b"c\r\n");
    holds(&sixth, "blob", &BLOB);

    the_editor_shows_each_text_value_decoded_and_a_binary_value_as_base64(&k3s, gateway).await;
    immutable_objects_refuse_an_edit(&k3s, gateway).await;
    config_maps_keep_text_and_bytes_in_their_maps(&k3s, gateway).await;
    special_types_give_their_views(gateway).await;
}

/// The text of the editor of the Secret `db` (plan section 12.9): each text value decoded,
/// the binary value as the base64 that kubectl reads, with the tag `!!binary`, and a text
/// that the check of the editor reads with no mark.
async fn the_editor_shows_each_text_value_decoded_and_a_binary_value_as_base64(k3s: &K3s, gateway: &Gateway) {
    let (data, _) = stored(k3s, "secret", "db").await;
    let key = ObjectKey::new(NAMESPACE, "db");
    let text = manifest::read_for_edit(gateway, Kind::Secret, &key, "").await.expect("the read");
    assert_eq!(text.decoded, Some(Decoded { texts: 3, binary: 1 }), "user, crlf and api-key are text");
    let blob = data.get("blob").map(|base64| format!("  blob: !!binary {base64}\n"));
    assert!(blob.is_some_and(|line| text.text.contains(&line)), "the key blob as kubectl reads it");
    assert!(text.text.contains("  user: |\n    mine\n"), "the key user decoded");
    assert!(!text.text.contains("managedFields"));
    let check = check::check(&text.text, &Identity { kind: Kind::Secret, key });
    assert!(check.marks.is_empty(), "the check of the text has {} marks", check.marks.len());
}

/// The draft of an immutable Secret refuses each edit, and the server refuses a save.
async fn immutable_objects_refuse_an_edit(k3s: &K3s, gateway: &Gateway) {
    let made = k3s.kubectl(&["create", "secret", "generic", "fixed", "--namespace", NAMESPACE, "--from-literal=a=1"]);
    made.await.expect("kubectl");
    k3s.kubectl(&["patch", "secret", "fixed", "--namespace", NAMESPACE, "--patch", r#"{"immutable":true}"#])
        .await
        .expect("kubectl");
    let object = read(gateway, KeyKind::Secret, "fixed").await;
    assert!(object.immutable);
    let mut draft = Draft::new(Arc::clone(&object));
    assert_eq!(draft.set("a", b"2".to_vec()), Err(Refusal::Immutable));
    let mut mutable = KeyObject::clone(&object);
    mutable.immutable = false;
    let mut forced = Draft::new(Arc::new(mutable));
    forced.set("a", b"2".to_vec()).expect("a change");
    let refused = save(gateway, &forced).await;
    assert!(matches!(&refused, WriteState::Failed(error) if error.status() == Some(422)), "{refused:?}");
}

/// A ConfigMap: text in `data`, bytes in `binaryData`. A key stays in its map.
async fn config_maps_keep_text_and_bytes_in_their_maps(k3s: &K3s, gateway: &Gateway) {
    let body = json!({
        "metadata": { "name": "web", "namespace": NAMESPACE },
        "data": { "app.conf": "port: 80\n", "mode": "fast", "old": "x" },
        "binaryData": { "logo": STANDARD.encode(BLOB), "keep": STANDARD.encode(b"\x01\x02") },
    });
    let path = Request::new(format!("/api/v1/namespaces/{NAMESPACE}/configmaps"));
    let create = path.create(&PostParams::default(), serde_json::to_vec(&body).expect("JSON")).expect("a request");
    gateway.send::<Json>(create).await.expect("the ConfigMap");
    let (data, binary) = stored(k3s, "configmap", "web").await;

    let object = read(gateway, KeyKind::ConfigMap, "web").await;
    let mut draft = Draft::new(object);
    draft.set("mode", b"slow".to_vec()).expect("a change of a text");
    draft.set("logo", b"\x89PNG".to_vec()).expect("a change of bytes");
    draft.set("app.conf", b"\xfe\xff".to_vec()).expect("a text that becomes bytes");
    draft.add("new.txt", b"hello".to_vec()).expect("a new text");
    draft.remove("old").expect("a removal");
    done(&save(gateway, &draft).await);

    let (new_data, new_binary) = stored(k3s, "configmap", "web").await;
    same_except(&data, &new_data, &["mode", "app.conf", "new.txt", "old"]);
    same_except(&binary, &new_binary, &["logo", "app.conf"]);
    assert!(new_data.get("mode").map(String::as_str) == Some("slow") && new_data.contains_key("new.txt"));
    assert!(!new_data.contains_key("old") && !new_data.contains_key("app.conf"), "the moved key left `data`");
    holds(&new_binary, "logo", b"\x89PNG");
    holds(&new_binary, "app.conf", b"\xfe\xff");
}

/// The certificates of a TLS Secret and the registries of an image pull Secret.
async fn special_types_give_their_views(gateway: &Gateway) {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["web.default.example".to_owned()]).expect("a certificate");
    let pem =
        |label: &str, der: &[u8]| format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", STANDARD.encode(der));
    let mut tls = NewSecret::new(ClusterId::new("default"), NAMESPACE, "web-tls", SecretType::Tls);
    tls.entries.insert("tls.crt".to_owned(), Value::new(pem("CERTIFICATE", cert.der()).into_bytes()));
    tls.entries.insert("tls.key".to_owned(), Value::new(pem("PRIVATE KEY", &signing_key.serialize_der()).into_bytes()));
    assert_eq!(tls.hints(), Vec::<String>::new());
    done(&WriteState::of_create(keys::create(gateway, &tls).await));
    let Special::Certificates(certificates) = read(gateway, KeyKind::Secret, "web-tls").await.special() else {
        panic!("a TLS Secret has certificates");
    };
    let names: Vec<Vec<String>> = certificates.into_iter().map(|found| found.expect("a certificate").names).collect();
    assert_eq!(names, [vec!["DNS:web.default.example".to_owned()]]);

    // "robot:hidden" in base64.
    let config = br#"{"auths":{"ghcr.io":{"auth":"cm9ib3Q6aGlkZGVu"}}}"#;
    let mut pull = NewSecret::new(ClusterId::new("default"), NAMESPACE, "pull", SecretType::DockerConfigJson);
    pull.entries.insert(".dockerconfigjson".to_owned(), Value::new(config.to_vec()));
    done(&WriteState::of_create(keys::create(gateway, &pull).await));
    let Special::Registries(Ok(registries)) = read(gateway, KeyKind::Secret, "pull").await.special() else {
        panic!("an image pull Secret has registries");
    };
    let users: Vec<_> = registries
        .iter()
        .map(|registry| (registry.server.as_str(), registry.username.as_deref(), registry.has_password))
        .collect();
    assert_eq!(users, [("ghcr.io", Some("robot"), true)]);
}
