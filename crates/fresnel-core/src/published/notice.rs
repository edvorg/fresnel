//! The notice that one of many published states has a new value.
//!
//! A cluster has one store for each kind. A reader of all of them, for example the task
//! that asks the UI for a repaint, waits for this one notice and not for each store.

use std::sync::Arc;

use tokio::sync::watch;

/// Counts the publishes of a group of states. A clone counts for the same group.
#[derive(Clone, Debug)]
pub struct Notice {
    publishes: Arc<watch::Sender<u64>>,
}

impl Default for Notice {
    fn default() -> Self {
        Self::new()
    }
}

impl Notice {
    /// A notice for a new group of states.
    pub fn new() -> Self {
        Self { publishes: Arc::new(watch::Sender::new(0)) }
    }

    pub(super) fn tell(&self) {
        self.publishes.send_modify(|count| *count = count.wrapping_add(1));
    }

    /// The read side: it hears each publish after this call.
    pub fn changes(&self) -> Changes {
        Changes { publishes: self.publishes.subscribe() }
    }
}

/// Waits for the publishes of a group of states. Two publishes before a wait count as one.
#[derive(Debug)]
pub struct Changes {
    publishes: watch::Receiver<u64>,
}

impl Changes {
    /// Waits until a state of the group has a new value after the last call. It waits
    /// with no end after each publisher of the group dropped.
    pub async fn next(&mut self) {
        if self.publishes.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
