use k8s_openapi::api::core::v1::{ConfigMap, Secret};

use super::*;
use crate::keys::value::{BinaryType, Form};

fn cluster() -> ClusterId {
    ClusterId::new("ams")
}

/// The answer of the server for a Secret with these fields besides the metadata.
fn secret(fields: &str) -> Secret {
    let text = format!(
        r#"{{"apiVersion":"v1","kind":"Secret","metadata":{{"name":"db","namespace":"shop","uid":"u-db","resourceVersion":"41"}}{fields}}}"#
    );
    serde_json::from_str(&text).expect("a Secret")
}

fn config_map(fields: &str) -> ConfigMap {
    let text = format!(
        r#"{{"apiVersion":"v1","kind":"ConfigMap","metadata":{{"name":"web","namespace":"shop","uid":"u-web","resourceVersion":"7"}}{fields}}}"#
    );
    serde_json::from_str(&text).expect("a ConfigMap")
}

fn forms(object: &KeyObject) -> Vec<(&str, Place, Form)> {
    object.entries.iter().map(|(key, entry)| (key.as_str(), entry.place, entry.value.form())).collect()
}

#[test]
fn a_secret_decodes_each_value_and_keeps_its_bytes_its_type_and_its_version() {
    // "admin\n", the bytes 00 ff 10, and "s3cr3t" with no line feed, in base64.
    let fields =
        r#","type":"kubernetes.io/basic-auth","data":{"username":"YWRtaW4K","blob":"AP8Q","password":"czNjcjN0"}"#;
    let object = KeyObject::of_secret(cluster(), secret(fields)).expect("the object");
    assert_eq!(
        object.target,
        Target { cluster: cluster(), kind: KeyKind::Secret, key: ObjectKey::new("shop", "db"), uid: "u-db".into() }
    );
    assert_eq!((&*object.version, object.immutable), ("41", false));
    assert_eq!(object.secret_type, Some(SecretType::BasicAuth));
    assert_eq!(
        forms(&object),
        [
            ("blob", Place::Data, Form::Binary(BinaryType::Unknown)),
            ("password", Place::Data, Form::Text),
            ("username", Place::Data, Form::Text),
        ]
    );
    assert_eq!(object.value("blob").map(Value::bytes), Some(&[0x00, 0xff, 0x10][..]));
    assert_eq!(object.value("username").and_then(Value::text), Some("admin\n"));
    assert_eq!(object.value("password").and_then(Value::text), Some("s3cr3t"));
    assert_eq!(object.special(), Special::None, "basic-auth has no special view");

    let debug = format!("{object:?}");
    assert!(!debug.contains("s3cr3t") && !debug.contains("admin") && !debug.contains("czNjcjN0"), "{debug}");
}

#[test]
fn a_secret_with_no_type_is_opaque_and_one_with_no_data_has_no_key() {
    let object = KeyObject::of_secret(cluster(), secret(r#","immutable":true"#)).expect("the object");
    assert_eq!((object.secret_type, object.immutable, object.entries.len()), (Some(SecretType::Opaque), true, 0));
    let helm = KeyObject::of_secret(cluster(), secret(r#","type":"helm.sh/release.v1""#)).expect("the object");
    assert_eq!(helm.secret_type, Some(SecretType::Other("helm.sh/release.v1".into())));
}

#[test]
fn a_config_map_has_text_in_data_and_bytes_in_binary_data() {
    // The bytes 1f 8b 08 00 (gzip) and the text "ok" in base64.
    let fields = r#","data":{"app.conf":"port: 80\n","banner":"\u001b[1mhi"},"binaryData":{"logo.gz":"H4sIAA==","plain":"b2s="}"#;
    let object = KeyObject::of_config_map(cluster(), config_map(fields)).expect("the object");
    assert_eq!((object.target.kind, &*object.version, &object.secret_type), (KeyKind::ConfigMap, "7", &None));
    assert_eq!(
        forms(&object),
        [
            ("app.conf", Place::Data, Form::Text),
            ("banner", Place::Data, Form::Binary(BinaryType::ControlText)),
            ("logo.gz", Place::BinaryData, Form::Binary(BinaryType::Gzip)),
            ("plain", Place::BinaryData, Form::Text),
        ]
    );
    assert_eq!(object.special(), Special::None);
    let immutable = KeyObject::of_config_map(cluster(), config_map(r#","immutable":true"#)).expect("the object");
    assert!(immutable.immutable && immutable.entries.is_empty());
}

#[test]
fn an_answer_with_a_key_in_both_maps_or_with_no_identity_is_an_error() {
    let both = config_map(r#","data":{"a":"x"},"binaryData":{"a":"eA=="}"#);
    let error = KeyObject::of_config_map(cluster(), both).expect_err("no server gives this");
    assert_eq!(error.to_string(), "the ConfigMap has a key in `data` and in `binaryData`");

    let mut no_uid = secret("");
    no_uid.metadata.uid = None;
    let error = KeyObject::of_secret(cluster(), no_uid).expect_err("no UID");
    assert_eq!(error.to_string(), "the answer of the server has no UID");
    let mut no_version = secret("");
    no_version.metadata.resource_version = Some(String::new());
    let error = KeyObject::of_secret(cluster(), no_version).expect_err("no version");
    assert_eq!(error.to_string(), "the answer of the server has no resource version");
    let mut no_name = config_map("");
    no_name.metadata.name = None;
    assert!(KeyObject::of_config_map(cluster(), no_name).is_err());
}

#[test]
fn the_written_result_of_an_object_is_its_identity_and_its_version() {
    let object = KeyObject::of_secret(cluster(), secret("")).expect("the object");
    let written = object.written().expect("a result");
    assert_eq!(written.key(), &ObjectKey::new("shop", "db"));
    assert_eq!(KeyKind::of(Kind::Secret).map(KeyKind::kind), Some(Kind::Secret));
    assert_eq!(KeyKind::of(Kind::ConfigMap), Some(KeyKind::ConfigMap));
    assert_eq!(KeyKind::of(Kind::Pod), None);
}
