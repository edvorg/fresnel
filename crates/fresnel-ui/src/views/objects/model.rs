//! What a kind gives to its page, and what the rows of each kind share: the stores that
//! they come from and what a cell needs of each cluster at its paint.

use std::collections::HashMap;
use std::sync::Arc;

use egui::Color32;
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::{Describe, Object};
use fresnel_core::published::Held;
use fresnel_core::store::Snapshot;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::filter::{Namespaces, ObjectFilter};
use crate::format::{self, Clock};
use crate::theme::Palette;
use crate::widgets::table::model::{Model, ObjectId};

/// The stores of one kind of the selected clusters at one moment.
pub(crate) struct Stores<T: Send + Sync + 'static> {
    pub(crate) clusters: Vec<(ClusterId, Held<Snapshot<T>>)>,
}

impl<T: Send + Sync + 'static> Clone for Stores<T> {
    fn clone(&self) -> Self {
        Self { clusters: self.clusters.clone() }
    }
}

impl<T: Object> Stores<T> {
    /// True while the store of its cluster has this object. A new object with the name of
    /// it is an other object.
    pub(crate) fn has(&self, id: &ObjectId) -> bool {
        let store = self.clusters.iter().find(|(cluster, _)| *cluster == id.cluster);
        let object = store.and_then(|(_, store)| store.items.get(&id.key));
        object.is_some_and(|object| uid(object.meta()) == &*id.uid)
    }
}

/// The UID of an object. Empty for an object that its server gave none.
pub(crate) fn uid(meta: &ObjectMeta) -> &str {
    meta.uid.as_deref().unwrap_or_default()
}

/// The names of the objects of a kind in the texts of its page.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nouns {
    /// The title of the page, for example `Pods`.
    pub(crate) title: &'static str,
    /// One object, for example `pod`.
    pub(crate) one: &'static str,
    /// Some objects, for example `pods`.
    pub(crate) many: &'static str,
}

impl Nouns {
    /// "1 pod", "1,200 pods".
    pub(crate) fn count(&self, count: u64) -> String {
        let noun = if count == 1 { self.one } else { self.many };
        format!("{} {noun}", format::thousands(count))
    }
}

/// What a row shows of its cluster.
pub(crate) struct ClusterMark {
    pub(crate) color: Color32,
    /// Set for a cluster whose store is not live: since when its rows are old.
    pub(crate) old: Option<String>,
}

/// What a cell needs at its paint that is not in its row.
pub(crate) struct RowScene {
    pub(crate) palette: &'static Palette,
    pub(crate) clock: Clock,
    pub(crate) marks: HashMap<ClusterId, ClusterMark>,
}

/// The mark of a row whose cluster left the selection. The row goes with the next rows.
const NOT_SELECTED: &str = "old: the cluster is not selected";

impl RowScene {
    /// Since when the rows of a cluster are old. `None` for current rows.
    pub(crate) fn old(&self, cluster: &ClusterId) -> Option<&str> {
        self.marks.get(cluster).map_or(Some(NOT_SELECTED), |mark| mark.old.as_deref())
    }

    /// The color of a cluster.
    pub(crate) fn color(&self, cluster: &ClusterId) -> Color32 {
        self.marks.get(cluster).map_or(self.palette.text_faint, |mark| mark.color)
    }
}

/// What a kind gives to its page: the model of its table, its nouns and the object of a
/// row. The Pods have a model of their own; each other kind has the model of its columns.
pub(crate) trait ObjectModel:
    Model<Data = Stores<Self::Object>, Filter = ObjectFilter, Summary = Namespaces, Scene = RowScene>
{
    type Object: Describe;

    fn nouns(&self) -> Nouns;

    /// The object of a row.
    fn object<'a>(&self, row: &'a Self::Row) -> &'a Arc<Self::Object>;
}
