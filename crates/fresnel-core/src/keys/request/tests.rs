use http::Method;
use http::header::CONTENT_TYPE;

use super::*;
use crate::cluster::ClusterId;
use crate::keys::draft::KeyWrite;
use crate::keys::secret_type::SecretType;
use crate::store::ObjectKey;

fn target(kind: KeyKind, namespace: &str) -> Target {
    Target { cluster: ClusterId::new("ams"), kind, key: ObjectKey::new(namespace, "db"), uid: "u-db".into() }
}

fn write(key: &str, new: Option<(Place, &[u8])>, old: Option<Place>) -> KeyWrite {
    KeyWrite { key: key.to_owned(), new: new.map(|(place, bytes)| (place, Value::new(bytes.to_vec()))), old }
}

fn body(request: &Request<Vec<u8>>) -> Json {
    serde_json::from_slice(request.body()).expect("a JSON body")
}

#[test]
fn the_read_is_a_get_of_the_object_in_its_namespace() {
    let request = read(&target(KeyKind::ConfigMap, "shop")).expect("a request");
    assert_eq!(
        (request.method(), request.uri().to_string()),
        (&Method::GET, "/api/v1/namespaces/shop/configmaps/db".to_owned())
    );
    let error = read(&target(KeyKind::Secret, "")).expect_err("no namespace");
    assert_eq!(error.to_string(), "a Secret is in a namespace, and the request has none");
}

#[test]
fn the_save_of_a_secret_is_a_merge_patch_of_the_changed_keys_in_base64_with_the_version() {
    let save = Save {
        target: target(KeyKind::Secret, "shop"),
        version: "41".into(),
        writes: vec![
            write("password", Some((Place::Data, b"new value")), Some(Place::Data)),
            write("binary", Some((Place::Data, b"\x00\xff")), Some(Place::Data)),
            write("old", None, Some(Place::Data)),
            write("added", Some((Place::Data, b"")), None),
        ],
    };
    let request = super::save(&save).expect("a request");
    assert_eq!(request.method(), Method::PATCH);
    assert_eq!(request.uri().to_string(), "/api/v1/namespaces/shop/secrets/db?&fieldManager=fresnel");
    assert_eq!(
        request.headers().get(CONTENT_TYPE).map(|value| value.as_bytes()),
        Some(&b"application/merge-patch+json"[..])
    );
    assert_eq!(
        body(&request),
        json!({
            "metadata": { "resourceVersion": "41" },
            "data": { "password": "bmV3IHZhbHVl", "binary": "AP8=", "old": null, "added": "" }
        }),
        "only the changed keys, never `stringData`"
    );
}

#[test]
fn the_save_of_a_config_map_puts_text_into_data_and_bytes_into_binary_data_and_clears_a_moved_key() {
    let save = Save {
        target: target(KeyKind::ConfigMap, "shop"),
        version: "7".into(),
        writes: vec![
            write("conf", Some((Place::Data, b"port: 81\n")), Some(Place::Data)),
            write("logo", Some((Place::BinaryData, b"\x89PNG")), Some(Place::BinaryData)),
            write("moved", Some((Place::BinaryData, b"\xfe")), Some(Place::Data)),
            write("gone", None, Some(Place::BinaryData)),
        ],
    };
    let request = super::save(&save).expect("a request");
    assert_eq!(request.uri().path(), "/api/v1/namespaces/shop/configmaps/db");
    assert_eq!(
        body(&request),
        json!({
            "metadata": { "resourceVersion": "7" },
            "data": { "conf": "port: 81\n", "moved": null },
            "binaryData": { "logo": "iVBORw==", "moved": "/g==", "gone": null }
        })
    );

    let bad = Save { writes: vec![write("conf", Some((Place::Data, b"\xfe")), None)], ..save };
    let error = super::save(&bad).expect_err("bytes in data");
    assert_eq!(error.to_string(), "a value in `data` of a ConfigMap must be UTF-8");
}

#[test]
fn the_create_of_a_secret_has_its_name_its_type_and_each_value_in_base64() {
    let mut secret = NewSecret::new(ClusterId::new("ams"), "shop", "web-tls", SecretType::Tls);
    secret.entries.insert("tls.crt".to_owned(), Value::new(b"crt".to_vec()));
    secret.annotations.insert("team".to_owned(), "shop".to_owned());
    let request = create(&secret).expect("a request");
    assert_eq!(request.method(), Method::POST);
    assert_eq!(request.uri().to_string(), "/api/v1/namespaces/shop/secrets?&fieldManager=fresnel");
    assert_eq!(
        body(&request),
        json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": { "name": "web-tls", "namespace": "shop", "annotations": { "team": "shop" } },
            "type": "kubernetes.io/tls",
            "data": { "tls.crt": "Y3J0", "tls.key": "" }
        })
    );

    let plain = NewSecret::new(ClusterId::new("ams"), "shop", "plain", SecretType::Opaque);
    let created = body(&create(&plain).expect("a request"));
    assert_eq!((created.pointer("/metadata/annotations"), &created["data"]), (None, &json!({})));
    let nowhere = NewSecret { namespace: String::new(), ..plain };
    assert!(create(&nowhere).is_err(), "no namespace");
}
