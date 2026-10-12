//! The notice that a state of a cluster changed.
//!
//! The UI reads each state as a copy. This notice tells it when to read again. A caller
//! never gets a lock or a guard that could stop a task of the cluster.

use std::sync::Arc;

use tokio::sync::watch;

/// Counts the state changes of one cluster.
#[derive(Clone, Debug)]
pub(crate) struct Changed(Arc<watch::Sender<u64>>);

impl Changed {
    pub(crate) fn new() -> Self {
        Self(Arc::new(watch::Sender::new(0)))
    }

    pub(crate) fn notify(&self) {
        self.0.send_modify(|count| *count = count.wrapping_add(1));
    }

    pub(crate) fn watch(&self) -> Changes {
        Changes(self.0.subscribe())
    }
}

/// Waits for state changes of one cluster. Two changes before a wait count as one.
#[derive(Debug)]
pub struct Changes(watch::Receiver<u64>);

impl Changes {
    /// Waits until a state of the cluster changed after the last call. It waits with no end
    /// after the cluster dropped.
    pub async fn next(&mut self) {
        if self.0.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
        self.0.borrow_and_update();
    }
}
