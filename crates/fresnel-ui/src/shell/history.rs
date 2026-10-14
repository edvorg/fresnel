//! The visited objects (plan section 11.5): Back and Forward move through them, as in a
//! browser. A visit after a step back drops the objects after it.

use crate::views::objects::Link;

/// The most objects that the history keeps. The oldest one leaves first.
const LIMIT: usize = 100;

#[derive(Debug, Default)]
pub(super) struct History {
    visited: Vec<Link>,
    /// The place of the object of now in `visited`.
    at: usize,
}

impl History {
    /// Keeps an object that the detail panel shows now. The object of now again is no visit.
    pub(super) fn visit(&mut self, link: Link) {
        if self.visited.get(self.at) == Some(&link) {
            return;
        }
        let keep = if self.visited.is_empty() { 0 } else { self.at + 1 };
        self.visited.truncate(keep);
        self.visited.push(link);
        if self.visited.len() > LIMIT {
            self.visited.remove(0);
        }
        self.at = self.visited.len() - 1;
    }

    /// The object before the one of now. `None` at the first one.
    pub(super) fn back(&mut self) -> Option<Link> {
        self.at = self.at.checked_sub(1)?;
        self.visited.get(self.at).cloned()
    }

    /// The object after the one of now, after a step back. `None` at the last one.
    pub(super) fn forward(&mut self) -> Option<Link> {
        let next = self.visited.get(self.at + 1)?.clone();
        self.at += 1;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use fresnel_core::cluster::ClusterId;
    use fresnel_core::kinds::Kind;
    use fresnel_core::store::ObjectKey;

    use super::*;

    fn link(name: &str) -> Link {
        Link { cluster: ClusterId::new("ams"), kind: Kind::Pod, key: ObjectKey::new("shop", name) }
    }

    fn name(link: Option<Link>) -> Option<String> {
        link.map(|link| link.key.name.to_string())
    }

    #[test]
    fn back_and_forward_move_through_the_visits_and_a_new_visit_drops_the_ones_after_it() {
        let mut history = History::default();
        assert_eq!((name(history.back()), name(history.forward())), (None, None), "no visit");
        for pod in ["a", "b", "b", "c"] {
            history.visit(link(pod));
        }
        assert_eq!(name(history.back()), Some("b".to_owned()), "the same object two times is one visit");
        assert_eq!(name(history.back()), Some("a".to_owned()));
        assert_eq!(name(history.back()), None, "the first one");
        assert_eq!(name(history.forward()), Some("b".to_owned()));
        // The panel shows the object of a step: that is no new visit.
        history.visit(link("b"));
        assert_eq!(name(history.forward()), Some("c".to_owned()));
        assert_eq!(name(history.forward()), None, "the last one");

        assert_eq!(name(history.back()), Some("b".to_owned()));
        history.visit(link("d"));
        assert_eq!(name(history.forward()), None, "c went with the new visit");
        assert_eq!(name(history.back()), Some("b".to_owned()));
    }

    #[test]
    fn the_history_keeps_the_newest_visits_to_its_limit() {
        let mut history = History::default();
        for number in 0..LIMIT + 20 {
            history.visit(link(&number.to_string()));
        }
        let mut steps = 0;
        while history.back().is_some() {
            steps += 1;
        }
        assert_eq!(steps, LIMIT - 1);
        assert_eq!(name(history.forward()), Some("21".to_owned()), "the oldest one that stays");
    }
}
