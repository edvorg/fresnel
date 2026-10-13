//! The result of a write that a store does not show yet (plan section 9.10).
//!
//! The answer of the server to a write gives the object after the write. The watch event
//! of the same change arrives later: some milliseconds, or much more on a bad network.
//! In that time, a view shows the object of the store as "change pending".
//!
//! The rule is a comparison of that answer with a snapshot, and not a mark in the store.
//! A mark would need a second writer of the store, and a mark that no event removes
//! would stay for all time.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::items::ObjectKey;
use super::snapshot::Snapshot;
use crate::kinds::Object;

/// What a write left in the cluster, as the answer of the server tells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    key: ObjectKey,
    /// An other object can get the same name later. The write is not about that one.
    uid: String,
    /// The resource version after the write. `None`: the write removed the object.
    version: Option<String>,
}

impl Written {
    /// The result of a write whose answer is the object: a create, a change, or a delete
    /// that waits for a finalizer or for the end of a pod. `None` for metadata with no
    /// name, no UID or no resource version. The answer of a server has each one.
    pub fn changed(answer: &ObjectMeta) -> Option<Self> {
        let version = answer.resource_version.clone().filter(|version| !version.is_empty())?;
        Self::of(answer, Some(version))
    }

    /// The result of a delete that the server completed. Its answer is a status, so the
    /// identity comes from the object that the user selected.
    pub fn removed(object: &ObjectMeta) -> Option<Self> {
        Self::of(object, None)
    }

    fn of(meta: &ObjectMeta, version: Option<String>) -> Option<Self> {
        let uid = meta.uid.clone().filter(|uid| !uid.is_empty())?;
        Some(Self { key: ObjectKey::of(meta)?, uid, version })
    }

    /// The identity of the object of the write.
    pub fn key(&self) -> &ObjectKey {
        &self.key
    }
}

impl<T: Object> Snapshot<T> {
    /// True while the store holds the object of the write in a state from before the
    /// write: the watch event of the write did not arrive yet.
    ///
    /// False for an object that the store does not have, because no row can show the
    /// mark. Thus a result that no event follows does not stay pending for all time, for
    /// example when some other user removed the object a moment later.
    pub fn is_pending(&self, written: &Written) -> bool {
        let Some(stored) = self.items.get(&written.key).map(|object| object.meta()) else {
            return false;
        };
        if stored.uid.as_deref() != Some(written.uid.as_str()) {
            return false;
        }
        match &written.version {
            None => true,
            Some(version) => is_older(stored.resource_version.as_deref(), version),
        }
    }
}

/// True when the version of the store is from before the version of the write. A
/// kube-apiserver gives a number that grows with each change of the cluster.
///
/// With a version that is not a number, no rule tells the older one. The answer is then
/// false: a mark that never goes is a worse error than no mark.
fn is_older(stored: Option<&str>, written: &str) -> bool {
    let number = |version: &str| version.parse::<u128>().ok();
    match (stored.and_then(number), number(written)) {
        (Some(stored), Some(written)) => stored < written,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::SystemTime;

    use super::*;
    use crate::kinds::Meta;
    use crate::store::StoreState;

    fn meta(name: &str, uid: &str, version: &str) -> ObjectMeta {
        let text = format!(r#"{{"name":"{name}","namespace":"shop","uid":"{uid}","resourceVersion":"{version}"}}"#);
        serde_json::from_str(&text).expect("the metadata of an object")
    }

    /// A snapshot with these objects, each one as name, UID and resource version.
    fn snapshot(objects: &[(&str, &str, &str)]) -> Snapshot<Meta> {
        let mut snapshot = Snapshot::empty(StoreState::Live, SystemTime::UNIX_EPOCH);
        for (name, uid, version) in objects {
            let metadata = meta(name, uid, version);
            snapshot.items.insert(ObjectKey::of(&metadata).expect("a name"), Arc::new(Meta { metadata }));
        }
        snapshot
    }

    #[test]
    fn a_change_is_pending_until_the_store_has_its_version_or_a_later_one() {
        // The versions are numbers: "100" is after "20", and a compare of the texts says the opposite.
        let scaled = Written::changed(&meta("web", "uid-web", "100")).expect("a result");
        assert_eq!(scaled.key(), &ObjectKey::new("shop", "web"));
        assert!(snapshot(&[("web", "uid-web", "20")]).is_pending(&scaled));
        // The event of an earlier change is not the event of the write.
        assert!(snapshot(&[("web", "uid-web", "99")]).is_pending(&scaled));
        assert!(!snapshot(&[("web", "uid-web", "100")]).is_pending(&scaled), "the event of the write arrived");
        // The store can get a later version first, for example with a new list.
        assert!(!snapshot(&[("web", "uid-web", "101")]).is_pending(&scaled));

        let other = snapshot(&[("db", "uid-db", "9"), ("web", "uid-new", "30")]);
        assert!(!other.is_pending(&scaled), "a new object with the name of the old one is not the object of the write");
        let created = Written::changed(&meta("job", "uid-job", "50")).expect("a result");
        assert!(!other.is_pending(&created), "the store has no row for a mark yet");
    }

    #[test]
    fn a_delete_is_pending_while_the_store_has_that_object() {
        let removed = Written::removed(&meta("web", "uid-web", "20")).expect("a result");
        assert!(snapshot(&[("web", "uid-web", "20")]).is_pending(&removed), "the DELETED event did not arrive");
        assert!(snapshot(&[("web", "uid-web", "21")]).is_pending(&removed), "the version has no meaning for a delete");
        assert!(!snapshot(&[("web", "uid-new", "30")]).is_pending(&removed), "the next object of that name");
        assert!(!snapshot(&[("db", "uid-db", "9")]).is_pending(&removed), "no object, no mark");
    }

    #[test]
    fn a_version_that_is_not_a_number_and_metadata_with_a_gap_give_no_mark() {
        let opaque = Written::changed(&meta("web", "uid-web", "v-7f3a")).expect("a result");
        assert!(!snapshot(&[("web", "uid-web", "20")]).is_pending(&opaque), "no rule tells which version is older");
        let numbered = Written::changed(&meta("web", "uid-web", "21")).expect("a result");
        assert!(!snapshot(&[("web", "uid-web", "")]).is_pending(&numbered));

        assert_eq!(Written::changed(&meta("web", "uid-web", "")), None, "no version");
        assert_eq!(Written::changed(&meta("web", "", "21")), None, "no UID");
        assert_eq!(Written::removed(&meta("", "uid-web", "21")), None, "no name");
        assert!(Written::removed(&meta("web", "uid-web", "")).is_some(), "a delete needs no version");
    }
}
