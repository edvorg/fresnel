//! The manifest of one object on the fake API server (plan section 12.9).

use fresnel_core::kinds::Kind;
use fresnel_core::manifest::{self, ManagedFields};
use fresnel_core::net::{ErrorClass, NetSettings};
use fresnel_core::store::ObjectKey;
use fresnel_testkit::Fault;
use http::Method;
use http::header::ACCEPT_ENCODING;

use crate::bed::Bed;

const POD: &str = "/api/v1/namespaces/shop/pods/web";
const NODE: &str = "/api/v1/nodes/k3s";

#[tokio::test]
async fn the_manifest_is_the_yaml_of_the_object_with_no_managed_fields_until_the_switch_keeps_them() {
    let bed = Bed::start(NetSettings::default()).await;
    let pod = r#"{"apiVersion":"v1","kind":"Pod","metadata":{"name":"web","namespace":"shop","resourceVersion":"42",
        "managedFields":[{"manager":"kubectl","operation":"Update"}],
        "annotations":{"kubectl.kubernetes.io/last-applied-configuration":"{}"}},
        "spec":{"containers":[{"name":"web","image":"nginx:1.27"}]}}"#;
    bed.server.set_body(POD, pod);
    let key = ObjectKey::new("shop", "web");

    let removed =
        manifest::fetch(&bed.link.gateway, Kind::Pod, &key, "", ManagedFields::Removed).await.expect("a manifest");
    let expected = "apiVersion: v1\nkind: Pod\nmetadata:\n  annotations:\n    kubectl.kubernetes.io/last-applied-configuration: \
                    \"{}\"\n  name: web\n  namespace: shop\n  resourceVersion: \"42\"\nspec:\n  containers:\n  - image: \
                    nginx:1.27\n    name: web\n";
    assert_eq!(removed.text, expected, "the keys in the sequence of kubectl, and the copy of `kubectl apply` stays");
    assert_eq!((removed.version.as_deref(), removed.managed_fields), (Some("42"), ManagedFields::Removed));

    let kept = manifest::fetch(&bed.link.gateway, Kind::Pod, &key, "", ManagedFields::Kept).await.expect("a manifest");
    assert!(kept.text.contains("managedFields:") && kept.text.contains("manager: kubectl"), "{}", kept.text);
    assert_eq!(kept.text.replace(MANAGED, ""), removed.text, "the switch changes nothing else");

    // A kind with no namespace has its path with no namespace.
    bed.server.set_body(NODE, r#"{"apiVersion":"v1","kind":"Node","metadata":{"name":"k3s","resourceVersion":"9"}}"#);
    let node = ObjectKey::new("", "k3s");
    let node = manifest::fetch(&bed.link.gateway, Kind::Node, &node, "", ManagedFields::Removed).await;
    assert_eq!(node.expect("a manifest").version.as_deref(), Some("9"));

    let reads: Vec<_> = bed.server.requests().into_iter().filter(|seen| seen.uri.path() != "/version").collect();
    let paths: Vec<&str> = reads.iter().map(|seen| seen.uri.path()).collect();
    assert_eq!(paths, [POD, POD, NODE], "one GET of the object for each fetch");
    assert!(reads.iter().all(|seen| seen.method == Method::GET && seen.headers.contains_key(ACCEPT_ENCODING)));
}

#[tokio::test]
async fn a_manifest_that_the_server_refuses_is_an_error_with_its_class() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.add_fault(Fault::forbidden().on_path("/api/v1/namespaces/shop/secrets"));
    bed.server.add_fault(Fault::not_found().on_path(POD));
    let gateway = &bed.link.gateway;
    let (db, web, none) = (ObjectKey::new("shop", "db"), ObjectKey::new("shop", "web"), ObjectKey::new("shop", ""));

    let refused = manifest::fetch(gateway, Kind::Secret, &db, "", ManagedFields::Removed).await.expect_err("HTTP 403");
    assert_eq!(refused.class(), ErrorClass::Forbidden);
    let gone = manifest::fetch(gateway, Kind::Pod, &web, "", ManagedFields::Kept).await.expect_err("HTTP 404");
    assert_eq!(
        (gone.status(), gone.to_string().as_str()),
        (Some(404), "the server could not find the requested resource (HTTP 404)")
    );
    let error = manifest::fetch(gateway, Kind::Pod, &none, "", ManagedFields::Removed).await.expect_err("no name");
    assert_eq!(error.class(), ErrorClass::Invalid);
}

/// Bad input and bad data: a key with no namespace for a kind in a namespace would read the
/// path `namespaces//pods/web`, and a server can answer a read with some other object, as
/// the fake server answers a path with no body with an empty list. Neither shows as the
/// manifest of the object.
#[tokio::test]
async fn a_key_with_no_namespace_sends_nothing_and_an_answer_with_an_other_object_is_an_error() {
    let bed = Bed::start(NetSettings::default()).await;
    let gateway = &bed.link.gateway;
    let fetch = async |key: ObjectKey| manifest::fetch(gateway, Kind::Pod, &key, "", ManagedFields::Removed).await;
    let reads = || bed.server.requests().into_iter().filter(|seen| seen.uri.path() != "/version").count();

    let no_namespace = fetch(ObjectKey::new("", "web")).await.expect_err("a pod is in a namespace");
    assert_eq!(no_namespace.class(), ErrorClass::Invalid);
    assert_eq!(reads(), 0, "no request");

    let list = fetch(ObjectKey::new("shop", "web")).await.expect_err("an empty list is not the pod");
    assert_eq!(list.class(), ErrorClass::Invalid);
    bed.server.set_body(POD, r#"{"apiVersion":"v1","kind":"Pod","metadata":{"name":"api","namespace":"shop"}}"#);
    let other = fetch(ObjectKey::new("shop", "web")).await.expect_err("an other pod");
    assert_eq!(other.to_string(), "the server answered the read of the manifest with an other object");
    assert_eq!(reads(), 2);
}

/// New data during a read: a StatefulSet deletes its pod `db-0` and makes a new `db-0`.
/// The read for the panel of the old pod gets the new one, which is an other object.
#[tokio::test]
async fn an_answer_with_the_name_and_an_other_uid_is_an_error_and_an_empty_uid_takes_any() {
    let bed = Bed::start(NetSettings::default()).await;
    let gateway = &bed.link.gateway;
    bed.server
        .set_body(POD, r#"{"apiVersion":"v1","kind":"Pod","metadata":{"name":"web","namespace":"shop","uid":"new"}}"#);
    let key = ObjectKey::new("shop", "web");
    let fetch = async |uid: &str| manifest::fetch(gateway, Kind::Pod, &key, uid, ManagedFields::Removed).await;

    let old = fetch("old").await.expect_err("the pod of the panel is gone");
    assert_eq!(
        (old.class(), old.to_string().as_str()),
        (ErrorClass::Invalid, "the object of this name is an other object now: it has an other UID")
    );
    assert!(fetch("new").await.expect("the same pod").text.contains("uid: new"));
    assert!(fetch("").await.expect("any pod of the name").text.contains("uid: new"));
}

/// The `managedFields` of the pod of the first test, as the YAML writer gives them.
const MANAGED: &str = "  managedFields:\n  - manager: kubectl\n    operation: Update\n";
