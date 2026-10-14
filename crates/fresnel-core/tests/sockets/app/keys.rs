//! The key view in the core (plan section 12.2): the object of the view loads while the view
//! is open, a new version in the store gives a new read, a save and a create go to the
//! cluster of their object only (weakness 9 of plan section 6), and a save in progress
//! ends also when the view shows an other object.

use std::sync::Arc;

use fresnel_core::app::{Command, CoreSettings, CoreView, Health, KeysView, ReadState};
use fresnel_core::cluster::ClusterId;
use fresnel_core::journal::Level;
use fresnel_core::keys::{Draft, KeyKind, NewSecret, SecretType, Target, WriteState};
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{FakeApiServer, Fault, FaultProxy, ProxyMode};
use http::Method;

use super::{App, home, write_kubeconfig};

pub(super) const DB: &str = "/api/v1/namespaces/shop/secrets/db";
const SECRETS_OF_SHOP: &str = "/api/v1/namespaces/shop/secrets";

/// The answer of the server for the Secret `db` with one value and this version.
pub(super) fn secret(name: &str, version: u32, password: &str) -> String {
    format!(
        r#"{{"kind":"Secret","apiVersion":"v1","metadata":{{"name":"{name}","namespace":"shop","uid":"u-{name}",
        "resourceVersion":"{version}"}},"type":"Opaque","data":{{"password":"{password}"}}}}"#
    )
}

