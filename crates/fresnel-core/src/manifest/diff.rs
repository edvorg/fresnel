//! The line diff of two texts (plan section 12.9): the diff before the save of an editor, and
//! the two sides of a merge after a conflict.
//!
//! The diff compares whole lines with their line ends, so a line feed that the user adds or
//! removes at the end is a change. Myers' algorithm finds the fewest changed lines. Its time
//! and memory grow with the square of that count, so past `MOST_CHANGES` the lines between
//! the first and the last change are one change: a text that changes in a thousand lines
//! is a new text, and a merge of it with an other change is a conflict.

use std::ops::Range;

/// The most changed lines that the diff tells apart. The fronts of 1,000 steps hold half a
/// million numbers, and a manifest edit changes some lines.
const MOST_CHANGES: usize = 1_000;

/// One change: the lines `old` of the first text become the lines `new` of the second.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// The lines of a text, each with its line feed.
pub fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// The changes from `old` to `new`, in their sequence. Equal texts give none. Two changes
/// have at least one equal line between them.
pub fn hunks(old: &[&str], new: &[&str]) -> Vec<Hunk> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let rest = old.len().min(new.len()) - prefix;
    let suffix = old.iter().rev().zip(new.iter().rev()).take(rest).take_while(|(a, b)| a == b).count();
    let (a, b) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);
    let runs = myers(a, b).unwrap_or_default();
    let mut hunks = Vec::new();
    let (mut x, mut y) = (0, 0);
    for (run_x, run_y, length) in runs.into_iter().chain([(a.len(), b.len(), 0)]) {
        if run_x > x || run_y > y {
            hunks.push(Hunk { old: prefix + x..prefix + run_x, new: prefix + y..prefix + run_y });
        }
        (x, y) = (run_x + length, run_y + length);
    }
    hunks
}

/// The furthest x on each diagonal k = x - y after one count of changes: entry i is the
/// diagonal `2 * i - d`. `None`: no path in the grid reaches the diagonal.
type Front = Vec<Option<usize>>;

/// The runs of equal lines of the shortest edit, as (x, y, length) in their sequence, or
/// `None` past `MOST_CHANGES`.
fn myers(a: &[&str], b: &[&str]) -> Option<Vec<(usize, usize, usize)>> {
    let size = (a.len(), b.len());
    let mut fronts: Vec<Front> = Vec::new();
    for d in 0..=MOST_CHANGES.min(a.len() + b.len()) {
        let mut front = Front::with_capacity(d + 1);
        let mut end = None;
        for i in 0..=d {
            let k = diagonal(i, d);
            let start = match fronts.last() {
                Some(before) => step(before, d, k, size).map(|(x, _)| x),
                None => Some(0),
            };
            let point = start.and_then(|x| Some((x, x.checked_add_signed(-k)?)));
            let x = point.map(|(x, y)| x + snake(a, b, x, y));
            if x.is_some_and(|x| x == a.len() && x.checked_add_signed(-k) == Some(b.len())) {
                end = Some(k);
            }
            front.push(x);
        }
        fronts.push(front);
        if let Some(k) = end {
            return Some(path(&fronts, k, size));
        }
    }
    None
}

/// The diagonal of entry `i` of the front of `d` changes.
fn diagonal(i: usize, d: usize) -> isize {
    (2 * i) as isize - d as isize
}

/// The furthest x on diagonal `k` in the front of `d` changes.
fn reach(front: &Front, d: usize, k: isize) -> Option<usize> {
    let index = usize::try_from(k + d as isize).ok().filter(|index| index % 2 == 0)? / 2;
    front.get(index).copied().flatten()
}

/// The point on diagonal `k` after one more change from the front of `d - 1` changes,
/// before its run of equal lines: a line of `b` (down from `k + 1`) or a line of `a` (right
/// from `k - 1`), whichever reaches further in the grid of `size`. Gives its x and the
/// diagonal of the move, so the path back takes the same move.
fn step(before: &Front, d: usize, k: isize, (width, height): (usize, usize)) -> Option<(usize, isize)> {
    let in_grid = |x: &usize| x.checked_add_signed(-k).is_some_and(|y| y <= height);
    let down = reach(before, d - 1, k + 1).filter(in_grid).map(|x| (x, k + 1));
    let right = reach(before, d - 1, k - 1).map(|x| x + 1).filter(|x| *x <= width).map(|x| (x, k - 1));
    match (down, right) {
        (Some(down), Some(right)) => Some(if right.0 > down.0 { right } else { down }),
        (one, other) => one.or(other),
    }
}

/// The count of equal lines from (x, y) on.
fn snake(a: &[&str], b: &[&str], x: usize, y: usize) -> usize {
    a.iter().skip(x).zip(b.iter().skip(y)).take_while(|(a, b)| a == b).count()
}

/// The runs of equal lines on the path that ends at the end of the grid of `size` on
/// diagonal `k` of the last front, back to the start.
fn path(fronts: &[Front], mut k: isize, size: (usize, usize)) -> Vec<(usize, usize, usize)> {
    let mut runs = Vec::new();
    let mut x = size.0;
    for d in (0..fronts.len()).rev() {
        let before = d.checked_sub(1).and_then(|before| fronts.get(before));
        let (start, from) = match before {
            Some(before) => step(before, d, k, size).unwrap_or((x, k)),
            None => (0, k),
        };
        if x > start {
            runs.push((start, start.wrapping_add_signed(-k), x - start));
        }
        let Some(before) = before else {
            break;
        };
        x = reach(before, d - 1, from).unwrap_or(0);
        k = from;
    }
    runs.reverse();
    runs
}

#[cfg(test)]
mod tests;
