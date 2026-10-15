//! The table of each kind (plan sections 11.2 and 11.4): the columns of the description of
//! the kind in `fresnel-core`, with the Cluster column after the name and the namespace,
//! and the CPU and memory columns after the age for a kind with metrics (`views/metrics`).
//! The Pods have a model of their own (`views/pods`), with the status of each pod.
//!
//! The worker computes the value of each column of a row one time, so a sort compares
//! values and reads no object, and a frame lays out the texts of the rows on the screen.
//!
//! - `cells`: the text and the tone of each value.

mod cells;

use std::cmp::Ordering;
use std::marker::PhantomData;
use std::sync::Arc;

use fresnel_core::kinds::{self, Describe, Kind, Object, ObjectHealth, Value, Visit};
use fresnel_core::net::task::Tasks;

use crate::views::metrics::gauge::{Resource, Usage};
use crate::views::metrics::{self, has_columns};
use crate::views::objects::{
    NamespaceSet, Namespaces, Nouns, ObjectFilter, ObjectModel, ObjectPage, Page, RowScene, Stores, uid,
};
use crate::views::pods::PodModel;
use crate::widgets::table::model::{Cell, Column, Computed, Model, ObjectId, contains_ignore_case};
use crate::widgets::worker::{Cancel, Wake};

/// The count of objects between two reads of the cancel mark.
const CANCEL_STEP: usize = 1024;
/// The width of the Cluster column.
const CLUSTER_WIDTH: f32 = 120.0;

/// The page of a kind: the Pods with their model, each other kind with the model of its
/// columns. It starts the worker thread of the table.
pub(crate) fn page(kind: Kind, tasks: &Tasks, wake: Wake) -> Box<dyn Page> {
    kind.visit(NewPage { tasks, wake })
}

struct NewPage<'a> {
    tasks: &'a Tasks,
    wake: Wake,
}

impl Visit for NewPage<'_> {
    type Output = Box<dyn Page>;

    fn visit<K: Describe>(self) -> Box<dyn Page> {
        if K::ID == Kind::Pod {
            return Box::new(ObjectPage::new(self.tasks, self.wake, PodModel));
        }
        Box::new(ObjectPage::new(self.tasks, self.wake, KindModel::<K>::new()))
    }
}

/// What a column of the table shows.
#[derive(Clone, Copy, Debug)]
enum Field {
    Cluster,
    /// The column of the description of the kind at this place.
    Value(usize),
    Usage(Resource),
}

/// One row: an object with its identity, its health value, the value of each column of its
/// kind and its metrics.
pub(crate) struct KindRow<K> {
    id: ObjectId,
    object: Arc<K>,
    health: ObjectHealth,
    values: Vec<Value<'static>>,
    usage: Option<Usage>,
}

pub(crate) struct KindModel<K: Describe> {
    columns: Vec<Column>,
    fields: Vec<Field>,
    of_kind: Vec<kinds::Column<K>>,
    kind: PhantomData<K>,
}

impl<K: Describe> KindModel<K> {
    pub(crate) fn new() -> Self {
        let of_kind = kinds::columns::<K>();
        // The Cluster column follows the name, and the namespace of a kind with namespaces.
        let cluster_at = if K::ID.info().namespaced { 2 } else { 1 };
        let (mut columns, mut fields) = (Vec::new(), Vec::new());
        for (index, column) in of_kind.iter().enumerate() {
            if index == cluster_at {
                columns.push(Column::new("Cluster", CLUSTER_WIDTH));
                fields.push(Field::Cluster);
            }
            let mut shown = Column::new(column.title, column.width);
            shown = if column.numeric { shown.numeric() } else { shown };
            columns.push(if column.shown { shown } else { shown.hidden() });
            fields.push(Field::Value(index));
        }
        if has_columns(K::ID) {
            columns.extend(Resource::BOTH.map(Resource::column));
            fields.extend(Resource::BOTH.map(Field::Usage));
        }
        Self { columns, fields, of_kind, kind: PhantomData }
    }

