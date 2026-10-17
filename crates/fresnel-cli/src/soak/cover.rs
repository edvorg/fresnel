//! Does a store hold the state of a list of the API, or a later one?
//!
//! The cluster changes during the soak, so a store and a list are seldom equal at one
//! moment. The test is thus "at least as new": each object of the list is in the store in
//! its version or a later one, and each other object of the store came after the list. A
//! resource version of Kubernetes grows with each change of the cluster, so a later version
//! is a larger number. An object of the list that the store does not have needs a second
//! list: it is no fault when the object is gone there.

use std::collections::BTreeMap;
use std::sync::Arc;

/// The namespace and the name of an object.
pub(super) type Key = (Arc<str>, Arc<str>);

/// One object as a list or a store has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Version {
    pub(super) uid: Arc<str>,
    /// The resource version.
    pub(super) text: Arc<str>,
    /// The same as a number. `None` for a text that is not a number.
    pub(super) number: Option<u64>,
}

impl Version {
    pub(super) fn new(uid: Option<&str>, version: Option<&str>) -> Self {
        let text = version.unwrap_or_default();
        Self { uid: uid.unwrap_or_default().into(), text: text.into(), number: text.parse().ok() }
    }
}

/// The objects of one kind.
pub(super) type Versions = BTreeMap<Key, Version>;

/// One list of the API: its objects and the resource version of the list.
#[derive(Clone, Debug, Default)]
pub(super) struct Listed {
    pub(super) objects: Versions,
    pub(super) version: Option<u64>,
}

/// The result of one comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// The store has the state of the list or a later one.
    Covered,
    /// The store does not have some objects of the list: a later list tells if they are gone.
    NeedsLater,
    /// The store is behind the list. The text names the first difference.
    Behind(String),
}

/// Compares the store with the list `first`. `later` is a list after the read of the store.
/// A store that keeps only the newest objects of its kind (`partial`, the Events at their
/// limit) can lack an object. An object that only the store has is stale when `later` lacks
/// it too: a list in pages of k3s once lacked an Event that the API had before and after it.
pub(super) fn cover(first: &Listed, store: &Versions, later: Option<&Listed>, partial: bool) -> Verdict {
    let mut absent = false;
    for (key, listed) in &first.objects {
        match store.get(key) {
            Some(stored) if is_at_least(stored, listed) => {}
            Some(_) => return Verdict::Behind(format!("{} has an older version", name(key))),
            None if partial => {}
            None => match later.map(|later| later.objects.get(key)) {
                None => absent = true,
                Some(Some(again)) if again.uid == listed.uid => {
                    return Verdict::Behind(format!("{} is not in the store", name(key)));
                }
                Some(_) => {}
            },
        }
    }
    let stale = store.iter().find(|(key, stored)| {
        let after =
            |list: &Listed| matches!((stored.number, list.version), (Some(stored), Some(list)) if stored > list);
        let in_later =
            later.map(|later| later.objects.get(*key).is_some_and(|again| again.uid == stored.uid) || after(later));
        !first.objects.contains_key(*key) && !after(first) && in_later != Some(true)
    });
    match (stale, later) {
        (Some(_), None) => Verdict::NeedsLater,
        (Some((key, _)), Some(_)) => Verdict::Behind(format!("{} is only in the store", name(key))),
        (None, _) if absent => Verdict::NeedsLater,
        (None, _) => Verdict::Covered,
    }
}

/// The same object in the same version, or a later one.
fn is_at_least(stored: &Version, listed: &Version) -> bool {
    match (stored.number, listed.number) {
        (Some(stored), Some(listed)) => stored >= listed,
        _ => stored == listed,
    }
}

fn name((namespace, name): &Key) -> String {
    if namespace.is_empty() { name.to_string() } else { format!("{namespace}/{name}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(objects: &[(&str, &str, u64)]) -> Versions {
        let entry = |(name, uid, number): &(&str, &str, u64)| {
            ((Arc::from("ns"), Arc::from(*name)), Version::new(Some(uid), Some(&number.to_string())))
        };
        objects.iter().map(entry).collect()
    }

    fn listed(objects: &[(&str, &str, u64)], version: u64) -> Listed {
        Listed { objects: versions(objects), version: Some(version) }
    }

    #[test]
    fn a_store_with_the_same_or_later_objects_covers_the_list() {
        let list = listed(&[("a", "u-a", 10), ("b", "u-b", 12)], 20);
        assert_eq!(cover(&list, &versions(&[("a", "u-a", 10), ("b", "u-b", 12)]), None, false), Verdict::Covered);
        let later = versions(&[("a", "u-a", 25), ("b", "u-b", 12), ("c", "u-c", 21)]);
        assert_eq!(cover(&list, &later, None, false), Verdict::Covered, "a change and a new object after the list");
    }

    #[test]
    fn an_old_version_or_an_object_that_the_api_removed_is_behind() {
        let list = listed(&[("a", "u-a", 10)], 20);
        let old = versions(&[("a", "u-a", 9)]);
        assert_eq!(cover(&list, &old, None, false), Verdict::Behind("ns/a has an older version".to_owned()));
        let gone = versions(&[("a", "u-a", 10), ("z", "u-z", 15)]);
        assert_eq!(cover(&list, &gone, None, false), Verdict::NeedsLater, "a second list tells");
        let behind = Verdict::Behind("ns/z is only in the store".to_owned());
        assert_eq!(cover(&list, &gone, Some(&listed(&[("a", "u-a", 10)], 30)), false), behind);
        // A list of k3s in pages once lacked an object that the API had before and after it.
        let again = listed(&[("a", "u-a", 10), ("z", "u-z", 15)], 30);
        assert_eq!(cover(&list, &gone, Some(&again), false), Verdict::Covered);
        let other = listed(&[("a", "u-a", 10), ("z", "u-z2", 25)], 30);
        assert_eq!(cover(&list, &gone, Some(&other), false), behind, "a new object of the same name");
    }

    #[test]
    fn an_absent_object_needs_a_later_list_and_is_no_fault_when_it_is_gone_there() {
        let list = listed(&[("a", "u-a", 10), ("b", "u-b", 12)], 20);
        let store = versions(&[("a", "u-a", 10)]);
        assert_eq!(cover(&list, &store, None, false), Verdict::NeedsLater);
        let gone = listed(&[("a", "u-a", 10)], 30);
        assert_eq!(cover(&list, &store, Some(&gone), false), Verdict::Covered);
        let new_object = listed(&[("a", "u-a", 10), ("b", "u-b2", 28)], 30);
        assert_eq!(cover(&list, &store, Some(&new_object), false), Verdict::Covered, "deleted, then a new one");
        let still = listed(&[("a", "u-a", 10), ("b", "u-b", 12)], 30);
        let behind = Verdict::Behind("ns/b is not in the store".to_owned());
        assert_eq!(cover(&list, &store, Some(&still), false), behind);
        assert_eq!(cover(&list, &store, None, true), Verdict::Covered, "a store at its limit");
    }

    #[test]
    fn a_version_that_is_not_a_number_must_be_equal() {
        let text = |version: &str| Version::new(Some("u-a"), Some(version));
        let list = Listed { objects: [((Arc::from(""), Arc::from("a")), text("x1"))].into(), version: None };
        let same: Versions = [((Arc::from(""), Arc::from("a")), text("x1"))].into();
        let other: Versions = [((Arc::from(""), Arc::from("a")), text("x2"))].into();
        assert_eq!(cover(&list, &same, None, false), Verdict::Covered);
        assert_eq!(cover(&list, &other, None, false), Verdict::Behind("a has an older version".to_owned()));
    }
}
