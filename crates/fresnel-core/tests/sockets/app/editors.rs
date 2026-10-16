//! The reads and the saves of the editors (plan section 12.9): the text of each editor of
//! the UI, a save that does not go, and a replace with no answer.

use std::sync::Arc;
use std::time::Duration;

use fresnel_core::app::{Command, CoreSettings, CoreView, EditRequest, EditSave, EditState, Health, SaveState, Stage};
use fresnel_core::cluster::ClusterId;
use fresnel_core::config::{Edit, EditValue};
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::Decoded;
use fresnel_core::manifest::merge::Side;
use fresnel_core::net::{Limits, NetSettings};
use fresnel_core::ops::Target;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{FakeApiServer, Fault};
use http::Method;

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

/// A save of an editor in a read-only cluster ends as not sent with the reason, also from a
/// view that did not close its gate, and the server gets no write; a save of an editor that
/// is not open does not go either. The journal tells both. A dry run in a cluster that is
/// not read-only goes as a PUT with `dryRun`, and its answer is the text of the editor.
#[tokio::test(flavor = "multi_thread")]
async fn a_save_in_a_read_only_cluster_and_of_a_closed_editor_does_not_go() {
    let ams = FakeApiServer::start().await.expect("a server");
    let secret = r#"{"apiVersion":"v1","kind":"Secret","metadata":{"name":"db","namespace":"shop","uid":"u-db",
        "resourceVersion":"7"},"data":{"password":"aHVudGVyMg=="}}"#;
    ams.set_body(SECRET, secret);
    let home = home("editors-read-only");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.until_health("ams", Health::Live).await;
    app.core.send(Command::ShowEditors(vec![request(1, "ams")]));
    let view = app.until("the text", |view| read(view, 1).is_some_and(|state| state.is_some())).await;
    let Some(EditState::Done(base)) = view.edits.first().map(|read| read.state.clone()) else { panic!("a text") };
    let save = |number: u64, editor: u64| EditSave {
        editor: request(editor, "ams"),
        number,
        base: Arc::clone(&base),
        text: base.text.replace("hunter2", "hunter3").into(),
        stage: Stage::Check,
    };
    let state = |view: &CoreView| view.edits.first().and_then(|read| read.save.clone()).map(|view| view.state);
    let puts = || ams.requests().into_iter().filter(|seen| seen.method == Method::PUT).count();

    app.core.send(Command::EditSettings(vec![Edit::new(&["clusters", "ams", "read_only"], EditValue::Flag(true))]));
    app.until("the read-only cluster", |view| view.config.read_only(&ClusterId::new("ams"))).await;
    app.core.send(Command::SaveEditor(save(1, 1)));
    let refused = app.until("the refusal", |view| state(view).is_some()).await;
    let refusal = "ams is read-only in the settings: the app makes no change in it.";
    assert!(matches!(state(&refused), Some(SaveState::NotSent(error)) if error.to_string() == refusal));
    let texts = app.until_texts("ams", |texts| texts.iter().any(|text| text.contains("did not go"))).await;
    assert!(texts.contains(&format!("the save of Secret shop/db did not go: {refusal}")), "{texts:#?}");
    assert_eq!(puts(), 0, "no write");

    app.core.send(Command::EditSettings(vec![Edit::new(&["clusters", "ams", "read_only"], EditValue::Flag(false))]));
    app.until("a cluster that takes changes", |view| !view.config.read_only(&ClusterId::new("ams"))).await;
    app.core.send(Command::SaveEditor(save(2, 9)));
    let closed = "the save of Secret shop/db did not go: its editor is not open";
    app.until_texts("ams", |texts| texts.iter().any(|text| text == closed)).await;
    app.core.send(Command::SaveEditor(save(3, 1)));
    let checked = app.until("the dry run", |view| matches!(state(view), Some(SaveState::Checked(_)))).await;
    assert!(matches!(state(&checked), Some(SaveState::Checked(text)) if text.text.contains("password: hunter2")));
    let put = ams.requests().into_iter().find(|seen| seen.method == Method::PUT).expect("the PUT of the dry run");
    let query = put.uri.query().unwrap_or_default().to_owned();
    assert_eq!((put.uri.path(), query.as_str()), (SECRET, "&dryRun=All&fieldManager=fresnel&fieldValidation=Strict"));
}

/// A replace with no answer ends as not known and goes one time only (design rule 6). The
/// save reads the object after it: with that read, the text of the user merges with the
/// object; with no read, the state tells why the read failed.
#[tokio::test(flavor = "multi_thread")]
async fn a_replace_with_no_answer_is_not_known_and_reads_the_object_again() {
    let ams = FakeApiServer::start().await.expect("a server");
    let secret = r#"{"apiVersion":"v1","kind":"Secret","metadata":{"name":"db","namespace":"shop","uid":"u-db",
        "resourceVersion":"7"},"data":{"password":"aHVudGVyMg=="}}"#;
    ams.set_body(SECRET, secret);
    let home = home("editors-unknown");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false)]);
    let limits = Limits { write: Duration::from_secs(2), get: Duration::from_secs(2), ..Limits::default() };
    let app = App::started(
        &home,
        CoreSettings { net: NetSettings { limits, ..NetSettings::default() }, ..CoreSettings::default() },
    );
    app.until_health("ams", Health::Live).await;
    app.core.send(Command::ShowEditors(vec![request(1, "ams")]));
    let view = app.until("the text", |view| read(view, 1).is_some_and(|state| state.is_some())).await;
    let Some(EditState::Done(base)) = view.edits.first().map(|read| read.state.clone()) else { panic!("a text") };
    let mine = base.text.replace("hunter2", "hunter3");
    let save = |number: u64| EditSave {
        editor: request(1, "ams"),
        number,
        base: Arc::clone(&base),
        text: mine.as_str().into(),
        stage: Stage::Send,
    };
    let state = |view: &CoreView| view.edits.first().and_then(|read| read.save.clone()).map(|view| view.state);
    let ended = |number: u64| {
        move |view: &CoreView| {
            let save = view.edits.first().and_then(|read| read.save.clone());
            save.is_some_and(|save| save.save.number == number && save.state != SaveState::Pending)
        }
    };
    let puts = || ams.requests().into_iter().filter(|seen| seen.method == Method::PUT).count();

    // The replace and the read after it get no answer.
    ams.add_fault(Fault::no_answer().on_path(SECRET));
    app.core.send(Command::SaveEditor(save(1)));
    let view = app.until("the end of the save", ended(1)).await;
    let Some(SaveState::Unknown(error, None)) = state(&view) else { panic!("not known: {:?}", state(&view)) };
    assert!(error.to_string().contains(", and the read after it failed: "), "{error}");
    assert_eq!(puts(), 1, "the replace went one time");

    // The fault ends during the replace, before the read after it.
    ams.clear_faults();
    ams.add_fault(Fault::no_answer().on_path(SECRET).for_time(Duration::from_secs(1)));
    app.core.send(Command::SaveEditor(save(2)));
    let view = app.until("the end of the save", ended(2)).await;
    let Some(SaveState::Unknown(_, Some(remote))) = state(&view) else { panic!("a read: {:?}", state(&view)) };
    assert!(remote.merge.is_clean() && remote.merge.text(|_| Side::Ours) == mine, "{:?}", remote.merge);
    assert_eq!(puts(), 2, "each replace went one time");
}
