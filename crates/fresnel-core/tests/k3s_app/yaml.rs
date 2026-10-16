//! The create and the apply of a text of documents through `Command::RunInTurn`, as the YAML
//! tab sends them (plan section 12.9 and the acceptance of M11): each document is one job in
//! the target cluster and namespace, each job waits for the one before it, and a conflict of
//! a server-side apply names its fields until a force takes them.

use std::sync::Arc;

use fresnel_core::app::{Command, CoreView, JobView};
use fresnel_core::cluster::ClusterId;
use fresnel_core::manifest::documents::documents;
use fresnel_core::ops::{JobState, Operation, Order, StepState, Target};
use fresnel_core::store::ObjectKey;

use crate::bed::Bed;
use crate::scene::kubectl;

/// The target namespace of the text.
const EXTRA: &str = "extra";

/// The orders of a text for cluster b and the namespace `extra`, as the YAML tab makes them.
fn orders(text: &str, operation: impl Fn(fresnel_core::manifest::documents::Document) -> Operation) -> Vec<Order> {
    let entries = documents(text).expect("valid YAML");
    let documents = entries.into_iter().map(|entry| entry.expect("an object of a kind of the app"));
    let order = |document: fresnel_core::manifest::documents::Document| {
        let namespace = document.namespace_in(EXTRA).expect("the target namespace");
        let key = ObjectKey::new(&namespace, &document.name);
        let target = Target { cluster: ClusterId::new("b"), kind: document.kind, key, uid: "".into() };
        Order::new(target, operation(document)).expect("an order")
    };
    documents.map(order).collect()
}

/// Sends the orders as one `Command::RunInTurn` and waits for the end of each job.
async fn run(bed: &Bed, orders: Vec<Order>) -> Vec<Arc<JobView>> {
    let first = bed.core.view().jobs.jobs.last().map_or(1, |job| job.number + 1);
    let count = orders.len();
    bed.core.send(Command::RunInTurn(orders));
    let mine = |view: &CoreView| -> Vec<Arc<JobView>> {
        view.jobs.jobs.iter().filter(|job| job.number >= first).cloned().collect()
    };
    let view = bed
        .until("the end of each job of the text", |view| {
            let jobs = mine(view);
            jobs.len() == count && jobs.iter().all(|job| job.report.state.is_final())
        })
        .await;
    mine(&view)
}

/// A text with a Namespace and three objects in it: the create of each one goes in the
/// sequence of the text, after the end of the one before.
pub(crate) async fn a_file_with_many_documents_creates_each_object(bed: &Bed) {
    let text = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: extra\n---\napiVersion: v1\nkind: ConfigMap\n\
                metadata:\n  name: one\ndata:\n  mode: fast\n---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: two\n  \
                namespace: extra\nstringData:\n  token: t-1\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  \
                name: three\nspec:\n  selector:\n    matchLabels: {app: three}\n  template:\n    metadata:\n      \
                labels: {app: three}\n    spec:\n      containers:\n      - {name: pause, image: rancher/mirrored-pause:3.6}\n";
    let jobs = run(bed, orders(text, Operation::Create)).await;
    let states: Vec<&JobState> = jobs.iter().map(|job| &job.report.state).collect();
    assert_eq!(states, [&JobState::Done; 4], "{jobs:#?}");
    for pair in jobs.windows(2) {
        let wait = format!("wait for the end of job {}", pair[0].number);
        let first = pair[1].report.steps.first().map(|step| (step.text.as_str(), &step.state));
        let ended = StepState::Done(format!("job {} ended", pair[0].number));
        assert_eq!(first, Some((wait.as_str(), &ended)), "each job waits for the one before it");
    }
    let objects = ["configmap/one", "secret/two", "deployment/three"];
    let names = kubectl(&bed.b, &[&["get"][..], &objects, &["--namespace", EXTRA, "--output", "name"]].concat()).await;
    assert_eq!(names.lines().collect::<Vec<_>>(), ["configmap/one", "secret/two", "deployment.apps/three"]);
    let in_a = kubectl(&bed.a, &["get", "namespaces", "--output", "name"]).await;
    assert!(!in_a.lines().any(|line| line == "namespace/extra"), "cluster a gets no object");
}

/// A server-side apply makes a new object; an apply of a field that kubectl set is a
/// conflict that names the field and its manager, and the apply with force takes it.
pub(crate) async fn an_apply_conflict_names_its_fields_and_a_force_takes_them(bed: &Bed) {
    let create = ["create", "configmap", "applied", "--namespace", EXTRA, "--from-literal=mode=slow"];
    kubectl(&bed.b, &create).await;
    let text = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: applied\ndata:\n  mode: fast\n---\napiVersion: v1\n\
                kind: ConfigMap\nmetadata:\n  name: fresh\ndata:\n  mode: new\n";
    let jobs = run(bed, orders(text, |document| Operation::Apply { document, force: false })).await;
    let JobState::Failed(error) = &jobs[0].report.state else { panic!("a conflict: {jobs:#?}") };
    assert_eq!(error.status(), Some(409), "{error}");
    let fields: Vec<&str> = error.apply_conflicts().collect();
    assert_eq!(fields.len(), 1, "{fields:?}");
    assert!(fields[0].starts_with(".data.mode (conflict with \"kubectl-create\""), "{fields:?}");
    assert_eq!(jobs[1].report.state, JobState::Done, "the next document goes, as in kubectl");
    let get = |name: &'static str| async move {
        kubectl(&bed.b, &["get", "configmap", name, "--namespace", EXTRA, "--output", "jsonpath={.data.mode}"]).await
    };
    assert_eq!((get("applied").await, get("fresh").await), ("slow".to_owned(), "new".to_owned()));

    let force = orders(text, |document| Operation::Apply { document, force: true });
    let jobs = run(bed, force.into_iter().take(1).collect()).await;
    assert_eq!(jobs[0].report.state, JobState::Done, "{jobs:#?}");
    assert_eq!(get("applied").await, "fast");
    let managers = "jsonpath={.metadata.managedFields[*].manager}";
    let managers = kubectl(&bed.b, &["get", "configmap", "applied", "--namespace", EXTRA, "--output", managers]).await;
    assert!(managers.split_whitespace().any(|manager| manager == "fresnel"), "{managers}");
}
