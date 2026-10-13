//! The limit for the lists that one cluster runs at the same time (plan section 8.11).
//!
//! Many lists at the same moment fill the request queue of an API server, and one slow
//! list then holds each other one. A list takes a slot for all its pages. When more lists
//! wait than slots are free, the kind with the highest priority gets the next slot: the
//! small kinds that each view needs are then ready before the Events.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

use crate::kinds::Kind;

struct Inner {
    free: usize,
    /// The lists that wait. The first entry gets the next slot: the sequence of the kinds
    /// is their priority, and the number makes each entry of one kind unique.
    waiting: BTreeSet<(Kind, u64)>,
    next_number: u64,
}

/// The slots of one cluster.
pub(crate) struct ListGate {
    inner: Mutex<Inner>,
    /// Wakes each list that waits, after a slot is free or the first entry changed.
    moved: Notify,
}

impl ListGate {
    /// A gate with this count of slots, and one slot at least.
    pub(crate) fn new(lists: usize) -> Arc<Self> {
        let inner = Inner { free: lists.max(1), waiting: BTreeSet::new(), next_number: 0 };
        Arc::new(Self { inner: Mutex::new(inner), moved: Notify::new() })
    }

    /// Waits for a slot. The caller can drop the wait at each moment.
    pub(crate) async fn take(self: &Arc<Self>, kind: Kind) -> Slot {
        let ticket = Ticket::new(self, kind);
        loop {
            let moved = self.moved.notified();
            tokio::pin!(moved);
            // The registration comes before the check, so that no free slot is lost.
            moved.as_mut().enable();
            if ticket.try_take() {
                return Slot { gate: Arc::clone(self) };
            }
            moved.await;
        }
    }

    /// Each use holds the lock for one small change.
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One list in the set of the lists that wait. Its drop removes it, also when the caller
/// dropped the wait.
struct Ticket<'a> {
    gate: &'a ListGate,
    entry: (Kind, u64),
}

impl<'a> Ticket<'a> {
    fn new(gate: &'a ListGate, kind: Kind) -> Self {
        let mut inner = gate.inner();
        let entry = (kind, inner.next_number);
        inner.next_number += 1;
        inner.waiting.insert(entry);
        Self { gate, entry }
    }

    /// Takes a slot if one is free and no list with a higher priority waits.
    fn try_take(&self) -> bool {
        let mut inner = self.gate.inner();
        let first = inner.waiting.first() == Some(&self.entry);
        if !first || inner.free == 0 {
            return false;
        }
        inner.free -= 1;
        inner.waiting.remove(&self.entry);
        true
    }
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        self.gate.inner().waiting.remove(&self.entry);
        // The next entry is the first one now, and a second slot can be free.
        self.gate.moved.notify_waiters();
    }
}

/// One slot. The list holds it to its end, and the drop frees it.
pub(crate) struct Slot {
    gate: Arc<ListGate>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.gate.inner().free += 1;
        self.gate.moved.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use futures_util::FutureExt as _;
    use tokio::task::yield_now;

    use super::*;

    #[tokio::test]
    async fn the_lists_that_wait_get_a_slot_in_the_sequence_of_their_kinds() {
        let gate = ListGate::new(2);
        let (first, second) = (gate.take(Kind::Event).await, gate.take(Kind::Secret).await);
        let order = Arc::new(Mutex::new(Vec::new()));
        let wait = |kind: Kind| {
            let (gate, order) = (Arc::clone(&gate), Arc::clone(&order));
            async move {
                let slot = gate.take(kind).await;
                order.lock().expect("a lock").push(kind);
                // The list runs until the test frees each slot.
                yield_now().await;
                drop(slot);
            }
        };
        // The Events ask first. Each kind before them in the list gets its slot before them.
        let lists = [Kind::Event, Kind::Pod, Kind::Secret, Kind::Namespace, Kind::Pod].map(|kind| Box::pin(wait(kind)));
        let mut lists = futures_util::future::join_all(lists);
        assert!((&mut lists).now_or_never().is_none(), "no slot is free");
        assert_eq!(*order.lock().expect("a lock"), []);

        drop(first);
        assert!((&mut lists).now_or_never().is_none());
        assert_eq!(*order.lock().expect("a lock"), [Kind::Namespace], "one free slot is one list");
        drop(second);
        lists.await;
        let expected = [Kind::Namespace, Kind::Pod, Kind::Pod, Kind::Secret, Kind::Event];
        assert_eq!(*order.lock().expect("a lock"), expected);
    }

    #[tokio::test]
    async fn a_wait_that_its_caller_dropped_holds_no_slot_and_no_place() {
        let gate = ListGate::new(1);
        let held = gate.take(Kind::Pod).await;
        let mut dropped = Box::pin(gate.take(Kind::Namespace));
        assert!((&mut dropped).now_or_never().is_none());
        let mut patient = Box::pin(gate.take(Kind::Event));
        assert!((&mut patient).now_or_never().is_none());

        drop(dropped);
        drop(held);
        let slot = patient.now_or_never().expect("the list with the highest priority is gone");
        assert!(gate.take(Kind::Pod).now_or_never().is_none(), "the one slot is in use");
        drop(slot);
        assert!(gate.take(Kind::Pod).now_or_never().is_some());
        assert_eq!(ListGate::new(0).inner().free, 1, "a gate has one slot at least");
    }
}
