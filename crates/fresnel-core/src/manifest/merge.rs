//! The merge of two edits of one read (plan section 12.9): the text of the user and the new
//! text of the server, after a save that the server refused with a conflict.
//!
//! A change of one side that the other side did not touch goes into the merge. Two changes
//! of the same lines are one change when they are equal, and a conflict when they differ:
//! the user sees the three versions and chooses one side. A change that ends where an
//! insertion of the other side starts touches it, so the order of their lines is not
//! clear: that is a conflict too. Two changes of lines next to each other are not.
//!
//! The merge works on the lines of the YAML text, so it keeps the form of the text of the
//! user. The server checks the result in the dry run of the next save.

use std::ops::Range;

use super::diff::{self, Hunk};

/// The side of a conflict that the merge takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The text of the user.
    Ours,
    /// The new text of the server.
    Theirs,
}

/// Lines that the two sides changed in different ways.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The lines of the read.
    pub base: String,
    /// The lines of the user.
    pub ours: String,
    /// The lines of the server now.
    pub theirs: String,
}

/// A part of a merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// Lines that the merge takes: equal in the three versions, or changed by one side, or
    /// changed in the same way by both.
    Merged(String),
    Conflict(Conflict),
}

/// The merge of two edits of one text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merge {
    pub parts: Vec<Part>,
}

impl Merge {
    /// The conflicts, in the sequence of the text.
    pub fn conflicts(&self) -> impl Iterator<Item = &Conflict> {
        self.parts.iter().filter_map(|part| match part {
            Part::Conflict(conflict) => Some(conflict),
            Part::Merged(_) => None,
        })
    }

    /// True when the two sides changed no line in different ways.
    pub fn is_clean(&self) -> bool {
        self.conflicts().next().is_none()
    }

    /// The text of the merge with the side of each conflict that `choose` gives, by the
    /// number of the conflict from 0.
    pub fn text(&self, mut choose: impl FnMut(usize) -> Side) -> String {
        let mut number = 0;
        let mut text = String::new();
        for part in &self.parts {
            match part {
                Part::Merged(lines) => text.push_str(lines),
                Part::Conflict(conflict) => {
                    let side = choose(number);
                    number += 1;
                    text.push_str(if side == Side::Ours { &conflict.ours } else { &conflict.theirs });
                }
            }
        }
        text
    }
}

/// Merges the edits `ours` and `theirs` of the text `base`.
pub fn merge(base: &str, ours: &str, theirs: &str) -> Merge {
    let (base, ours, theirs) = (diff::lines(base), diff::lines(ours), diff::lines(theirs));
    let sides = [(&ours[..], diff::hunks(&base, &ours)), (&theirs[..], diff::hunks(&base, &theirs))];
    let mut next = [0, 0];
    let mut parts = Vec::new();
    let mut at = 0;
    while let Some(group) = group(&sides, &mut next) {
        push(&mut parts, Part::Merged(join(&base[at..group.lines.start])));
        let [ours, theirs] = [0, 1].map(|side| {
            let (lines, hunks) = &sides[side];
            group.hunks[side].clone().map(|hunk| apply(&base, lines, &hunks[hunk], group.lines.clone()))
        });
        let part = match (ours, theirs) {
            (Some(ours), Some(theirs)) if ours != theirs => {
                Part::Conflict(Conflict { base: join(&base[group.lines.clone()]), ours, theirs })
            }
            // A group has a hunk of at least one side.
            (ours, theirs) => Part::Merged(ours.or(theirs).unwrap_or_default()),
        };
        push(&mut parts, part);
        at = group.lines.end;
    }
    push(&mut parts, Part::Merged(join(&base[at..])));
    Merge { parts }
}

/// Changes of the two sides that touch each other: the lines of the base that they cover,
/// and the range of the hunks of each side, `None` for a side with none.
struct Group {
    lines: Range<usize>,
    hunks: [Option<Range<usize>>; 2],
}

/// The next group of hunks from `next`, which it moves past the group.
fn group(sides: &[(&[&str], Vec<Hunk>); 2], next: &mut [usize; 2]) -> Option<Group> {
    let hunk = |side: usize, next: &[usize; 2]| sides[side].1.get(next[side]).map(|hunk| hunk.old.clone());
    let first = (0..2).filter_map(|side| Some((hunk(side, next)?.start, side))).min()?.1;
    let start = hunk(first, next)?.start;
    let mut group = Group { lines: start..start, hunks: [None, None] };
    // True when an insertion ends the group: a change that starts there touches it.
    let mut insertion = false;
    let mut side = first;
    loop {
        let Some(old) = hunk(side, next) else {
            return Some(group);
        };
        if old.end > group.lines.end {
            (group.lines.end, insertion) = (old.end, false);
        }
        insertion |= old.is_empty() && old.end == group.lines.end;
        group.hunks[side] = Some(group.hunks[side].as_ref().map_or(next[side], |hunks| hunks.start)..next[side] + 1);
        next[side] += 1;
        let joins = |side: &usize| {
            hunk(*side, next).is_some_and(|old| {
                old.start < group.lines.end || (old.start == group.lines.end && (old.is_empty() || insertion))
            })
        };
        match (0..2).find(joins) {
            Some(joining) => side = joining,
            None => return Some(group),
        }
    }
}

/// The lines `range` of the base with the hunks of one side applied.
fn apply(base: &[&str], side: &[&str], hunks: &[Hunk], range: Range<usize>) -> String {
    let mut text = String::new();
    let mut at = range.start;
    for hunk in hunks {
        text.push_str(&join(&base[at..hunk.old.start]));
        text.push_str(&join(&side[hunk.new.clone()]));
        at = hunk.old.end;
    }
    text.push_str(&join(&base[at..range.end]));
    text
}

fn join(lines: &[&str]) -> String {
    lines.concat()
}

/// Adds a part, and joins merged lines to the merged lines before them.
fn push(parts: &mut Vec<Part>, part: Part) {
    match (parts.last_mut(), part) {
        (_, Part::Merged(lines)) if lines.is_empty() => {}
        (Some(Part::Merged(before)), Part::Merged(lines)) => before.push_str(&lines),
        (_, part) => parts.push(part),
    }
}

#[cfg(test)]
mod tests;
