//! The kinds of the app: one registry of each resource type in scope (plan section 11.3).
//!
//! A kind is a typed identity, not a text. `list` holds the one list of all kinds, and the
//! file of each kind holds its description (plan section 11.4): the sidebar group, the
//! tier, the columns, the search text, the health value and the actions.
//!
//! - `describe`: the trait of a description. `column`: the columns and the value of a
//!   cell. `action`: the actions. `health`: the health value. `object`: what a store reads.
//! - `read`: the readers that the columns of many kinds share.
//! - One file for each kind. `pod` has also the status text of a pod, and `event` the state
//!   now of the problem of a warning Event.

mod action;
mod column;
mod describe;
mod health;
mod list;
mod object;
mod read;

mod api_service;
mod certificate_signing_request;
mod cluster_role;
mod cluster_role_binding;
mod config_map;
mod controller_revision;
mod cron_job;
mod daemon_set;
mod deployment;
mod endpoint_slice;
mod endpoints;
mod event;
mod flow_schema;
mod horizontal_pod_autoscaler;
mod ingress;
mod ingress_class;
mod job;
mod lease;
mod limit_range;
mod mutating_webhook_configuration;
mod namespace;
mod network_policy;
mod node;
mod pod;
mod pod_disruption_budget;
mod pod_template;
mod priority_class;
mod priority_level_configuration;
mod replica_set;
mod replication_controller;
mod resource_quota;
mod role;
mod role_binding;
mod runtime_class;
mod secret;
mod service;
mod service_account;
mod stateful_set;
mod validating_admission_policy;
mod validating_admission_policy_binding;
mod validating_webhook_configuration;

pub use action::Action;
pub use column::{Column, Value, columns};
pub use describe::{Describe, Scope, Sidebar};
pub(crate) use endpoint_slice::is_ready;
pub use event::{ProblemNow, ProblemState, Witness};
pub use health::ObjectHealth;
pub use list::{Kind, Visit};
pub use object::{Meta, Object};
pub use pod::PodStatus;

/// How much data of a kind the app keeps (plan section 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    /// A store that is always on, with full objects.
    A,
    /// A store that is always on, with the metadata of each object only. The server sends
    /// no other field, so no store holds the value of a Secret. A view that needs the full
    /// objects starts a store of them on demand.
    B,
    /// A store on demand only, because the objects of the kind change all the time.
    C,
}

/// The description of one kind with no type of its objects.
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
    /// The name of the kind in the sidebar, for example `Pods`.
    pub title: &'static str,
    /// The group of the kind in the sidebar.
    pub sidebar: Sidebar,
    /// How much data the app keeps.
    pub tier: Tier,
    /// True for a kind whose objects are in a namespace.
    pub namespaced: bool,
    /// The actions of the kind besides the actions of each kind.
    pub actions: &'static [Action],
}

impl KindInfo {
    /// The description of a kind, from the constants of its object type.
    const fn of<K: Describe>() -> Self {
        Self {
            name: <K as k8s_openapi::Resource>::KIND,
            group: K::GROUP,
            version: K::VERSION,
            plural: K::URL_PATH_SEGMENT,
            title: K::TITLE,
            sidebar: K::SIDEBAR,
            tier: K::TIER,
            namespaced: <K::Scope as Scope>::NAMESPACED,
            actions: K::ACTIONS,
        }
    }

    /// The path of the kind for all namespaces. A store reads all namespaces, and the
    /// namespace filter works in the app.
    pub fn path(&self) -> String {
        format!("{}/{}", self.group_path(), self.plural)
    }

    /// The path of the kind in one namespace, for a request about the objects of one
    /// namespace only.
    pub fn path_in(&self, namespace: &str) -> String {
        format!("{}/namespaces/{namespace}/{}", self.group_path(), self.plural)
    }

    fn group_path(&self) -> String {
        let Self { group, version, .. } = self;
        if group.is_empty() { format!("/api/{version}") } else { format!("/apis/{group}/{version}") }
    }
}

impl Kind {
    /// The kind with this name in a manifest, for example `Pod`. `None` for a kind that
    /// is out of the scope of the app.
    pub fn named(name: &str) -> Option<Kind> {
        Self::ALL.iter().copied().find(|kind| kind.info().name == name)
    }

    /// Each action on an object of the kind: the actions of each kind, then its own ones.
    pub fn actions(self) -> impl Iterator<Item = Action> {
        Action::EACH_KIND.into_iter().chain(self.info().actions.iter().copied())
    }
}

impl Sidebar {
    /// The kinds of the group, in the sequence of the sidebar.
    pub fn kinds(self) -> impl Iterator<Item = Kind> {
        Kind::ALL.iter().copied().filter(move |kind| kind.info().sidebar == self)
    }
}

#[cfg(test)]
mod tests;
