//! The parts of the tests of the sources and the views: a script that answers each open
//! of a stream, the stores of the pods, and the lines of a server.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::sync::Arc;
use std::time::SystemTime;

use bytes::Bytes;
use k8s_openapi::api::core::v1::Pod;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::request::Origin;
use super::source::{Chunks, Opener};
use crate::error::CoreError;
use crate::kinds::Object;
use crate::published::{Disposal, Notice, Publisher, Reader};
use crate::store::{Items, ObjectKey, Snapshot, StoreState};

/// One open of a stream that waits for the answer of the test.
pub(in crate::logs) struct Call {
    pub(in crate::logs) pod: ObjectKey,
    pub(in crate::logs) container: String,
    pub(in crate::logs) previous: bool,
    pub(in crate::logs) origin: Origin,
    answer: oneshot::Sender<Result<Scripted, CoreError>>,
}

impl Call {
    /// Opens the stream. The test sends its data and drops the sender for its end.
    pub(in crate::logs) fn open(self) -> mpsc::UnboundedSender<Result<Bytes, CoreError>> {
        let (data, received) = mpsc::unbounded_channel();
        self.answer.send(Ok(Scripted { received })).map_err(drop).expect("the source waits");
        data
    }

    pub(in crate::logs) fn refuse(self, error: CoreError) {
        self.answer.send(Err(error)).map_err(drop).expect("the source waits");
    }
}

/// The opener of a test: each open is a [`Call`].
#[derive(Clone)]
pub(in crate::logs) struct Script {
    calls: mpsc::UnboundedSender<Call>,
}

impl Script {
    pub(in crate::logs) fn new() -> (Self, mpsc::UnboundedReceiver<Call>) {
        let (calls, received) = mpsc::unbounded_channel();
        (Self { calls }, received)
    }
}

pub(in crate::logs) struct Scripted {
    received: mpsc::UnboundedReceiver<Result<Bytes, CoreError>>,
}

impl Opener for Script {
    type Stream = Scripted;

    async fn open(
        &self,
        pod: &ObjectKey,
        container: &str,
        previous: bool,
        origin: Origin,
    ) -> Result<Scripted, CoreError> {
        let (answer, answered) = oneshot::channel();
        let call = Call { pod: pod.clone(), container: container.to_owned(), previous, origin, answer };
        self.calls.send(call).map_err(drop).expect("the test reads each call");
        answered.await.expect("the test answers each call")
    }
}

impl Chunks for Scripted {
    async fn next(&mut self) -> Option<Result<Bytes, CoreError>> {
        self.received.recv().await
    }
}

/// A pod `web-1` with the containers `app` and `side`. `app` has this state and these
/// other fields of its status.
pub(in crate::logs) fn pod(name: &str, uid: &str, app: Value, change: impl FnOnce(&mut Value)) -> Pod {
    let mut pod = json!({
        "metadata": {"name": name, "namespace": "shop", "uid": uid},
        "spec": {"containers": [{"name": "app"}, {"name": "side"}]},
        "status": {"phase": "Running", "containerStatuses": [
            {"name": "app", "image": "a", "imageID": "", "ready": true, "restartCount": 0, "state": app},
        ]},
    });
    change(&mut pod);
    serde_json::from_value(pod).expect("a pod")
}

/// A pod whose container `app` runs as the instance `id`.
pub(in crate::logs) fn running(id: &str) -> Pod {
    pod("web-1", "u1", json!({"running": {}}), |pod| pod["status"]["containerStatuses"][0]["containerID"] = json!(id))
}

/// A live store of a test with a complete list.
pub(in crate::logs) struct Store<T> {
    publisher: Publisher<Snapshot<T>>,
    generation: u64,
}

impl<T: Object> Store<T> {
    pub(in crate::logs) fn new(disposal: &Disposal, objects: Vec<T>) -> Self {
        let first = Snapshot::empty(StoreState::Live, SystemTime::UNIX_EPOCH);
        let mut store = Self { publisher: Publisher::new(first, disposal.bin(), Notice::new()), generation: 0 };
        store.set(objects);
        store
    }

    pub(in crate::logs) fn reader(&self) -> Reader<Snapshot<T>> {
        self.publisher.reader()
    }

    /// A complete list with these objects.
    pub(in crate::logs) fn set(&mut self, objects: Vec<T>) {
        self.generation += 1;
        let mut snapshot = Snapshot::empty(StoreState::Live, SystemTime::UNIX_EPOCH);
        (snapshot.generation, snapshot.listed) = (self.generation, true);
        snapshot.items = objects.into_iter().map(Arc::new).collect::<Items<T>>();
        self.publisher.publish(snapshot);
    }
}

/// A line of the log of the server at `seconds` since 1970, with its text.
pub(in crate::logs) fn data(lines: &[(f64, &str)]) -> Result<Bytes, CoreError> {
    let text: String = lines
        .iter()
        .map(|(seconds, text)| {
            let nanos = (seconds * 1e9).round() as i64;
            let time = k8s_openapi::jiff::Timestamp::from_nanosecond(i128::from(nanos)).expect("a time");
            format!("{time:.9} {text}\n")
        })
        .collect();
    Ok(Bytes::from(text))
}
