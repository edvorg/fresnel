//! The key view in the core with fast input and bad data: a second click on Save, and the
//! answer of a save with an other object.

use std::sync::Arc;
use std::time::Duration;

use fresnel_core::app::{Command, CoreSettings, Health, KeysView, ReadState, SaveJob};
use fresnel_core::keys::{Draft, KeyObject, Save, WriteState};
use fresnel_testkit::{FakeApiServer, FaultProxy, ProxyMode};
use http::Method;
use tokio::time::timeout;

use super::keys::{DB, keys, password, requests, secret, target};
use super::{App, home, write_kubeconfig};
use crate::common::LIMIT;

/// Each byte takes this time through the proxy, so the answer of a PATCH is on its way for
/// so long after the server has the request.
const ON_ITS_WAY: Duration = Duration::from_millis(500);

/// A core with the key view of the Secret `db` of the cluster "ams" open, behind a proxy.
struct Bed {
    server: FakeApiServer,
    proxy: FaultProxy,
    app: App,
    /// The read of the view.
    object: Arc<KeyObject>,
}

impl Bed {
    async fn open(label: &str) -> Self {
        let server = FakeApiServer::start().await.expect("a server");
        let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
        server.set_body(DB, &secret("db", 41, "YW1zLTE="));
        let home = home(label);
        write_kubeconfig(&home, "ams", &[("ams", proxy.addr(), &server, false)]);
        let app = App::started(&home, CoreSettings::default());
        app.until_health("ams", Health::Live).await;
        app.core.send(Command::ShowKeys(Some(target("ams"))));
        let open = app.until("the Secret", |view| password(view, "ams").is_some()).await;
        let object = keys(&open, "ams").and_then(|keys| keys.object.clone()).expect("the object");
        Self { server, proxy, app, object }
    }

    /// The save of a new value of `password` on the read of the view.
    fn save(&self, value: &[u8]) -> Save {
        let mut draft = Draft::new(Arc::clone(&self.object));
        draft.set("password", value.to_vec()).expect("a change");
        draft.save().expect("a save")
    }

    /// Waits until the server has the head of `count` requests of this method and path.
    async fn until_seen(&self, method: &Method, path: &str, count: usize) {
        let wait = async {
            loop {
                let before = self.server.counters().requests;
                if requests(&self.server, method, path) >= count {
                    return;
                }
                self.server.wait_for(|counters| counters.requests > before).await;
            }
        };
        timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("in time: {count} requests {method} {path}"));
    }

    /// Waits until the save of the key view of "ams" ended, and gives it.
    async fn until_saved(&self) -> SaveJob {
        let ended = |view: &KeysView| view.save.as_ref().is_some_and(|job| job.state != WriteState::Pending);
        let view =
            self.app.until("the end of the save", |view| keys(view, "ams").is_some_and(|keys| ended(&keys))).await;
        keys(&view, "ams").and_then(|keys| keys.save.clone()).expect("the save")
    }
}

/// Fast input: a second click on Save while the save is on its way. The same save again
/// would go with the old version, and the server would refuse it as a conflict with the
/// first one. So it does not go, and the view shows the end of the first one.
#[tokio::test(flavor = "multi_thread")]
async fn the_same_save_again_while_it_is_on_its_way_sends_no_second_patch() {
    let bed = Bed::open("keys-twice").await;
    bed.proxy.set(ProxyMode::Latency(ON_ITS_WAY));
    bed.app.core.send(Command::SaveKeys(bed.save(b"new")));
    bed.until_seen(&Method::PATCH, DB, 1).await;
    bed.app.core.send(Command::SaveKeys(bed.save(b"new")));

    let job = bed.until_saved().await;
    assert!(matches!(job.state, WriteState::Done(_)), "{job:?}");
    assert_eq!((job.number, requests(&bed.server, &Method::PATCH, DB)), (1, 1), "the first save, one PATCH");
    bed.proxy.set(ProxyMode::Pass);
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
