//! The search of an editor: the hits of a text in its lines, the hit of the cursor, and the
//! moves to the next and the one before.
//!
//! The search reads 8,000 lines in less than a millisecond, so it runs in the frame of a
//! change. A text with no capital letter ignores the case, as the search of a log
//! (`logs::needle`). The hits have a limit: a search of `e` would else hold a place for each
//! letter of the text.

use crate::views::logs::needle::Needle;

use super::text::{Hit, Pos, TextArea};

/// The most hits of a search.
const MOST: usize = 10_000;

#[derive(Debug, Default)]
pub(super) struct Find {
    /// True while the bar of the search shows.
    pub(super) open: bool,
    /// The text of the field.
    pub(super) text: String,
    /// The hits for the needle and the generation of the text.
    hits: Vec<Hit>,
    found: Option<(Option<Needle>, u64)>,
    /// True when the search found more hits than it keeps.
    more: bool,
    /// The hit of the cursor.
    current: Option<usize>,
    /// True when the field takes the focus in the next frame.
    pub(super) focus: bool,
    /// The identity of the field in its last frame.
    pub(super) field: Option<egui::Id>,
}

impl Find {
    /// Opens the bar and gives its field the focus.
    pub(super) fn start(&mut self) {
        (self.open, self.focus) = (true, true);
    }

    /// The hits for the text of the field in this generation of the text.
    pub(super) fn update(&mut self, text: &TextArea) {
        let needle = if self.open { Needle::new(&self.text) } else { None };
        let wanted = (needle, text.generation());
        if self.found.as_ref() == Some(&wanted) {
            return;
        }
        (self.hits, self.more, self.current) = (Vec::new(), false, None);
        if let Some(needle) = &wanted.0 {
            'lines: for line in 0..text.lines() {
                let content = text.line(line);
                for bytes in needle.places(content, content.len()) {
                    if self.hits.len() == MOST {
                        self.more = true;
                        break 'lines;
                    }
                    self.hits.push(Hit { line, bytes });
                }
            }
        }
        self.current = self.hits.iter().position(|hit| Pos::new(hit.line, hit.bytes.start) >= text.cursor());
        self.found = Some(wanted);
    }

    /// True when this is the identity of the field.
    pub(super) fn owns(&self, id: egui::Id) -> bool {
        self.open && self.field == Some(id)
    }

    pub(super) fn hits(&self) -> &[Hit] {
        if self.open { &self.hits } else { &[] }
    }

    pub(super) fn current(&self) -> Option<usize> {
        self.current
    }

    /// "3 of 12", "1 hit", "no hit", "10,000+ hits".
    pub(super) fn count(&self) -> String {
        let total = crate::format::thousands(self.hits.len() as u64);
        match (self.hits.is_empty(), self.more, self.current) {
            (true, _, _) => "no hit".to_owned(),
            (false, true, _) => format!("{total}+ hits"),
            (false, false, Some(current)) => format!("{} of {total}", current + 1),
            (false, false, None) => crate::format::count(self.hits.len() as u64, "hit"),
        }
    }

    /// Selects the next hit after the cursor, or the one before it, round the end.
    pub(super) fn go(&mut self, text: &mut TextArea, forward: bool) {
        if self.hits.is_empty() {
            return;
        }
        let cursor = text.cursor();
        let place = |hit: &Hit| Pos::new(hit.line, hit.bytes.start);
        let index = if forward {
            self.hits.iter().position(|hit| place(hit) > cursor || self.current.is_none() && place(hit) == cursor)
        } else {
            self.hits.iter().rposition(|hit| Pos::new(hit.line, hit.bytes.end) < cursor)
        };
        let last = self.hits.len() - 1;
        let index = index.unwrap_or(if forward { 0 } else { last });
        if let Some(hit) = self.hits.get(index) {
            text.select(Pos::new(hit.line, hit.bytes.start)..Pos::new(hit.line, hit.bytes.end));
            self.current = Some(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_finds_each_hit_and_goes_round_the_end_in_both_ways() {
        let mut text = TextArea::new("name: web\nimage: nginx\nlabels:\n  name: WEB\n", 100, true);
        let mut find = Find { open: true, text: "web".to_owned(), ..Find::default() };
        find.update(&text);
        assert_eq!(
            find.hits(),
            [Hit { line: 0, bytes: 6..9 }, Hit { line: 3, bytes: 8..11 }],
            "small letters ignore the case"
        );
        assert_eq!(find.count(), "1 of 2", "the hit at the cursor");
        find.go(&mut text, true);
        assert_eq!((text.cursor(), find.count()), (Pos::new(0, 9), "1 of 2".to_owned()));
        find.go(&mut text, true);
        assert_eq!((text.cursor(), find.count()), (Pos::new(3, 11), "2 of 2".to_owned()));
        find.go(&mut text, true);
        assert_eq!(text.cursor(), Pos::new(0, 9), "round the end");
        find.go(&mut text, false);
        assert_eq!(text.cursor(), Pos::new(3, 11), "round the start");
        find.text = "WEB".to_owned();
        find.update(&text);
        assert_eq!(find.hits().len(), 1, "a capital letter keeps the case");
        find.open = false;
        find.update(&text);
        assert!(find.hits().is_empty());
    }

    #[test]
    fn a_search_keeps_10_000_hits_and_tells_of_more() {
        let text = TextArea::new(&"e".repeat(20_000), 10, true);
        let mut find = Find { open: true, text: "e".to_owned(), ..Find::default() };
        find.update(&text);
        assert_eq!((find.hits().len(), find.count().as_str()), (MOST, "10,000+ hits"));
    }
}
