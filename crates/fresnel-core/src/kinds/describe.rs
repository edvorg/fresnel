//! The description of one kind (plan section 11.4). The file of each kind implements
//! [`Describe`] for its object type, and the list of the kinds registers that type.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::{ClusterResourceScope, Metadata, NamespaceResourceScope, Resource};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Action, Column, Kind, ObjectHealth, Tier};

/// The group of a kind in the sidebar (plan section 11.3). The sequence is the sequence
/// of the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sidebar {
    /// Nodes, Namespaces and Events.
    Cluster,
    /// Pods and the kinds that make pods.
    Workloads,
    /// ConfigMaps, Secrets and the rules for pods and for the API server.
    Config,
    /// Services and the kinds that carry traffic.
    Network,
    /// ServiceAccounts and the kinds of RBAC.
    AccessControl,
    /// The kinds with the generic view only.
    Other,
}

impl Sidebar {
    /// Each group, in the sequence of the sidebar.
    pub const ALL: [Self; 6] =
        [Self::Cluster, Self::Workloads, Self::Config, Self::Network, Self::AccessControl, Self::Other];

    /// The title of the group.
    pub fn title(self) -> &'static str {
        match self {
            Self::Cluster => "Cluster",
            Self::Workloads => "Workloads",
            Self::Config => "Config",
            Self::Network => "Network",
            Self::AccessControl => "Access control",
            Self::Other => "Other",
        }
    }
}

/// What the app knows of the objects of one kind: one implementation for each object type
/// in the scope of the app, in the file of its kind.
///
/// A value of the description comes from the object alone, with no clock and no other
/// object. Thus a row of a table changes only when its own object changes. A view writes
/// the status of an object as JSON to read the parts that each kind has, such as its
/// conditions.
pub trait Describe:
    Resource<Scope: Scope> + Metadata<Ty = ObjectMeta> + DeserializeOwned + Serialize + Send + Sync + 'static
{
    /// The identity of the kind. The list of the kinds checks that it names this type.
    /// (The name `KIND` is the name of the kind in a manifest, from `Resource`.)
    const ID: Kind;
    /// The name of the kind in the sidebar and in a title: the plural, for example `Pods`.
    const TITLE: &'static str;
    /// The group of the kind in the sidebar.
    const SIDEBAR: Sidebar;
    /// How much data of the kind the app keeps (plan sections 9.3 to 9.5).
    const TIER: Tier;
    /// The columns of the kind besides the columns of each kind: see [`super::columns`].
    const COLUMNS: &'static [Column<Self>];
    /// The actions of the kind besides the actions of each kind: see [`Kind::actions`].
    /// In M2, an action is a name only.
    const ACTIONS: &'static [Action];

    /// How well the object does.
    fn health(&self) -> ObjectHealth;

    /// Gives each text of the object that the quick search matches, besides the name, the
    /// namespace, the labels and the owners, which each object has (plan section 12.1):
    /// images, node and IP addresses.
    fn search<'a>(&'a self, _texts: &mut Vec<&'a str>) {}

    /// Removes the fields that no store may keep, before the object goes into a store.
    /// The metadata rule of each kind runs too.
    fn trim(&mut self) {}
}

/// Where the objects of a kind are: in a namespace, or in the cluster. The two scope types
/// of `k8s-openapi` implement it, so the type of a kind tells it.
pub trait Scope {
    /// True for a kind whose objects are in a namespace.
    const NAMESPACED: bool;
}

impl Scope for NamespaceResourceScope {
    const NAMESPACED: bool = true;
}

impl Scope for ClusterResourceScope {
    const NAMESPACED: bool = false;
}

/// The health value of an object with no rule of its own kind: an object in its delete is
/// on its way, and each other object is healthy.
pub(super) fn by_metadata(meta: &ObjectMeta) -> ObjectHealth {
    if meta.deletion_timestamp.is_some() { ObjectHealth::Progressing } else { ObjectHealth::Healthy }
}
