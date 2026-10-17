//! The memory account of one minute: each bounded part of the core with its items and its
//! bytes (`Core::memory`), measured with the counting allocator of the program. The report
//! names the parts that grew from the end of the warm-up to the last minute.

use fresnel_core::app::Core;
use serde::Serialize;

use crate::memory::Copies;

/// One part of the core in the record of a minute.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Piece {
    pub(super) name: String,
    pub(super) items: usize,
    pub(super) bytes: usize,
}

/// The parts of the core now.
pub(super) fn take(core: &Core) -> Vec<Piece> {
    let parts = core.memory(&Copies);
    parts.into_iter().map(|part| Piece { name: part.name, items: part.items, bytes: part.bytes }).collect()
}

/// The change of one part from one minute to a later one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Change {
    pub(super) name: String,
    pub(super) items: (usize, usize),
    pub(super) bytes: (usize, usize),
}

/// The parts whose bytes grew most from `base` to `last`, the largest growth first: a part
/// of `last` only grew from nothing. At most `count` parts.
pub(super) fn grown(base: &[Piece], last: &[Piece], count: usize) -> Vec<Change> {
    let before = |name: &str| base.iter().find(|piece| piece.name == name).map_or((0, 0), |p| (p.items, p.bytes));
    let mut changes: Vec<Change> = last
        .iter()
        .map(|piece| {
            let (items, bytes) = before(&piece.name);
            Change { name: piece.name.clone(), items: (items, piece.items), bytes: (bytes, piece.bytes) }
        })
        .filter(|change| change.bytes.1 > change.bytes.0)
        .collect();
    changes.sort_by_key(|change| std::cmp::Reverse(change.bytes.1 - change.bytes.0));
    changes.truncate(count);
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(name: &str, items: usize, bytes: usize) -> Piece {
        Piece { name: name.to_owned(), items, bytes }
    }

    #[test]
    fn the_parts_that_grew_most_come_first_and_a_new_part_grew_from_nothing() {
        let base = [piece("journal", 10, 1000), piece("store Pod", 5, 5000), piece("log ring", 9, 900)];
        let last = [piece("journal", 10, 1000), piece("store Pod", 6, 6000), piece("log ring", 3, 300)];
        let last = [&last[..], &[piece("store on demand Secret", 2, 2500)]].concat();
        let change = |name: &str, items, bytes| Change { name: name.to_owned(), items, bytes };
        let wanted = [change("store on demand Secret", (0, 2), (0, 2500)), change("store Pod", (5, 6), (5000, 6000))];
        assert_eq!(grown(&base, &last, 5), wanted);
        assert_eq!(grown(&base, &last, 1).len(), 1);
    }
}
