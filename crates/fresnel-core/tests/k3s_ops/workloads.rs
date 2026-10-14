//! Scale, restart, rollout status, history and rollback: each operation of the app on an
//! object gives the state that kubectl gives its twin.

use fresnel_core::kinds::Kind;
use fresnel_core::ops::{JobState, Operation, Order, Outcome, Report, Rollout, StepState};
use k8s_openapi::api::apps::v1::Deployment;
use tokio::time::timeout;

use crate::bed::{APP, Cluster, KUBECTL, LIMIT, resource};
use crate::common::in_time;
use crate::scene::revision_numbers;

const WORKLOADS: [(Kind, &str); 3] = [(Kind::Deployment, "web"), (Kind::StatefulSet, "db"), (Kind::DaemonSet, "agent")];

fn object(kind: Kind, name: &str) -> String {
    format!("{}/{name}", resource(kind))
}

fn done(report: &Report) {
    assert_eq!(report.state, JobState::Done, "the job: {report:#?}");
}

/// `kubectl scale` with the same count, and the mark "change pending" until the watch event.
pub(crate) async fn scale(b: &Cluster) {
    let scaled = [
        (Kind::Deployment, "web"),
        (Kind::StatefulSet, "db"),
        (Kind::ReplicaSet, "set"),
        (Kind::ReplicationController, "legacy"),
    ];
    for (kind, name) in scaled {
        let report = b.run(kind, name, Operation::Scale(2)).await;
        done(&report);
        b.kubectl(&["scale", &object(kind, name), "--replicas", "2", "--namespace", KUBECTL]).await;
        b.same_as_kubectl(kind, name).await;
        let Outcome::Written(Some(written)) = &report.outcome else { panic!("the answer of the write: {report:#?}") };
        if kind == Kind::Deployment {
            let mut reader = b.stores.stores.full::<Deployment>().expect("the store of the Deployments");
            let seen = async {
                while reader.read().is_pending(written) {
                    reader.changed().await;
                }
            };
            timeout(LIMIT, seen).await.expect("the watch event of the write ends the mark");
        }
    }
}

/// `kubectl rollout restart`, and its refusal of a paused Deployment.
pub(crate) async fn restart(b: &Cluster) {
    for (kind, name) in WORKLOADS {
        done(&b.run(kind, name, Operation::Restart).await);
        b.kubectl(&["rollout", "restart", &object(kind, name), "--namespace", KUBECTL]).await;
        b.same_as_kubectl(kind, name).await;
    }
    let report = b.run(Kind::Deployment, "paused", Operation::Restart).await;
    let text = "can't restart paused deployment (run rollout resume first)";
    assert_eq!(report.state.error().map(ToString::to_string).as_deref(), Some(text));
    let refused = b.k3s.kubectl(&["rollout", "restart", "deployments.apps/paused", "--namespace", KUBECTL]).await;
    assert!(refused.expect_err("kubectl refuses it too").to_string().contains(text));
    b.same_as_kubectl(Kind::Deployment, "paused").await;
}

/// The last text of the rollout status is the last line of `kubectl rollout status`, and a
/// rollout that cannot complete waits until the cancel.
pub(crate) async fn rollout_status(b: &Cluster) {
    for (kind, name) in WORKLOADS {
        let report = b.run(kind, name, Operation::RolloutStatus).await;
        done(&report);
        let Outcome::Rollout(Rollout::Complete(text)) = &report.outcome else { panic!("complete: {report:#?}") };
        let kubectl = b.kubectl(&["rollout", "status", &object(kind, name), "--namespace", APP]).await;
        assert_eq!(Some(text.as_str()), kubectl.lines().last(), "{name}");
    }

    let order = Order::new(b.target(Kind::Deployment, "stuck").await, Operation::RolloutStatus).expect("an order");
    let (mut watched, run) = b.watch(order);
    let wait = async move {
        while let Some(report) = watched.reports.recv().await {
            if matches!(report.outcome, Outcome::Rollout(Rollout::Waiting(_))) {
                watched.cancel.send_replace(true);
                return watched;
            }
        }
        panic!("the job ended before it waited");
    };
    let (state, watched) = in_time("the cancel ends the job", async { tokio::join!(run, wait) }).await;
    assert_eq!(state, JobState::Cancelled);
    drop(watched);
}

/// The revisions are the ones of `kubectl rollout history`, with the newest as the current one.
pub(crate) async fn history(b: &Cluster) {
    for (kind, name) in WORKLOADS {
        let report = b.run(kind, name, Operation::History).await;
        done(&report);
        let Outcome::Revisions(revisions) = &report.outcome else { panic!("revisions: {report:#?}") };
        let numbers: Vec<i64> = revisions.iter().map(|revision| revision.number).collect();
        assert_eq!(numbers, revision_numbers(b, APP, kind, name).await, "{name}");
        let current: Vec<bool> = revisions.iter().map(|revision| revision.current).collect();
        let mut newest = vec![false; current.len()];
        if let Some(last) = newest.last_mut() {
            *last = true;
        }
        assert_eq!(current, newest, "{name}: the template of now is the newest revision");
    }
}

/// `kubectl rollout undo`, to the revision before and to revision 1, and the skip of a
/// rollback to the template of now.
pub(crate) async fn rollback(b: &Cluster) {
    for (kind, name) in WORKLOADS {
        done(&b.run(kind, name, Operation::Rollback(0)).await);
        b.kubectl(&["rollout", "undo", &object(kind, name), "--namespace", KUBECTL]).await;
        b.same_as_kubectl(kind, name).await;

        done(&b.run(kind, name, Operation::Rollback(1)).await);
        b.kubectl(&["rollout", "undo", &object(kind, name), "--to-revision", "1", "--namespace", KUBECTL]).await;
        b.same_as_kubectl(kind, name).await;

        let newest = *revision_numbers(b, APP, kind, name).await.last().expect("a revision");
        let report = b.run(kind, name, Operation::Rollback(newest)).await;
        done(&report);
        let skipped = format!("skipped rollback (current template already matches revision {newest})");
        assert_eq!(report.steps.last().map(|step| &step.state), Some(&StepState::Done(skipped)), "{name}");
        let (revision, object) = (newest.to_string(), object(kind, name));
        let kubectl = b.kubectl(&["rollout", "undo", &object, "--to-revision", &revision, "--namespace", APP]).await;
        assert!(kubectl.contains("skipped rollback"), "kubectl skips it too");
    }
    let report = b.run(Kind::Deployment, "web", Operation::Rollback(99)).await;
    let refusal = report.state.error().map(ToString::to_string);
    assert_eq!(refusal.as_deref(), Some("unable to find specified revision 99 in history"));
}
