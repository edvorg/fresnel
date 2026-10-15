//! The operations of a view through `Command::Run`: each order runs as a job in the cluster
//! of its object to its final state, with the mark "change pending" until the watch event
//! of its write. `Command::Cancel` ends a rollout status that waits.

use fresnel_core::app::Command;
use fresnel_core::kinds::Describe;
use fresnel_core::ops::{JobState, Operation, Order, Outcome, Propagation, Rollout, StepState, Target, trigger_name};
use fresnel_core::published::Held;
use fresnel_core::store::{ObjectKey, Snapshot};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::batch::v1::{CronJob, Job};

use crate::bed::{Bed, job, snapshot, written};
use crate::scene::{APP, field};

fn order(target: Target, operation: Operation) -> Order {
    Order::new(target, operation).expect("an order")
}

/// The store of a kind in a cluster at this moment: the store of before a command.
fn store<K: Describe>(bed: &Bed, cluster: &str) -> Held<Snapshot<K>> {
    snapshot::<K>(&bed.core.view(), cluster).expect("a store of the kind")
}

/// One command with an order for `web` in each cluster: each job scales the object of its
/// own cluster to the count of its order.
pub(crate) async fn scale_in_each_cluster(bed: &Bed) {
    let before = [store::<Deployment>(bed, "a"), store::<Deployment>(bed, "b")];
    let orders = vec![
        order(bed.target::<Deployment>("a", "web"), Operation::Scale(3)),
        order(bed.target::<Deployment>("b", "web"), Operation::Scale(2)),
    ];
    let jobs = bed.run(orders).await;
    let expected = [("a", &bed.a, "3"), ("b", &bed.b, "2")];
    for ((job, before), (cluster, k3s, replicas)) in jobs.iter().zip(&before).zip(expected) {
        bed.mark::<Deployment>(cluster, before, written(job)).await;
        assert_eq!(field(k3s, "deployment/web", "{.spec.replicas}").await, replicas, "web in {cluster}");
    }
}

/// `kubectl rollout undo`: `web` gets the template of its first revision, with no variable.
pub(crate) async fn rollback(bed: &Bed) {
    let before = store::<Deployment>(bed, "b");
    let job = bed.run_one(order(bed.target::<Deployment>("b", "web"), Operation::Rollback(0))).await;
    bed.mark::<Deployment>("b", &before, written(&job)).await;
    let env = "{.spec.template.spec.containers[0].env}";
    assert_eq!(field(&bed.b, "deployment/web", env).await, "", "the first revision has no variable");
}

/// `kubectl rollout restart`: the template gets the time of the restart.
pub(crate) async fn restart(bed: &Bed) {
    let before = store::<Deployment>(bed, "b");
    let job = bed.run_one(order(bed.target::<Deployment>("b", "web"), Operation::Restart)).await;
    bed.mark::<Deployment>("b", &before, written(&job)).await;
    let at = r"{.spec.template.metadata.annotations.kubectl\.kubernetes\.io/restartedAt}";
    assert!(!field(&bed.b, "deployment/web", at).await.is_empty(), "the time of the restart in the template");
}

/// A delete: the row keeps the mark until the DELETED event, and then it goes.
pub(crate) async fn delete(bed: &Bed) {
    let before = store::<Deployment>(bed, "b");
    let delete = Operation::Delete(Propagation::Background);
    let job = bed.run_one(order(bed.target::<Deployment>("b", "old"), delete)).await;
    let written = written(&job);
    bed.mark::<Deployment>("b", &before, written).await;
    assert!(store::<Deployment>(bed, "b").items.get(written.key()).is_none(), "the row of old goes");
    assert_eq!(field(&bed.b, "deployment/old", "{.metadata.name}").await, "", "old is gone");
}

/// "Trigger now" of a CronJob: the write is the new Job, which has no row and thus no mark
/// until its watch event brings its row.
pub(crate) async fn trigger(bed: &Bed) {
    let name = trigger_name("nightly");
    let before = store::<Job>(bed, "b");
    let job = bed.run_one(order(bed.target::<CronJob>("b", "nightly"), Operation::Trigger(name.clone()))).await;
    let written = written(&job);
    assert_eq!(written.key(), &ObjectKey::new(APP, &name), "the write is the new Job");
    assert!(!before.is_pending(written), "no row, no mark");
    bed.until("the row of the new Job", |view| {
        snapshot::<Job>(view, "b")
            .is_some_and(|jobs| jobs.items.get(written.key()).is_some() && !jobs.is_pending(written))
    })
    .await;
    let owner = field(&bed.b, &format!("job/{name}"), "{.metadata.ownerReferences[0].name}").await;
    assert_eq!(owner, "nightly", "the CronJob owns the new Job");
}

/// The rollout status of `stuck` waits for pods that never start, until `Command::Cancel`.
pub(crate) async fn cancel_ends_a_rollout_status(bed: &Bed) {
    let number = bed.send(vec![order(bed.target::<Deployment>("b", "stuck"), Operation::RolloutStatus)]);
    bed.until("the rollout status waits", |view| {
        job(view, number).is_some_and(|job| matches!(job.report.outcome, Outcome::Rollout(Rollout::Waiting(_))))
    })
    .await;
    bed.core.send(Command::Cancel(number));
    let view =
        bed.until("the end of the job", |view| job(view, number).is_some_and(|job| job.report.state.is_final())).await;
    let report = &job(&view, number).expect("the job").report;
    assert_eq!(report.state, JobState::Cancelled, "{report:#?}");
    let follow = report.steps.iter().map(|step| (step.text.as_str(), &step.state)).collect::<Vec<_>>();
    assert_eq!(follow, [("follow the rollout", &StepState::Stopped)]);
}
