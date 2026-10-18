//! The client of a forward in a fault test, as in the soak: a fetch through the local port
//! again and again, and a look at the state of the forward each 50 ms.
//!
//! A run of failed fetches in which each look saw "active" is a gap with no visible state
//! (design rule 4, plan section 12.3): the soak fails for one of 20 s. A state that shows for
//! a moment and goes back to "active" while the fetches still fail hides the rest of the
//! run, so the gap counts from the last look that saw it.

use std::cell::Cell;
use std::time::Duration;

use tokio::time::{Instant, sleep};

/// The time between two looks at the state.
const LOOK: Duration = Duration::from_millis(50);
/// The pause after each fetch.
const PAUSE: Duration = Duration::from_secs(1);

/// Fetches with `fetch` and looks with `active` until `faults` ends. Gives the longest time
/// in a run of failed fetches in which each look saw "active": from the start of the run, or
/// from a later look that saw an other state, to the end of a failed fetch. Also gives the
/// count of the fetches that failed.
pub(crate) async fn longest_hidden_failure<Fetched: Future<Output = bool>>(
    fetch: impl Fn() -> Fetched,
    active: impl Fn() -> bool,
    faults: impl Future<Output = ()>,
) -> (Duration, usize) {
    // The time of the last look that saw an other state than "active".
    let (done, shown) = (Cell::new(false), Cell::new(None));
    let (longest, failed) = (Cell::new(Duration::ZERO), Cell::new(0));
    let looks = async {
        while !done.get() {
            if !active() {
                shown.set(Some(Instant::now()));
            }
            sleep(LOOK).await;
        }
    };
    let fetches = async {
        let mut run = None;
        while !done.get() {
            let started = Instant::now();
            if fetch().await {
                run = None;
            } else {
                failed.set(failed.get() + 1);
                let start = *run.get_or_insert(started);
                let hidden_since = shown.get().map_or(start, |at: Instant| at.max(start));
                longest.set(longest.get().max(hidden_since.elapsed()));
            }
            sleep(PAUSE).await;
        }
    };
    let faults = async {
        faults.await;
        done.set(true);
    };
    tokio::join!(looks, fetches, faults);
    (longest.get(), failed.get())
}
