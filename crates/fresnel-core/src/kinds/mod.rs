//! The kinds of the app: one registry of each resource type in scope (plan section 11.3).
//!
//! A kind is a typed identity, not a text. `list` holds the one list of all kinds. For now,
//! a kind has what a store needs: its place in the API, its tier and its object type. The
//! columns, the search text and the actions of a kind come with the views.
//!
//! - `list`: the one list. `object`: what a store reads of an object.
//! - `health`: the health value of an object. One file for each tier A kind has the rule
//!   of the kind, and `pod` has also the status text of a pod.

mod cron_job;
mod daemon_set;
mod deployment;
mod endpoint_slice;
mod event;
mod health;
mod job;
mod list;
mod namespace;
mod node;
mod object;
mod pod;
mod replica_set;
mod service;
mod stateful_set;

pub(crate) use endpoint_slice::is_ready;
pub use health::{Assess, ObjectHealth};
pub use list::Kind;
pub(crate) use list::{Visitor, each_store};
pub use object::{Meta, Object};
pub use pod::PodStatus;

/// How much data of a kind the app keeps (plan section 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    /// A store that is always on, with full objects.
    A,
    /// A store that is always on, with the metadata of each object only. The server sends
    /// no other field, so no store holds the value of a Secret.
    B,
    /// No store. A view reads the kind while it needs it, because its objects change all
    /// the time.
    C,
}

/// The description of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindInfo {
    /// The name of the kind in a manifest, for example `Pod`.
    pub name: &'static str,
    /// The API group. Empty for the core group.
    pub group: &'static str,
    /// The version of the API group.
    pub version: &'static str,
    /// The name of the kind in a URL, for example `pods`.
    pub plural: &'static str,
    /// How much data the app keeps.
    pub tier: Tier,
}

impl KindInfo {
    /// The description of a kind, from the constants of its object type.
    const fn of<K: k8s_openapi::Resource>(tier: Tier) -> Self {
        Self { name: K::KIND, group: K::GROUP, version: K::VERSION, plural: K::URL_PATH_SEGMENT, tier }
    }

    /// The path of the kind for all namespaces. A store reads all namespaces, and the
    /// namespace filter works in the app.
    pub fn path(&self) -> String {
        let Self { group, version, plural, .. } = self;
        if group.is_empty() { format!("/api/{version}/{plural}") } else { format!("/apis/{group}/{version}/{plural}") }
    }
}

/// The object type of a tier A kind. A reader asks for the store of a kind with this type,
/// so the type of the objects and the kind cannot differ. Each such type has a health value.
pub trait Full: Object + Assess {
    /// The kind of the type.
    const KIND: Kind;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn names(tier: Tier) -> Vec<&'static str> {
        Kind::ALL.iter().map(|kind| kind.info()).filter(|info| info.tier == tier).map(|info| info.name).collect()
    }

    #[test]
    fn the_tiers_hold_the_kinds_of_the_plan() {
        let mut full = names(Tier::A);
        full.sort_unstable();
        let expected = [
            "CronJob",
            "DaemonSet",
            "Deployment",
            "EndpointSlice",
            "Event",
            "Job",
            "Namespace",
            "Node",
            "Pod",
            "ReplicaSet",
            "Service",
            "StatefulSet",
        ];
        assert_eq!(full, expected, "plan section 9.3");
        assert_eq!(names(Tier::B).len(), 27, "each other kind of plan section 11.3");
        assert_eq!(names(Tier::C), ["Lease", "Endpoints"], "they change all the time");
    }

    #[test]
    fn each_kind_has_its_own_path_and_no_store_holds_the_data_of_a_secret() {
        let paths: BTreeSet<String> = Kind::ALL.iter().map(|kind| kind.info().path()).collect();
        assert_eq!(paths.len(), Kind::ALL.len());
        // An Event names the kind of its object, so a name is one kind.
        assert!(Kind::ALL.iter().all(|kind| Kind::named(kind.info().name) == Some(*kind)));
        assert_eq!(Kind::named("Widget"), None, "a kind out of scope");
        assert_eq!(Kind::Pod.info().path(), "/api/v1/pods");
        assert_eq!(Kind::Deployment.info().path(), "/apis/apps/v1/deployments");
        assert_eq!(Kind::ApiService.info().path(), "/apis/apiregistration.k8s.io/v1/apiservices");
        assert_eq!((Kind::Secret.info().tier, Kind::ConfigMap.info().tier), (Tier::B, Tier::B));
    }

    #[test]
    fn the_first_lists_are_the_kinds_that_each_view_needs_and_the_events_come_last() {
        let stored: Vec<Kind> = Kind::ALL.iter().copied().filter(|kind| kind.info().tier != Tier::C).collect();
        assert_eq!(stored[..3], [Kind::Namespace, Kind::Node, Kind::Pod]);
        assert_eq!(stored.last(), Some(&Kind::Event));
        assert!(Kind::Pod < Kind::Secret && Kind::Secret < Kind::Event, "the sequence is the priority of a kind");
        assert_eq!(<k8s_openapi::api::core::v1::Pod as Full>::KIND, Kind::Pod);
    }
}
