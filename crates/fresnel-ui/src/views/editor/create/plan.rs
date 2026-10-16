//! The orders of a text of documents for a target cluster and namespace, and the jobs of the
//! last run of a tab.
//!
//! Each document of the text is one order: a create, an apply or an apply with force. A
//! document that cannot go, such as one of an other namespace, is a refusal with its reason,
//! and the others go, as in kubectl. The jobs of a run start in the sequence of its orders
//! with numbers above the last job of the view at the click, so the tab finds each one.

use std::sync::Arc;

use fresnel_core::app::{CoreView, JobView};
use fresnel_core::cluster::ClusterId;
use fresnel_core::manifest::documents::{Document, Entry};
use fresnel_core::ops::{Operation, Order, Target};
use fresnel_core::store::ObjectKey;

/// What a run does with each document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Way {
    Create,
    Apply,
    /// An apply that takes the fields of other managers.
    Force,
}

impl Way {
    pub(super) fn operation(self, document: Document) -> Operation {
        match self {
            Self::Create => Operation::Create(document),
            Self::Apply => Operation::Apply { document, force: false },
            Self::Force => Operation::Apply { document, force: true },
        }
    }

    /// "Create", "Apply", "Apply with force": the verb of a button.
    pub(super) fn verb(self) -> &'static str {
        match self {
            Self::Create => "Create",
            Self::Apply => "Apply",
            Self::Force => "Apply with force",
        }
    }
}

/// One document of a plan: its words, and its order or why it does not go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Item {
    /// "ConfigMap shop/one", or "Document 3" when it has no kind or name.
    pub(super) label: String,
    pub(super) order: Result<Order, String>,
}

impl Item {
    /// The item with a force in place of its apply.
    pub(super) fn forced(self) -> Self {
        let order = self.order.map(|order| match order.operation {
            Operation::Apply { document, .. } => Order { operation: Way::Force.operation(document), ..order },
            _ => order,
        });
        Self { order, ..self }
    }
}

/// The orders of the documents of a text.
pub(super) fn plan(entries: &[Entry], (cluster, namespace): (&ClusterId, &str), way: Way) -> Vec<Item> {
    let item = |(index, entry): (usize, &Entry)| match entry {
        Ok(document) => {
            let order = document.namespace_in(namespace).and_then(|namespace| {
                let key = ObjectKey::new(&namespace, &document.name);
                let target = Target { cluster: cluster.clone(), kind: document.kind, key, uid: "".into() };
                Order::new(target, way.operation(document.clone())).map_err(|error| error.to_string())
            });
            let label = match &order {
                Ok(order) => label(&order.target),
                Err(_) => format!("{} {}", document.kind.info().name, document.name),
            };
            Item { label, order }
        }
        Err(reason) => Item { label: format!("Document {}", index + 1), order: Err(reason.clone()) },
    };
    entries.iter().enumerate().map(item).collect()
}

/// "ConfigMap shop/one", "Namespace shop".
pub(super) fn label(target: &Target) -> String {
    let key = &target.key;
    let name = if key.namespace.is_empty() { key.name.to_string() } else { format!("{}/{}", key.namespace, key.name) };
    format!("{} {name}", target.kind.info().name)
}

/// The last run of a tab: what went, and what did not.
#[derive(Clone, Debug)]
pub(super) struct Run {
    pub(super) way: Way,
    /// The target cluster and namespace of the run. A force of its apply goes there, also
    /// when the bar of the tab names an other target now.
    pub(super) target: (ClusterId, String),
    pub(super) items: Vec<Item>,
    /// The number of the last job of the view at the click.
    after: u64,
}

impl Run {
    pub(super) fn new(way: Way, (target, items): ((ClusterId, String), Vec<Item>), view: &CoreView) -> Self {
        let after = view.jobs.jobs.last().map_or(0, |job| job.number);
        Self { way, target, items, after }
    }

    /// The orders of the run.
    pub(super) fn orders(&self) -> Vec<Order> {
        self.items.iter().filter_map(|item| item.order.clone().ok()).collect()
    }

    /// The job of each item in the view, in the sequence of the items: `None` for a refused
    /// document and for a job that the view does not have yet, or not any more.
    pub(super) fn jobs(&self, view: &CoreView) -> Vec<Option<Arc<JobView>>> {
        let mut from = self.after;
        let mut found = Vec::new();
        for item in &self.items {
            let job =
                item.order.as_ref().ok().and_then(|order| {
                    view.jobs.jobs.iter().find(|job| job.number > from && job.order == *order).cloned()
                });
            if let Some(job) = &job {
                from = job.number;
            }
            found.push(job);
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use fresnel_core::kinds::Kind;
    use fresnel_core::manifest::documents::documents;

    use super::*;

    #[test]
    fn each_document_is_an_order_in_the_target_and_a_document_that_cannot_go_tells_why() {
        let text = "apiVersion: v1\nkind: Namespace\nmetadata: {name: shop}\n---\napiVersion: v1\nkind: ConfigMap\n\
                    metadata: {name: one}\n---\napiVersion: v1\nkind: Secret\nmetadata: {name: two, namespace: web}\n\
                    ---\nkind: Pod\n";
        let entries = documents(text).expect("valid YAML");
        let items = plan(&entries, (&ClusterId::new("ams"), "shop"), Way::Apply);
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, ["Namespace shop", "ConfigMap shop/one", "Secret two", "Document 4"]);
        let order = items[1].order.as_ref().expect("an order");
        assert_eq!((order.target.cluster.as_str(), order.target.kind), ("ams", Kind::ConfigMap));
        assert!(matches!(&order.operation, Operation::Apply { force: false, .. }));
        let refusals: Vec<&str> =
            items.iter().filter_map(|item| item.order.as_ref().err()).map(String::as_str).collect();
        assert_eq!(
            refusals,
            ["Its namespace web is not the target namespace shop. Choose web to send it.", "It has no apiVersion."]
        );
    }
}
