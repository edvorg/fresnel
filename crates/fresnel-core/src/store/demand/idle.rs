//! The holders of each store on demand, and the time when a store with no holder ends.

use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

use crate::kinds::Kind;

/// The stores on demand of one cluster with their holders. `R` is a running store.
pub(super) struct Entries<R> {
    stores: HashMap<Kind, Entry<R>>,
}

pub(super) struct Entry<R> {
    pub(super) running: R,
    holders: usize,
    /// The time when the last holder went. `None` while a holder lives.
    idle_since: Option<Instant>,
}

impl<R> Default for Entries<R> {
    fn default() -> Self {
        Self { stores: HashMap::new() }
    }
}

impl<R> Entries<R> {
    /// The store of a kind with one more holder. `start` gives the store when the kind has
    /// none.
    pub(super) fn hold_or_start(&mut self, kind: Kind, start: impl FnOnce() -> R) -> &R {
        let entry = self.stores.entry(kind).or_insert_with(|| Entry { running: start(), holders: 0, idle_since: None });
        entry.holders += 1;
        entry.idle_since = None;
        &entry.running
    }

    /// One more holder of a store that runs.
    pub(super) fn hold(&mut self, kind: Kind) {
        if let Some(entry) = self.stores.get_mut(&kind) {
            entry.holders += 1;
            entry.idle_since = None;
        }
    }

    /// Counts a holder that went. True when it was the last one: the time of the store
    /// starts now.
    pub(super) fn release(&mut self, kind: Kind) -> bool {
        let Some(entry) = self.stores.get_mut(&kind) else {
            return false;
        };
        entry.holders = entry.holders.saturating_sub(1);
        let last = entry.holders == 0;
        if last {
            entry.idle_since = Some(Instant::now());
        }
        last
    }

    /// Takes each store whose time is over out of the map. Gives them, and the end of the
    /// next time that runs.
    pub(super) fn take_idle(&mut self, now: Instant, linger: Duration) -> (Vec<R>, Option<Instant>) {
        let over = |entry: &Entry<R>| entry.idle_since.is_some_and(|since| since + linger <= now);
        let kinds: Vec<Kind> = self.stores.iter().filter(|(_, entry)| over(entry)).map(|(kind, _)| *kind).collect();
        let stopped = kinds.iter().filter_map(|kind| self.stores.remove(kind)).map(|entry| entry.running).collect();
        let next = self.stores.values().filter_map(|entry| entry.idle_since).min().map(|since| since + linger);
        (stopped, next)
    }

    /// Takes each store out of the map.
    pub(super) fn take_all(&mut self) -> Vec<R> {
        self.stores.drain().map(|(_, entry)| entry.running).collect()
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::advance;

    use super::*;

    const LINGER: Duration = Duration::from_secs(60);

    #[tokio::test(start_paused = true)]
    async fn a_store_ends_its_time_after_its_last_holder_went_and_a_new_holder_keeps_it() {
        let mut entries = Entries::default();
        let mut starts = 0;
        let mut start = |name: &'static str| {
            starts += 1;
            name
        };
        assert_eq!(*entries.hold_or_start(Kind::Lease, || start("leases")), "leases");
        assert_eq!(*entries.hold_or_start(Kind::Lease, || start("second")), "leases", "one store for each kind");
        assert_eq!(entries.take_idle(Instant::now(), LINGER), (vec![], None), "two holders");

        assert!(!entries.release(Kind::Lease), "one holder stays");
        assert!(entries.release(Kind::Lease), "the last one went");
        let went = Instant::now();
        advance(LINGER - Duration::from_millis(1)).await;
        assert_eq!(entries.take_idle(Instant::now(), LINGER), (vec![], Some(went + LINGER)));

        // A view that opens again in the time takes the same store.
        entries.hold(Kind::Lease);
        advance(LINGER).await;
        assert_eq!(entries.take_idle(Instant::now(), LINGER), (vec![], None), "a holder lives");
        assert!(entries.release(Kind::Lease));
        advance(LINGER).await;
        assert_eq!(entries.take_idle(Instant::now(), LINGER), (vec!["leases"], None), "exactly at the end");
        assert_eq!(starts, 1);
        assert!(!entries.release(Kind::Lease), "a holder of a store that ended changes nothing");
    }

    #[tokio::test(start_paused = true)]
    async fn the_next_end_is_the_earliest_one_and_a_stop_takes_each_store() {
        let mut entries = Entries::default();
        entries.hold_or_start(Kind::Lease, || "leases");
        entries.hold_or_start(Kind::Endpoints, || "endpoints");
        entries.release(Kind::Endpoints);
        let first = Instant::now();
        let later = Duration::from_secs(10);
        advance(later).await;
        entries.release(Kind::Lease);
        assert_eq!(entries.take_idle(Instant::now(), LINGER), (vec![], Some(first + LINGER)));
        advance(LINGER - later).await;
        assert_eq!(entries.take_idle(Instant::now(), LINGER), (vec!["endpoints"], Some(first + later + LINGER)));
        assert_eq!(entries.take_all(), ["leases"]);
        assert_eq!(entries.take_idle(Instant::now(), LINGER), (vec![], None));
    }
}
