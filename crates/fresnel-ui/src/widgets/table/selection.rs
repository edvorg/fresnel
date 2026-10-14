//! The selection and the cursor of a table, by the identity of an object (plan section 11.2).
//!
//! In the UI prototype, the selection held row numbers next to the keys and never
//! removed a key: a deleted object stayed selected, and a result for other data moved
//! the cursor to some other object. Here the selection knows objects only. The table
//! finds the row of an object when a click or a key needs it.

use std::collections::HashSet;

use super::model::ObjectId;

/// The objects that the user selected in one table.
#[derive(Debug, Default)]
pub(crate) struct Selection {
    ids: HashSet<ObjectId>,
    /// The object that the keys move from and that the detail panel shows.
    cursor: Option<ObjectId>,
    /// The other end of a range that Shift selects.
    anchor: Option<ObjectId>,
}

impl Selection {
    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }

    /// The selected objects, in no fixed sequence: the objects of a bulk action.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &ObjectId> {
        self.ids.iter()
    }

    pub(crate) fn contains(&self, id: &ObjectId) -> bool {
        self.ids.contains(id)
    }

    pub(crate) fn cursor(&self) -> Option<&ObjectId> {
        self.cursor.as_ref()
    }

    pub(crate) fn anchor(&self) -> Option<&ObjectId> {
        self.anchor.as_ref()
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// A click, or a key with no Shift: this object only.
    pub(crate) fn only(&mut self, id: ObjectId) {
        self.ids.clear();
        self.ids.insert(id.clone());
        self.anchor = Some(id.clone());
        self.cursor = Some(id);
    }

    /// A click with Cmd: adds the object, or removes it.
    pub(crate) fn toggle(&mut self, id: ObjectId) {
        if !self.ids.remove(&id) {
            self.ids.insert(id.clone());
        }
        self.anchor = Some(id.clone());
        self.cursor = Some(id);
    }

    /// A click or a key with Shift: the objects of the rows from `anchor` to `cursor`.
    /// The table gives the anchor that it found in its rows, so the next Shift key makes
    /// the range longer or shorter from the same end.
    pub(crate) fn range(&mut self, ids: impl IntoIterator<Item = ObjectId>, anchor: ObjectId, cursor: ObjectId) {
        self.ids = ids.into_iter().collect();
        self.anchor = Some(anchor);
        self.cursor = Some(cursor);
    }

    /// Cmd+A: each object of the rows. The cursor stays.
    pub(crate) fn all(&mut self, ids: impl IntoIterator<Item = ObjectId>) {
        self.ids = ids.into_iter().collect();
    }

    /// Removes each object that `gone` names. The caller gives the objects that left
    /// their store, so no deleted object stays selected.
    pub(crate) fn forget(&mut self, gone: impl Fn(&ObjectId) -> bool) {
        self.ids.retain(|id| !gone(id));
        self.cursor.take_if(|id| gone(id));
        self.anchor.take_if(|id| gone(id));
    }
}

#[cfg(test)]
mod tests {
    use fresnel_core::cluster::ClusterId;
    use fresnel_core::store::ObjectKey;

    use super::*;

    fn id(cluster: &str, uid: &str) -> ObjectId {
        ObjectId { cluster: ClusterId::new(cluster), key: ObjectKey::new("shop", "web"), uid: uid.into() }
    }

    #[test]
    fn a_click_selects_one_object_and_cmd_adds_or_removes_one() {
        let mut selection = Selection::default();
        selection.only(id("ams", "a"));
        selection.toggle(id("ams", "b"));
        // The same UID in an other cluster is an other object (design rule 8).
        selection.toggle(id("fra", "a"));
        assert_eq!((selection.len(), selection.cursor()), (3, Some(&id("fra", "a"))));
        selection.toggle(id("ams", "a"));
        assert!(!selection.contains(&id("ams", "a")) && selection.contains(&id("fra", "a")));
        assert_eq!(selection.cursor(), Some(&id("ams", "a")), "the cursor is the last object of a click");
        selection.only(id("ams", "b"));
        assert_eq!((selection.len(), selection.anchor()), (1, Some(&id("ams", "b"))));
        selection.clear();
        assert!(selection.len() == 0 && selection.cursor().is_none() && selection.anchor().is_none());
    }

    #[test]
    fn a_range_has_the_anchor_of_its_caller_and_select_all_keeps_the_cursor() {
        let mut selection = Selection::default();
        selection.only(id("ams", "b"));
        selection.range([id("ams", "b"), id("ams", "c"), id("ams", "d")], id("ams", "b"), id("ams", "d"));
        assert_eq!(selection.len(), 3);
        assert_eq!((selection.anchor(), selection.cursor()), (Some(&id("ams", "b")), Some(&id("ams", "d"))));
        selection.range([id("ams", "a"), id("ams", "b")], id("ams", "b"), id("ams", "a"));
        assert_eq!((selection.len(), selection.anchor()), (2, Some(&id("ams", "b"))), "the range turns at the anchor");
        selection.all([id("ams", "a"), id("ams", "b"), id("ams", "c")]);
        assert_eq!((selection.len(), selection.cursor()), (3, Some(&id("ams", "a"))));
    }

    /// Defect A1b of the prototype: three deleted objects stayed "3 selected".
    #[test]
    fn the_selection_drops_the_objects_that_go_away_with_the_cursor_and_the_anchor() {
        let mut selection = Selection::default();
        selection.range([id("ams", "a"), id("ams", "b"), id("fra", "a")], id("ams", "a"), id("fra", "a"));
        selection.forget(|gone| gone.cluster.as_str() == "ams" && &*gone.uid == "b");
        assert_eq!(selection.len(), 2);
        assert_eq!((selection.anchor(), selection.cursor()), (Some(&id("ams", "a")), Some(&id("fra", "a"))));

        selection.forget(|gone| gone.cluster.as_str() == "fra");
        assert_eq!((selection.len(), selection.cursor()), (1, None), "the cursor went with its object");
        selection.forget(|gone| &*gone.uid == "a");
        assert!(selection.len() == 0 && selection.anchor().is_none());
    }
}