    /// True when a text of the row has `needle` in it: the name, the namespace, the cluster
    /// or a text of a column.
    fn matches(row: &KindRow<K>, needle: &str) -> bool {
        let id = [&*row.id.key.name, &*row.id.key.namespace, row.id.cluster.as_str()];
        id.iter().any(|text| contains_ignore_case(text, needle))
            || row.values.iter().any(|value| match value {
                Value::Text(text) => contains_ignore_case(text, needle),
                Value::List(texts) => texts.iter().any(|text| contains_ignore_case(text, needle)),
                _ => false,
            })
    }
}

impl<K: Describe> Model for KindModel<K> {
    type Data = Stores<K>;
    type Filter = ObjectFilter;
    type Row = KindRow<K>;
    type Summary = Namespaces;
    type Scene = RowScene;

    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn rows(
        &self,
        data: &Stores<K>,
        filter: &ObjectFilter,
        cancel: &Cancel<'_>,
    ) -> Option<Computed<KindRow<K>, Namespaces>> {
        let needle = filter.needle();
        let (mut rows, mut total, mut namespaces) = (Vec::new(), 0, NamespaceSet::default());
        for (cluster, snapshot) in &data.clusters {
            total += snapshot.items.len();
            let metered = data.metered(cluster);
            for (count, (key, object)) in snapshot.items.iter().enumerate() {
                if count.is_multiple_of(CANCEL_STEP) && cancel.is_set() {
                    return None;
                }
                namespaces.add(key);
                let health = object.health();
                if !filter.passes(key, health) {
                    continue;
                }
                let values = self.of_kind.iter().map(|column| (column.read)(object).into_owned()).collect();
                let id = ObjectId { cluster: cluster.clone(), key: key.clone(), uid: uid(object.meta()).into() };
                let usage = metered.and_then(|metered| metered.usage(key, &**object));
                let row = KindRow { id, object: Arc::clone(object), health, values, usage };
                if Self::matches(&row, &needle) {
                    rows.push(row);
                }
            }
        }
        Some(Computed { rows, total, summary: namespaces.done() })
    }

    fn compare(&self, a: &KindRow<K>, b: &KindRow<K>, column: usize) -> Ordering {
        match self.fields.get(column) {
            Some(Field::Cluster) => a.id.cluster.cmp(&b.id.cluster),
            Some(Field::Value(index)) => a.values.get(*index).cmp(&b.values.get(*index)),
            Some(Field::Usage(resource)) => metrics::cells::compare(a.usage.as_ref(), b.usage.as_ref(), *resource),
            None => Ordering::Equal,
        }
    }

    fn tie(&self, a: &KindRow<K>, b: &KindRow<K>) -> Ordering {
        a.id.key.cmp(&b.id.key).then_with(|| a.id.cluster.cmp(&b.id.cluster))
    }

    fn id<'a>(&self, row: &'a KindRow<K>) -> &'a ObjectId {
        &row.id
    }

    fn has(&self, data: &Stores<K>, row: &KindRow<K>) -> bool {
        data.has(&row.id)
    }

    fn cell<'a>(&self, row: &'a KindRow<K>, column: usize, scene: &'a RowScene) -> Cell<'a> {
        let old = scene.old(&row.id.cluster);
        let cell = match self.fields.get(column) {
            Some(Field::Cluster) => cells::cluster(&row.id.cluster, scene),
            Some(Field::Value(0)) => cells::name(&row.id.key.name, row.health, old, scene),
            Some(Field::Value(index)) => {
                let namespace = *index == 1 && K::ID.info().namespaced;
                cells::value(row.values.get(*index).unwrap_or(&Value::None), namespace, &scene.clock)
            }
            Some(Field::Usage(resource)) => {
                let problem = scene.problem(&row.id.cluster, K::ID);
                metrics::cells::cell(row.usage.as_ref(), *resource, problem, &scene.clock)
            }
            None => Cell::new(""),
        };
        cells::of_old_row(cell, old)
    }
}

impl<K: Describe> ObjectModel for KindModel<K> {
    type Object = K;

    fn nouns(&self) -> Nouns {
        Nouns { title: K::TITLE, one: K::KIND, many: K::TITLE }
    }

    fn object<'a>(&self, row: &'a KindRow<K>) -> &'a Arc<K> {
        &row.object
    }
}

#[cfg(test)]
mod tests;
