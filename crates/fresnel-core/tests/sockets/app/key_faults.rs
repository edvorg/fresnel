//! The key view in the core with bad data: the answer of a save with an other object.

use std::sync::Arc;

use fresnel_core::app::{Command, CoreSettings, Health, KeysView, ReadState, SaveJob};
use fresnel_core::keys::{Draft, KeyObject, Save, WriteState};
use fresnel_testkit::FakeApiServer;

use super::keys::{DB, keys, password, secret, target};
use super::{App, home, write_kubeconfig};

/// A core with the key view of the Secret `db` of the cluster "ams" open.
struct Bed {
    server: FakeApiServer,
    app: App,
    /// The read of the view.
    object: Arc<KeyObject>,
}

impl Bed {
    async fn open(label: &str) -> Self {
        let server = FakeApiServer::start().await.expect("a server");
        server.set_body(DB, &secret("db", 41, "YW1zLTE="));
        let home = home(label);
        write_kubeconfig(&home, "ams", &[("ams", server.addr(), &server, false)]);
        let app = App::started(&home, CoreSettings::default());
        app.until_health("ams", Health::Live).await;
        app.core.send(Command::ShowKeys(Some(target("ams"))));
        let open = app.until("the Secret", |view| password(view, "ams").is_some()).await;
        let object = keys(&open, "ams").and_then(|keys| keys.object.clone()).expect("the object");
        Self { server, app, object }
    }

    /// The save of a new value of `password` on the read of the view.
    fn save(&self, value: &[u8]) -> Save {
        let mut draft = Draft::new(Arc::clone(&self.object));
        draft.set("password", value.to_vec()).expect("a change");
        draft.save().expect("a save")
    }

    /// Waits until the save of the key view of "ams" ended, and gives it.
    async fn until_saved(&self) -> SaveJob {
        let ended = |view: &KeysView| view.save.as_ref().is_some_and(|job| job.state != WriteState::Pending);
        let view =
            self.app.until("the end of the save", |view| keys(view, "ams").is_some_and(|keys| ended(&keys))).await;
        keys(&view, "ams").and_then(|keys| keys.save.clone()).expect("the save")
    }
}

/// Bad data from the cluster: the answer of a save is an other object. The view does not
/// show its values as the values of the object of the view: the result is not known, and
/// the read after it gives the reason.
#[tokio::test(flavor = "multi_thread")]
async fn the_answer_of_a_save_with_an_other_object_is_no_result() {
    let bed = Bed::open("keys-other").await;
    // "YQ==" is "a". Each request on the path of db gets the Secret api from now on.
    bed.server.set_body(DB, &secret("api", 50, "YQ=="));
    bed.app.core.send(Command::SaveKeys(bed.save(b"new")));

    let job = bed.until_saved().await;
    let WriteState::Unknown(error) = &job.state else { panic!("a result that is not known: {job:?}") };
    assert_eq!(error.to_string(), "the server answered with an other object");
    let failed = |keys: &KeysView| matches!(keys.read, ReadState::Failed(_));
    let after =
        bed.app.until("the read after the save", |view| keys(view, "ams").is_some_and(|keys| failed(&keys))).await;
    let view = keys(&after, "ams").expect("the key view");
    let ReadState::Failed(error) = &view.read else { panic!("a failed read: {view:?}") };
    assert_eq!(error.to_string(), "the server answered with an other object");
    let object = view.object.as_ref().expect("the object of the read before");
    assert_eq!((object.target.key.name.as_ref(), &*object.version), ("db", "41"), "the object of the view");
}
