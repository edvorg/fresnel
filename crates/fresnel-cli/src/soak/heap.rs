//! The memory rule of the soak (docs/decisions.md). The buffers of the core have the small
//! limits of `app`, so each one is full early; after the warm-up, the heap must stay flat:
//! its floor may rise by the margin of `--memory-limit` at most. The floor of a minute is
//! the lowest heap of its window: a list after a fault holds its answer for some seconds, and
//! such a peak is no growth. The records of the soak itself are no part of it. A part of the
//! account above its own limit fails at once.

use serde::Serialize;

use super::account::Piece;
use super::app::{JOURNAL_LIMIT, MAX_EVENTS, METRICS_MEMORY, RING};
use super::minute::Minute;

/// The minutes of the window of a floor.
pub(super) const WINDOW: usize = 5;

/// The heap after the warm-up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Heap {
    /// The floor at the end of the warm-up.
    pub(super) base_bytes: usize,
    /// The most that a later floor rose above the base, and the minute of that floor.
    pub(super) growth_bytes: i64,
    pub(super) growth_minute: u64,
}

/// The floor of the minute with this index: the lowest heap of the window that ends with it.
fn floor(minutes: &[Minute], index: usize) -> Option<usize> {
    let window = minutes.get(index.saturating_sub(WINDOW - 1)..=index)?;
    window.iter().map(|minute| minute.heap_low.saturating_sub(minute.records_bytes)).min()
}

/// The heap after the warm-up of `warm_up` minutes. `None` before the end of the warm-up.
pub(super) fn heap(minutes: &[Minute], warm_up: usize) -> Option<Heap> {
    let base_index = warm_up.max(1) - 1;
    let base_bytes = floor(minutes, base_index)?;
    let bytes = |bytes: usize| i64::try_from(bytes).unwrap_or(i64::MAX);
    let later = (base_index + 1..minutes.len()).filter_map(|index| Some((index, floor(minutes, index)?)));
    // The first minute with the highest floor: `max_by_key` gives the last one of a sequence.
    let highest = later.rev().max_by_key(|(_, floor)| *floor);
    let (index, top) = highest.unwrap_or((base_index, base_bytes));
    let growth_minute = minutes.get(index).map_or(0, |minute| minute.minute);
    Some(Heap { base_bytes, growth_bytes: bytes(top) - bytes(base_bytes), growth_minute })
}

/// The parts of a minute above the limit that the soak gave their buffer: the journal, the
/// ring of the log view, the history of the metrics and the store of the Events. Each one
/// with its name and a text.
pub(super) fn over_limits(memory: &[Piece]) -> Vec<(String, String)> {
    let mut over = Vec::new();
    for piece in memory {
        let name = piece.name.as_str();
        let limit = |what: &str| (name.to_owned(), format!("{name}: {what}"));
        if name == "journal" && piece.items > JOURNAL_LIMIT {
            over.push(limit(&format!("{} entries, the limit is {JOURNAL_LIMIT}", piece.items)));
        } else if name.ends_with(": log ring") && (piece.items > RING.lines || piece.bytes > RING.bytes) {
            let limits = format!("{} lines and {} bytes", RING.lines, RING.bytes);
            over.push(limit(&format!("{} lines and {} bytes, the limit is {limits}", piece.items, piece.bytes)));
        } else if name.ends_with(": metrics history") && piece.bytes > METRICS_MEMORY {
            over.push(limit(&format!("{} bytes, the limit is {METRICS_MEMORY}", piece.bytes)));
        } else if name.ends_with(": store Event") && piece.items > MAX_EVENTS {
            over.push(limit(&format!("{} Events, the limit is {MAX_EVENTS}", piece.items)));
        }
    }
    over
}

#[cfg(test)]
mod tests {
    use super::super::report::tests::minute;
    use super::*;

    const MIB: usize = 1024 * 1024;

    #[test]
    fn the_floor_of_a_window_takes_no_peak_and_a_floor_that_rises_is_growth() {
        let lows = [9, 8, 4, 30, 30, 30, 30, 5, 30, 30, 30, 30, 5, 30, 30, 30, 30, 5];
        let mut minutes: Vec<Minute> = (1..).zip(lows).map(|(number, _)| minute(number, 40)).collect();
        minutes.iter_mut().zip(lows).for_each(|(minute, low)| minute.heap_low = low * MIB);
        let growth = Heap { base_bytes: 4 * MIB, growth_bytes: MIB as i64, growth_minute: 8 };
        assert_eq!(heap(&minutes, 3), Some(growth), "a peak in each window is no growth");
        assert_eq!(heap(&minutes, 40), None, "the warm-up did not end");
        assert_eq!(heap(&minutes[..17], 3).map(|heap| heap.growth_bytes), Some(MIB as i64));
        let all = heap(&minutes, 18).expect("the last minute");
        assert_eq!((all.base_bytes, all.growth_bytes, all.growth_minute), (5 * MIB, 0, 18));
    }

    #[test]
    fn a_part_above_the_limit_of_its_buffer_is_named() {
        let piece = |name: &str, items, bytes| Piece { name: name.to_owned(), items, bytes };
        let inside = [piece("journal", 1000, 0), piece("k3s: log ring", 2000, MIB), piece("k3s: store Event", 500, 0)];
        assert_eq!(over_limits(&inside), Vec::<(String, String)>::new());
        let over = [
            piece("journal", 1001, 0),
            piece("k3s: log ring", 10, MIB + 1),
            piece("k3s: metrics history", 9, METRICS_MEMORY + 1),
            piece("k3s: store Event", 501, 0),
            piece("k3s: store Pod", 9000, 0),
        ];
        let found = over_limits(&over);
        assert_eq!(found.len(), 4, "{found:?}");
        let named = |((name, text), piece): (&(String, String), &Piece)| *name == piece.name && text.starts_with(name);
        assert!(found.iter().zip(&over).all(named));
    }
}