/// The modified event of the Secret `db` in the metadata store, with this version.
fn modified(version: u32) -> String {
    let metadata = format!(r#"{{"name":"db","namespace":"shop","uid":"u-db","resourceVersion":"{version}"}}"#);
    format!(r#"{{"type":"MODIFIED","object":{{"kind":"PartialObjectMetadata","metadata":{metadata}}}}}"#)
}

pub(super) fn target(cluster: &str) -> Target {
    named(cluster, "db")
}

pub(super) fn named(cluster: &str, name: &str) -> Target {
    Target {
        cluster: ClusterId::new(cluster),
        kind: KeyKind::Secret,
        key: ObjectKey::new("shop", name),
        uid: "".into(),
    }
}

/// The key view of a cluster in the view of the core.
pub(super) fn keys(view: &CoreView, cluster: &str) -> Option<Arc<KeysView>> {
    view.clusters.iter().find(|row| row.id.as_str() == cluster)?.data.as_ref()?.keys.clone()
}

/// The value of `password` of the object of the key view, once read.
pub(super) fn password(view: &CoreView, cluster: &str) -> Option<Vec<u8>> {
    let keys = keys(view, cluster).filter(|keys| keys.read == ReadState::Read)?;
    keys.object.as_ref()?.value("password").map(|value| value.bytes().to_vec())
}

pub(super) fn requests(server: &FakeApiServer, method: &Method, path: &str) -> usize {
    server.requests().iter().filter(|seen| seen.method == method && seen.uri.path() == path).count()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_object_of_the_key_view_loads_while_the_view_is_open_and_a_save_goes_to_its_cluster_only() {
    let (ams, fra) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    // "ams-1" and "fra-1" in base64: the two clusters have a Secret of the same name.
    ams.set_body(DB, &secret("db", 41, "YW1zLTE="));
    fra.set_body(DB, &secret("db", 41, "ZnJhLTE="));
    let home = home("keys");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false), ("fra", fra.addr(), &fra, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.select("fra", true);
    app.until_health("ams", Health::Live).await;
    app.until_health("fra", Health::Live).await;
    assert_eq!(requests(&ams, &Method::GET, DB), 0, "no view is open: no value is read");

    app.core.send(Command::ShowKeys(Some(target("ams"))));
    let open = app.until("the Secret of ams", |view| password(view, "ams").is_some()).await;
    assert!(password(&open, "ams").as_deref() == Some(b"ams-1"), "the value of ams");
    assert!(keys(&open, "fra").is_none(), "the view shows the object of ams only");

    // A new version in the metadata store of the Secrets gives a new read.
    ams.set_body(DB, &secret("db", 42, "YW1zLTI="));
    assert!(ams.send_event(&Kind::Secret.info().path(), &modified(42)) > 0, "the watch of the Secrets");
    let changed = app.until("the new version", |view| password(view, "ams").as_deref() == Some(b"ams-2")).await;
    let object = keys(&changed, "ams").and_then(|keys| keys.object.clone()).expect("the object");
    assert_eq!(&*object.version, "42");

    // The save of an edit goes to ams, and its answer is the new object.
    let mut draft = Draft::new(object);
    draft.set("password", b"ams-3".to_vec()).expect("a change");
    let save = draft.save().expect("a save");
    ams.set_body(DB, &secret("db", 43, "YW1zLTM="));
    app.core.send(Command::SaveKeys(save.clone()));
    let saved = app
        .until("the save", |view| {
            keys(view, "ams").and_then(|keys| keys.save.clone()).is_some_and(|job| job.state != WriteState::Pending)
        })
        .await;
    let job = keys(&saved, "ams").and_then(|keys| keys.save.clone()).expect("the job");
    assert_eq!(job.save, save);
    let WriteState::Done(answer) = &job.state else { panic!("a done save: {job:?}") };
    assert_eq!(&*answer.version, "43");
    assert!(password(&saved, "ams").as_deref() == Some(b"ams-3"), "the value of the answer");
    assert_eq!((requests(&ams, &Method::PATCH, DB), requests(&fra, &Method::PATCH, DB)), (1, 0));

    // The view closes: the core holds no value. A save with no open view does not go.
    app.core.send(Command::ShowKeys(None));
    app.until("no key view", |view| keys(view, "ams").is_none()).await;
    app.core.send(Command::SaveKeys(save));
    let warned = app.until_texts("ams", |texts| texts.iter().any(|text| text.contains("did not go"))).await;
    assert!(warned.contains(&"the save of Secret shop/db did not go: its key view is not open".to_owned()));
    assert!(app.journal().iter().any(|(level, _, text)| *level == Level::Warning && text.contains("did not go")));
    assert_eq!(requests(&ams, &Method::PATCH, DB), 1, "no second PATCH");

    // A create goes to the cluster of the new Secret.
    fra.set_body(SECRETS_OF_SHOP, &secret("api", 9, "YQ=="));
    let new = NewSecret::new(ClusterId::new("fra"), "shop", "api", SecretType::Opaque);
    app.core.send(Command::CreateSecret(new.clone()));
    let created =
        app.until("the create", |view| view.create.as_ref().is_some_and(|job| job.state != WriteState::Pending)).await;
    let job = created.create.clone().expect("the job");
    assert_eq!((job.number, &job.secret), (1, &new));
    assert!(
        matches!(&job.state, WriteState::Done(secret) if secret.target.key == ObjectKey::new("shop", "api")),
        "{job:?}"
    );
    assert_eq!(
        (requests(&fra, &Method::POST, SECRETS_OF_SHOP), requests(&ams, &Method::POST, SECRETS_OF_SHOP)),
        (1, 0)
    );

    let absent = NewSecret::new(ClusterId::new("absent"), "shop", "api", SecretType::Opaque);
    app.core.send(Command::CreateSecret(absent));
    let refused = app
        .until("the second create", |view| {
            view.create.as_ref().is_some_and(|job| job.number == 2 && job.state != WriteState::Pending)
        })
        .await;
    let state = refused.create.map(|job| job.state.clone());
    assert!(
        matches!(&state, Some(WriteState::NotSent(error)) if error.to_string() == "absent is not connected"),
        "{state:?}"
    );
}

/// The stop of a write in progress would leave its result unknown. So a holder whose save
/// waits stays when the view shows an other object, sends the save, and then drops its values.
/// No view shows the end of that save, so the journal does, with no value.
#[tokio::test(flavor = "multi_thread")]
async fn a_save_that_waits_goes_out_also_when_the_view_shows_an_other_object() {
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    server.set_body(DB, &secret("db", 41, "YW1zLTE="));
    let home = home("keys-leave");
    write_kubeconfig(&home, "ams", &[("ams", proxy.addr(), &server, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.until_health("ams", Health::Live).await;
    app.core.send(Command::ShowKeys(Some(target("ams"))));
    let open = app.until("the Secret", |view| password(view, "ams").is_some()).await;
    let mut draft = Draft::new(keys(&open, "ams").and_then(|keys| keys.object.clone()).expect("the object"));
    draft.set("password", b"new".to_vec()).expect("a change");
    let save = draft.save().expect("a save");
    app.core.send(Command::ShowKeys(None));
    app.until("no key view", |view| keys(view, "ams").is_none()).await;

    // The read of a new view waits at the frozen proxy, so the save waits for it.
    proxy.set(ProxyMode::Freeze);
    app.core.send(Command::ShowKeys(Some(target("ams"))));
    app.core.send(Command::SaveKeys(save));
    app.core.send(Command::ShowKeys(Some(named("ams", "api"))));
    let moved = app
        .until("the view of api", |view| keys(view, "ams").is_some_and(|keys| keys.target.key.name.as_ref() == "api"))
        .await;
    assert!(keys(&moved, "ams").is_some_and(|keys| keys.save.is_none()), "the view of api has no save");
    assert_eq!(requests(&server, &Method::PATCH, DB), 0);

    proxy.set(ProxyMode::Pass);
    let texts = app.until_texts("ams", |texts| texts.iter().any(|text| text.contains("key view closed"))).await;
    let expected = "the save of Secret shop/db ended after its key view closed: done";
    assert!(texts.iter().any(|text| text == expected), "{texts:?}");
    assert_eq!(requests(&server, &Method::PATCH, DB), 1, "the save of the view before");
}

/// Design rule 5: a read that fails keeps the object of the read before, with the reason.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_that_fails_keeps_the_object_before_and_the_next_version_reads_again() {
    let server = FakeApiServer::start().await.expect("a server");
    server.set_body(DB, &secret("db", 41, "YW1zLTE="));
    let home = home("keys-fail");
    write_kubeconfig(&home, "ams", &[("ams", server.addr(), &server, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.until_health("ams", Health::Live).await;
    app.core.send(Command::ShowKeys(Some(target("ams"))));
    app.until("the Secret", |view| password(view, "ams").is_some()).await;

    server.add_fault(Fault::forbidden().on_path(DB));
    assert!(server.send_event(&Kind::Secret.info().path(), &modified(42)) > 0, "the watch of the Secrets");
    let failed = app
        .until("the failed read", |view| {
            keys(view, "ams").is_some_and(|keys| keys.read != ReadState::Read && keys.read != ReadState::Reading)
        })
        .await;
    let view = keys(&failed, "ams").expect("the key view");
    let ReadState::Failed(error) = &view.read else { panic!("a failed read: {view:?}") };
    assert_eq!(error.status(), Some(403));
    let kept = view.object.as_ref().and_then(|object| object.value("password")).map(|value| value.bytes().to_vec());
    assert!(kept.as_deref() == Some(b"ams-1"), "the object of the read before stays");

    server.clear_faults();
    server.set_body(DB, &secret("db", 43, "YW1zLTM="));
    assert!(server.send_event(&Kind::Secret.info().path(), &modified(43)) > 0);
    app.until("the read of the next version", |view| password(view, "ams").as_deref() == Some(b"ams-3")).await;
}
