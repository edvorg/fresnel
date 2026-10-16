//! The reads of the editors (plan section 12.9): the text of each editor of the UI.

use std::sync::Arc;

use fresnel_core::app::{Command, CoreSettings, CoreView, EditRequest, EditState, Health};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::Decoded;
use fresnel_core::ops::Target;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{FakeApiServer, Fault};

use super::{App, home, write_kubeconfig};

const SECRET: &str = "/api/v1/namespaces/shop/secrets/db";

fn request(number: u64, cluster: &str) -> EditRequest {
    let key = ObjectKey::new("shop", "db");
    EditRequest {
        number,
        target: Target { cluster: ClusterId::new(cluster), kind: Kind::Secret, key, uid: "u-db".into() },
    }
}

/// The state of the read of an editor: its text, its error, or `None` while it reads.
fn read(view: &CoreView, number: u64) -> Option<Option<Result<String, String>>> {
    let read = view.edits.iter().find(|read| read.request.number == number)?;
    Some(match &read.state {
        EditState::Reading => None,
        EditState::Done(text) => Some(Ok(text.text.to_string())),
        EditState::Failed(error) => Some(Err(error.to_string())),
    })
}

/// A cluster that does not answer holds no editor of an other cluster, and an editor that
/// closes leaves the view.
#[tokio::test(flavor = "multi_thread")]
async fn each_editor_gets_the_text_of_its_object_and_a_cluster_that_does_not_answer_holds_no_other() {
    let (ams, fra) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    let secret = r#"{"apiVersion":"v1","kind":"Secret","metadata":{"name":"db","namespace":"shop","uid":"u-db",
        "resourceVersion":"7","managedFields":[{"manager":"kubectl"}]},"data":{"password":"aHVudGVyMg==","raw":"/wA="}}"#;
    ams.set_body(SECRET, secret);
    fra.add_fault(Fault::no_answer().on_path(SECRET));
    let home = home("editors");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false), ("fra", fra.addr(), &fra, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.select("fra", true);
    app.until_health("ams", Health::Live).await;
    app.until_health("fra", Health::Live).await;

    let requests = vec![request(1, "fra"), request(2, "ams"), request(3, "absent")];
    app.core.send(Command::ShowEditors(requests.clone()));
    let view = app.until("the text of ams", |view| read(view, 2).is_some_and(|state| state.is_some())).await;
    let expected = "apiVersion: v1\ndata:\n  password: hunter2\n  raw: !!binary /wA=\nkind: Secret\nmetadata:\n  name: db\n  \
                    namespace: shop\n  resourceVersion: \"7\"\n  uid: u-db\n";
    assert_eq!(read(&view, 2), Some(Some(Ok(expected.to_owned()))), "decoded, with no managedFields");
    let done = view.edits.iter().find(|read| read.request.number == 2).map(|read| &read.state);
    let Some(EditState::Done(text)) = done else { panic!("the text of ams") };
    assert_eq!(
        (text.version.as_deref(), text.lines, text.decoded),
        (Some("7"), 10, Some(Decoded { texts: 1, binary: 1 }))
    );
    assert_eq!(read(&view, 1), Some(None), "fra does not answer, and its read waits");
    let absent = app.until("the read of absent", |view| read(view, 3).is_some_and(|state| state.is_some())).await;
    assert_eq!(read(&absent, 3), Some(Some(Err("absent is not connected".to_owned()))));

    // The editor of fra closes: its read leaves the view, and the others stay as they are.
    app.core.send(Command::ShowEditors(requests[1..].to_vec()));
    let closed = app.until("no read of fra", |view| read(view, 1).is_none()).await;
    assert_eq!(closed.edits.len(), 2);
    assert!(Arc::ptr_eq(&closed.edits[0], &view.edits[1]), "a done read stays the same read");
    let reads = ams.requests().into_iter().filter(|seen| seen.uri.path() == SECRET).count();
    assert_eq!(reads, 1, "one read for one editor");
}

/// A failed read ends: the editor asks again with a new request, which reads the object
/// again. A read that finds an other object of the name, with an other UID, fails.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_request_reads_again_and_an_other_uid_fails() {
    let ams = FakeApiServer::start().await.expect("a server");
    let secret = r#"{"apiVersion":"v1","kind":"Secret","metadata":{"name":"db","namespace":"shop","uid":"u-db",
        "resourceVersion":"7"},"data":{"password":"aHVudGVyMg=="}}"#;
    ams.set_body(SECRET, secret);
    ams.add_fault(Fault::not_found().on_path(SECRET));
    let home = home("editors-again");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.until_health("ams", Health::Live).await;

    app.core.send(Command::ShowEditors(vec![request(1, "ams")]));
    let failed = app.until("the failed read", |view| read(view, 1).is_some_and(|state| state.is_some())).await;
    assert!(matches!(read(&failed, 1), Some(Some(Err(_)))), "HTTP 404");
    ams.clear_faults();
    let mut other = request(3, "ams");
    other.target.uid = "u-old".into();
    app.core.send(Command::ShowEditors(vec![request(2, "ams"), other]));
    let done = |view: &CoreView| [2, 3].iter().all(|number| read(view, *number).is_some_and(|state| state.is_some()));
    let view = app.until("the reads again", done).await;
    assert!(matches!(read(&view, 2), Some(Some(Ok(text))) if text.contains("password: hunter2")), "the new read");
    let uid = "the object of this name is an other object now: it has an other UID";
    assert_eq!(read(&view, 3), Some(Some(Err(uid.to_owned()))));
}
