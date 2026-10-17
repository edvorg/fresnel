//! The memory rule of the soak (docs/decisions.md). The buffers of the core have the small
//! limits of `app`, and each minute checks each one against its limit. The rest of the heap
//! must stay flat after the warm-up: its floor may rise by `--memory-limit` at most. A buffer
//! that fills late thus looks like no leak, and a leak needs no room left by a buffer to show.
//! The floor of a minute is the lowest heap of its window, less the buffers and the records of
//! the soak itself: a list after a fault holds its answer for some seconds, and such a peak is
//! no growth.

use serde::Serialize;

use super::account::Piece;
use super::app::{JOURNAL_LIMIT, MAX_EVENTS, METRICS_MEMORY, RING};
use super::minute::Minute;

/// The minutes of the window of a floor.
pub(super) const WINDOW: usize = 5;

/// The heap after the warm-up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Heap {
    /// The floor at the end of the warm-up, and its minute.
    pub(super) base_bytes: usize,
    pub(super) base_minute: u64,
    /// The most that a later floor rose above the base, and the minute of that floor.
    pub(super) growth_bytes: i64,
    pub(super) growth_minute: u64,
}

/// A buffer of the core that has a limit of the soak.
enum Buffer {
    Journal,
    Ring,
    Metrics,
    Events,
}

/// The buffer that a part of the memory account is.
fn buffer(name: &str) -> Option<Buffer> {
    let ends = |end: &str| name.ends_with(end);
    match name {
        "journal" => Some(Buffer::Journal),
        _ if ends(": log ring") => Some(Buffer::Ring),
        _ if ends(": metrics history") => Some(Buffer::Metrics),
        _ if ends(": store Event") => Some(Buffer::Events),
        _ => None,
    }
}

/// The floor of the minute with this index: the lowest heap of the window that ends with it,
/// with no buffer and no record of the soak.
fn floor(minutes: &[Minute], index: usize) -> Option<usize> {
    let window = minutes.get(index.saturating_sub(WINDOW - 1)..=index)?;
    let buffers = |memory: &[Piece]| -> usize {
        memory.iter().filter(|piece| buffer(&piece.name).is_some()).map(|piece| piece.bytes).sum()
    };
    window.iter().map(|minute| minute.heap_low.saturating_sub(minute.records_bytes + buffers(&minute.memory))).min()
}

/// The heap after the warm-up of `warm_up` minutes. `None` before the end of the warm-up.
/// The base is the first floor after the warm-up whose window has no minute of an outage of
/// the environment: a cluster that the harness recreated in an outage has few objects, and a
/// floor of that time would be no base for a cluster with all its objects.
pub(super) fn heap(minutes: &[Minute], warm_up: usize) -> Option<Heap> {
    let calm = |index: usize| minutes[index.saturating_sub(WINDOW - 1)..=index].iter().all(|minute| !minute.outage);
    let base_index = (warm_up.max(1) - 1..minutes.len()).find(|index| calm(*index))?;
    let base_bytes = floor(minutes, base_index)?;
    let bytes = |bytes: usize| i64::try_from(bytes).unwrap_or(i64::MAX);
    let later = (base_index + 1..minutes.len()).filter_map(|index| Some((index, floor(minutes, index)?)));
    // The first minute with the highest floor: `max_by_key` gives the last one of a sequence.
    let highest = later.rev().max_by_key(|(_, floor)| *floor);
    let (index, top) = highest.unwrap_or((base_index, base_bytes));
    let (growth_minute, base_minute) = (minutes[index].minute, minutes[base_index].minute);
    Some(Heap { base_bytes, base_minute, growth_bytes: bytes(top) - bytes(base_bytes), growth_minute })
}

