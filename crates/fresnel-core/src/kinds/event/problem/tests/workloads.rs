//! The rules of a controller that could not make a pod, and of a Job that failed. The
//! readiness of the pods of a controller is an other problem: it does not count.

use super::*;
use crate::fixture::condition;

fn failed_create<K: Describe>(name: &str) -> Event {
    warning::<K>(name, "FailedCreate", "2026-10-13T17:58:20Z", None)
}

#[test]
fn a_replica_set_that_could_not_make_a_pod_is_ongoing_until_it_has_each_one() {
    let bed = Bed::new();
    let witness = bed.scene(&fixture::all(), &fixture::all());
    let made = state(Resolved(None), "the ReplicaSet has 1 of 1 pods");
    let broken = failed_create::<ReplicaSet>("broken-6bf768c954");
    assert_eq!(judged(&witness, &broken), made, "its one pod is not ready, which is an other problem");
    let short = bed.with::<ReplicaSet>("broken-6bf768c954", |set| set["status"]["replicas"] = 0.into());
    assert_eq!(judged(&short, &broken), state(Ongoing, "the ReplicaSet has 0 of 1 pods"));
    let failure = bed.with::<ReplicaSet>("broken-6bf768c954", |set| {
        let failure = condition(set, "ReplicaFailure");
        failure["reason"] = "FailedCreate".into();
    });
    let failing = state(Ongoing, "the ReplicaSet has the condition ReplicaFailure: FailedCreate");
    assert_eq!(judged(&failure, &broken), failing);
    let scaled = bed.with::<ReplicaSet>("broken-6bf768c954", |set| set["metadata"]["generation"] = 2.into());
    let old = state(Unknown, "the status of the ReplicaSet is not of its newest spec");
    assert_eq!(judged(&scaled, &broken), old, "a count of an other spec");
}

#[test]
fn a_stateful_set_or_a_daemon_set_is_judged_by_the_pods_that_it_made() {
    let bed = Bed::new();
    let witness = bed.scene(&fixture::all(), &fixture::all());
    let db = failed_create::<StatefulSet>("db");
    assert_eq!(judged(&witness, &db), state(Resolved(None), "the StatefulSet has 2 of 2 pods"));
    let update = bed.with::<StatefulSet>("db", |set| {
        (set["status"]["updateRevision"], set["status"]["updatedReplicas"]) = ("db-next".into(), 1.into());
    });
    let made = state(Resolved(None), "the StatefulSet has 2 of 2 pods");
    assert_eq!(judged(&update, &db), made, "an update of its pods makes no new problem of a create");
    let short = bed.with::<StatefulSet>("db", |set| set["status"]["replicas"] = 1.into());
    assert_eq!(judged(&short, &db), state(Ongoing, "the StatefulSet has 1 of 2 pods"));

    let agent = failed_create::<DaemonSet>("agent");
    assert_eq!(judged(&witness, &agent), state(Resolved(None), "the DaemonSet has 1 of 1 nodes with its pod"));
    let short = bed.with::<DaemonSet>("agent", |set| set["status"]["currentNumberScheduled"] = 0.into());
    assert_eq!(judged(&short, &agent), state(Ongoing, "the DaemonSet has 0 of 1 nodes with its pod"));
}

/// A Job runs with fewer pods also in its wait after a failed pod, so only a Job with each
/// pod that it wants, or a complete one, tells an end.
#[test]
fn a_job_that_could_not_make_a_pod_ends_with_its_pods_and_a_short_count_tells_nothing() {
    let bed = Bed::new();
    let witness = bed.scene(&fixture::all(), &fixture::all());
    assert_eq!(judged(&witness, &failed_create::<Job>("once")), state(Resolved(None), "the Job is complete"));
    let suspended = state(Unknown, "the Job is suspended, so it makes no pod now");
    assert_eq!(judged(&witness, &failed_create::<Job>("loop-b")), suspended);
    let failed = state(Unknown, "the Job failed, so it makes no pod");
    assert_eq!(judged(&witness, &failed_create::<Job>("fails")), failed);
    let running = |status: Value| bed.with::<Job>("once", |job| job["status"] = status);
    let runs = running(json!({"active": 1, "ready": 1}));
    let event = failed_create::<Job>("once");
    assert_eq!(judged(&runs, &event), state(Resolved(None), "the Job runs 1 of 1 pods"));
    let waits = running(json!({"failed": 1}));
    let short = state(Unknown, "the Job runs 0 of 1 pods: a wait after a failed pod looks the same");
    assert_eq!(judged(&waits, &event), short);
}

#[test]
fn a_job_that_failed_for_good_keeps_its_condition() {
    let bed = Bed::new();
    let witness = bed.scene(&fixture::all(), &fixture::all());
    let limit = warning::<Job>("fails", "BackoffLimitExceeded", "2026-10-13T17:58:27Z", None);
    assert_eq!(judged(&witness, &limit), state(Ongoing, "the Job failed: BackoffLimitExceeded"));
    let deadline = warning::<Job>("once", "DeadlineExceeded", "2026-10-13T17:58:27Z", None);
    assert_eq!(judged(&witness, &deadline), state(Unknown, "the Job has no condition Failed"));
}
