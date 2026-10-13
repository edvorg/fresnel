//! The limit for the lists that one cluster runs at the same time (plan section 8.11).
//!
//! Many lists at the same moment fill the request queue of an API server, and one slow
//! list then holds each other one. A list takes a slot for all its pages. When more lists
//! wait than slots are free, the first list of a store goes before each later one, and the
//! kind with the highest priority gets the next slot: the small kinds that each view needs
//! are then ready before the Events.
//!
//! A later list of a store waits in the sequence of its request. With the priority of its
//! kind, eight kinds whose lists fail at their time limit would hold the four slots for
//! all time, and no kind after them would get its first list.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

use crate::kinds::Kind;

/// The place of a list among the lists that wait. The sequence of the variants and of
/// the kinds is the sequence of the slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Turn {
    /// The first list of a store. The sequence of the kinds is its priority.
    First(Kind),
    /// Each later list of a store: after a failure, after HTTP 410 or after a refusal.
    Again,
}

struct Inner {
    free: usize,
    /// The lists that wait. The first entry gets the next slot. The number is the
    /// sequence of the requests: it makes each entry unique, and it is the sequence of
    /// the later lists.
    waiting: BTreeSet<(Turn, u64)>,
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
    pub(crate) async fn take(self: &Arc<Self>, turn: Turn) -> Slot {
        let ticket = Ticket::new(self, turn);
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
    entry: (Turn, u64),
}

impl<'a> Ticket<'a> {
    fn new(gate: &'a ListGate, turn: Turn) -> Self {
        let mut inner = gate.inner();
        let entry = (turn, inner.next_number);
        inner.next_number += 1;
        inner.waiting.insert(entry);
        Self { gate, entry }
    }

    /// Takes a slot if one is free and no list with an earlier place waits.
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

    const FIRST_POD: Turn = Turn::First(Kind::Pod);

    /// Waits for a slot, writes `name` into `order` and holds the slot for one more step.
    async fn list(gate: Arc<ListGate>, turn: Turn, name: &'static str, order: Arc<Mutex<Vec<&'static str>>>) {
        let slot = gate.take(turn).await;
        order.lock().expect("a lock").push(name);
        yield_now().await;
        drop(slot);
    }

    #[tokio::test]
    async fn the_first_lists_get_a_slot_in_the_sequence_of_their_kinds() {
        let gate = ListGate::new(2);
        let (first, second) = (gate.take(FIRST_POD).await, gate.take(FIRST_POD).await);
        let order = Arc::new(Mutex::new(Vec::new()));
        // The Events ask first. Each kind before them in the list gets its slot before them.
        let asks =
            [(Kind::Event, "Event"), (Kind::Pod, "Pod"), (Kind::Secret, "Secret"), (Kind::Namespace, "Namespace")];
        let lists =
            asks.map(|(kind, name)| Box::pin(list(Arc::clone(&gate), Turn::First(kind), name, Arc::clone(&order))));
        let mut lists = futures_util::future::join_all(lists);
        assert!((&mut lists).now_or_never().is_none(), "no slot is free");
        assert_eq!(*order.lock().expect("a lock"), [""; 0]);

        drop(first);
        assert!((&mut lists).now_or_never().is_none());
        assert_eq!(*order.lock().expect("a lock"), ["Namespace"], "one free slot is one list");
        drop(second);
        lists.await;
        assert_eq!(*order.lock().expect("a lock"), ["Namespace", "Pod", "Secret", "Event"]);
    }

    /// A kind whose list fails each time asks again and again. With the priority of its
    /// kind for each of these lists, the kinds after it would never get a slot.
    #[tokio::test]
    async fn a_later_list_waits_behind_each_first_list_and_in_the_sequence_of_its_request() {
        let gate = ListGate::new(1);
        let held = gate.take(FIRST_POD).await;
        let order = Arc::new(Mutex::new(Vec::new()));
        let asks = [
            (Turn::Again, "the second list of the namespaces"),
            (Turn::Again, "the second list of the nodes"),
            (Turn::First(Kind::Event), "the first list of the events"),
            (Turn::First(Kind::Secret), "the first list of the secrets"),
        ];
        let lists = asks.map(|(turn, name)| Box::pin(list(Arc::clone(&gate), turn, name, Arc::clone(&order))));
        let mut lists = futures_util::future::join_all(lists);
        assert!((&mut lists).now_or_never().is_none(), "no slot is free");
        drop(held);
        lists.await;
        let expected = [
            "the first list of the secrets",
            "the first list of the events",
            "the second list of the namespaces",
            "the second list of the nodes",
        ];
        assert_eq!(*order.lock().expect("a lock"), expected);
    }

    #[tokio::test]
    async fn a_wait_that_its_caller_dropped_holds_no_slot_and_no_place() {
        let gate = ListGate::new(1);
        let held = gate.take(FIRST_POD).await;
        let mut dropped = Box::pin(gate.take(Turn::First(Kind::Namespace)));
        assert!((&mut dropped).now_or_never().is_none());
        let mut patient = Box::pin(gate.take(Turn::Again));
        assert!((&mut patient).now_or_never().is_none());

        drop(dropped);
        drop(held);
        let slot = patient.now_or_never().expect("the list with the earlier place is gone");
        assert!(gate.take(FIRST_POD).now_or_never().is_none(), "the one slot is in use");
        drop(slot);
        assert!(gate.take(FIRST_POD).now_or_never().is_some());
        assert_eq!(ListGate::new(0).inner().free, 1, "a gate has one slot at least");
    }
}