/// The parts of a minute above the limit that the soak gave their buffer: the journal, the
/// ring of the log view, the history of the metrics and the store of the Events. Each one
/// with its name and a text.
pub(super) fn over_limits(memory: &[Piece]) -> Vec<(String, String)> {
    let mut over = Vec::new();
    for piece in memory {
        let name = piece.name.as_str();
        let text = match buffer(name) {
            Some(Buffer::Journal) if piece.items > JOURNAL_LIMIT => {
                format!("{} entries, the limit is {JOURNAL_LIMIT}", piece.items)
            }
            Some(Buffer::Ring) if piece.items > RING.lines || piece.bytes > RING.bytes => {
                let limits = format!("{} lines and {} bytes", RING.lines, RING.bytes);
                format!("{} lines and {} bytes, the limit is {limits}", piece.items, piece.bytes)
            }
            Some(Buffer::Metrics) if piece.bytes > METRICS_MEMORY => {
                format!("{} bytes, the limit is {METRICS_MEMORY}", piece.bytes)
            }
            Some(Buffer::Events) if piece.items > MAX_EVENTS => {
                format!("{} Events, the limit is {MAX_EVENTS}", piece.items)
            }
            _ => continue,
        };
        over.push((name.to_owned(), format!("{name}: {text}")));
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
        let growth = Heap { base_bytes: 4 * MIB, base_minute: 3, growth_bytes: MIB as i64, growth_minute: 8 };
        assert_eq!(heap(&minutes, 3), Some(growth), "a peak in each window is no growth");
        assert_eq!(heap(&minutes, 40), None, "the warm-up did not end");
        assert_eq!(heap(&minutes[..17], 3).map(|heap| heap.growth_bytes), Some(MIB as i64));
        let all = heap(&minutes, 18).expect("the last minute");
        assert_eq!((all.base_bytes, all.growth_bytes, all.growth_minute), (5 * MIB, 0, 18));

        // A recreate in an outage at the end of the warm-up: the cluster has few objects then.
        let lows = [9, 9, 2, 2, 9, 9, 9, 9, 9, 9, 9, 9];
        let mut minutes: Vec<Minute> = (1..).zip(lows).map(|(number, _)| minute(number, 40)).collect();
        minutes.iter_mut().zip(lows).for_each(|(minute, low)| minute.heap_low = low * MIB);
        minutes[2..5].iter_mut().for_each(|minute| minute.outage = true);
        let after = heap(&minutes, 4).expect("a minute with no outage in its window");
        assert_eq!((after.base_minute, after.base_bytes, after.growth_bytes), (10, 9 * MIB, 0));
        assert_eq!(heap(&minutes[..9], 4), None, "each window has a minute of the outage");
    }

    /// A ring and a journal that fill after the warm-up are no growth. A part with no limit of
    /// the soak that grows is, also while a buffer gives room back.
    #[test]
    fn a_buffer_that_fills_late_is_no_growth_and_the_rest_of_the_heap_is() {
        let piece = |name: &str, bytes| Piece { name: name.to_owned(), items: 1, bytes };
        let mut minutes: Vec<Minute> = (1..=12).map(|number| minute(number, 40)).collect();
        for (index, minute) in minutes.iter_mut().enumerate() {
            let (ring, journal) = (index * MIB / 2, index * MIB / 4);
            minute.memory = vec![piece("k3s: log ring", ring), piece("journal", journal), piece("k3s: store Pod", MIB)];
            minute.heap_low = 10 * MIB + ring + journal + MIB;
        }
        let flat = heap(&minutes, 1).expect("the heap after the warm-up");
        assert_eq!((flat.base_bytes, flat.growth_bytes), (11 * MIB, 0), "{flat:?}");

        for minute in &mut minutes[6..] {
            let ring = &mut minute.memory[0].bytes;
            (*ring, minute.heap_low) = (*ring - MIB, minute.heap_low - MIB + 3 * MIB);
            minute.memory[2].bytes += 3 * MIB;
        }
        let grown = heap(&minutes, 1).expect("the heap after the warm-up");
        assert_eq!((grown.growth_bytes, grown.growth_minute), (3 * MIB as i64, 11), "{grown:?}");
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
