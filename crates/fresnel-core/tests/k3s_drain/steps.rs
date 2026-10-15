//! The steps of the test. Each drain of the app on the node `drained` runs with the same
//! options as `kubectl drain` on the node `twin`, and the two nodes end with the same pods:
//! the same labels, as the names of the pods of a Deployment differ.

use std::collections::BTreeSet;
use std::time::Duration;

use fresnel_core::app::{Command, CoreView, JobView};
use fresnel_core::ops::drain::{Drain, DrainOptions, PodState, Verdict};
use fresnel_core::ops::{JobState, Operation, Order, Outcome};
use tokio::time::Instant;

use crate::bed::{Bed, Cluster, DRAINED, TWIN, job};

/// The options that let each pod of the scene go: the pod of the DaemonSet stays.
const EACH: DrainOptions = DrainOptions {
    ignore_daemon_sets: true,
    delete_empty_dir_data: true,
    force: true,
    grace_period: None,
    timeout: None,
};

/// Runs `kubectl drain twin` with the flags of the options. Gives the error text of a
/// drain that failed.
async fn kubectl_drain(b: &Cluster, options: &DrainOptions) -> Option<String> {
    let flags = options.flags();
    let arguments: Vec<&str> = ["drain", TWIN].into_iter().chain(flags.split_whitespace()).collect();
    b.k3s.kubectl(&arguments).await.err().map(|error| error.to_string())
}

/// The labels of the pods on a node.
async fn left(cluster: &Cluster, node: &str) -> BTreeSet<String> {
    cluster.pods_on(node).await.into_values().collect()
}

fn labels(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// The drain of a job.
fn drain(job: &JobView) -> &Drain {
    match &job.report.outcome {
        Outcome::Drain(drain) => drain,
        other => panic!("the outcome of a drain: {other:?}\n{job:#?}"),
    }
}

/// The count of refusals of the pod of the drain of job `number` that this budget holds.
fn held_by(view: &CoreView, number: u64, budget: &str) -> u32 {
    let pods = job(view, number).map(|job| match &job.report.outcome {
        Outcome::Drain(drain) => drain.pods.to_vec(),
        _ => Vec::new(),
    });
    let held = |state: &PodState| match state {
        PodState::Blocked { budget: Some(name), attempts, .. } if name == budget => *attempts,
        _ => 0,
    };
    pods.unwrap_or_default().iter().map(|pod| held(&pod.state)).max().unwrap_or(0)
}

/// Uncordons the two nodes, the one of the app through the core.
async fn uncordon(bed: &Bed) {
    let job = bed.run("b", DRAINED, Operation::Uncordon).await;
    assert_eq!(job.report.state, JobState::Done, "{job:#?}");
    bed.b.kubectl(&["uncordon", TWIN]).await;
    assert!(!bed.b.cordoned(DRAINED).await && !bed.b.cordoned(TWIN).await, "kubectl reads each node open");
}

/// `Command::Run` with a cordon and an uncordon: the patch of `kubectl cordon`.
pub(crate) async fn cordon_and_uncordon(bed: &Bed) {
    let job = bed.run("b", DRAINED, Operation::Cordon).await;
    assert_eq!(job.report.state, JobState::Done, "{job:#?}");
    bed.b.kubectl(&["cordon", TWIN]).await;
    assert!(bed.b.cordoned(DRAINED).await && bed.b.cordoned(TWIN).await, "kubectl reads each node cordoned");
    uncordon(bed).await;
}

/// With no option, the pod of the DaemonSet, the pod with `emptyDir` and the pod with no
/// controller block the drain: the app and kubectl name the same pods, evict none of them,
/// and leave the node cordoned.
pub(crate) async fn a_pod_that_blocks_stops_the_drain_before_its_first_eviction(bed: &Bed) {
    let (app, twin) = (bed.b.pods_on(DRAINED).await, bed.b.pods_on(TWIN).await);
    let options = DrainOptions::default();
    let (job, refused) =
        tokio::join!(bed.run("b", DRAINED, Operation::Drain(options)), kubectl_drain(&bed.b, &options));
    let refused = refused.expect("kubectl refuses the drain");
    let JobState::Failed(error) = &job.report.state else { panic!("a refusal: {job:#?}") };
    let blocks = drain(&job).pods.iter().filter(|pod| matches!(pod.verdict, Verdict::Block(_)));
    let blocked: BTreeSet<String> = blocks.map(|pod| app[&*pod.key.name].clone()).collect();
    let by_kubectl = twin.iter().filter(|(name, _)| refused.contains(&format!("/{name}"))).map(|(_, label)| label);
    assert_eq!(blocked, by_kubectl.cloned().collect(), "the pods that block: the app and kubectl\n{refused}");
    assert_eq!(blocked, labels(&["agent", "lone", "scratch"]));
    assert!(error.to_string().starts_with("3 pods block the drain, so no eviction went out."), "{error}");
    assert_eq!((bed.b.pods_on(DRAINED).await, bed.b.pods_on(TWIN).await), (app, twin), "no eviction went out");
    assert!(drain(&job).cordoned && bed.b.cordoned(DRAINED).await && bed.b.cordoned(TWIN).await);
    uncordon(bed).await;
}

/// A PodDisruptionBudget holds the pod of `guarded`: the app names the budget, sends the
/// eviction again after its pause, and ends at the time limit as kubectl does, with each
/// other pod evicted. The pod of the DaemonSet stays.
pub(crate) async fn a_budget_holds_a_pod_by_its_name_until_the_time_limit(bed: &Bed) {
    let options = DrainOptions { timeout: Some(Duration::from_secs(20)), ..EACH };
    let number = bed.send(Order::new(bed.node("b", DRAINED).await, Operation::Drain(options)).expect("an order"));
    let twice = |view: &CoreView| held_by(view, number, "guarded") >= 2;
    let (_, refused) =
        tokio::join!(bed.until("the budget guarded holds a pod twice", twice), kubectl_drain(&bed.b, &options));
    let job = bed.ended(number).await;
    let JobState::Failed(error) = &job.report.state else { panic!("the time limit: {job:#?}") };
    let text = "the drain did not end in its time limit of 20 s: the PodDisruptionBudget guarded holds 1 pod";
    assert_eq!(error.to_string(), text);
    let refused = refused.expect("kubectl ends at its time limit");
    assert!(refused.contains("global timeout reached: 20s"), "{refused}");
    let expected = labels(&["agent", "guarded"]);
    assert_eq!((left(&bed.b, DRAINED).await, left(&bed.b, TWIN).await), (expected.clone(), expected));
    assert!(bed.b.cordoned(DRAINED).await && bed.b.cordoned(TWIN).await);
}

/// A cancel ends the pause after a refusal at once, and the node stays cordoned.
pub(crate) async fn a_cancel_stops_the_drain_and_the_node_stays_cordoned(bed: &Bed) {
    let number = bed.send(Order::new(bed.node("b", DRAINED).await, Operation::Drain(EACH)).expect("an order"));
    bed.until("the budget guarded holds a pod", |view| held_by(view, number, "guarded") >= 1).await;
    let asked = Instant::now();
    bed.core.send(Command::Cancel(number));
    let job = bed.ended(number).await;
    assert!(asked.elapsed() < Duration::from_secs(5), "the cancel ended the pause of 5 s: {:?}", asked.elapsed());
    assert_eq!(job.report.state, JobState::Cancelled, "{job:#?}");
    let states: Vec<&PodState> = drain(&job).pods.iter().map(|pod| &pod.state).collect();
    assert!(states.contains(&&PodState::Stopped) && states.contains(&&PodState::Kept), "{states:?}");
    assert!(bed.b.cordoned(DRAINED).await, "a cancel does not uncordon");
    assert_eq!(left(&bed.b, DRAINED).await, labels(&["agent", "guarded"]));
}

/// With no budget, the drain ends as kubectl drain: only the pod of the DaemonSet stays.
pub(crate) async fn with_no_budget_the_drain_ends_as_kubectl_drain(bed: &Bed) {
    for namespace in ["app", "kubectl"] {
        bed.b.kubectl(&["delete", "poddisruptionbudget", "guarded", "--namespace", namespace]).await;
    }
    let options = DrainOptions { grace_period: Some(1), timeout: Some(Duration::from_secs(120)), ..EACH };
    let (job, refused) =
        tokio::join!(bed.run("b", DRAINED, Operation::Drain(options)), kubectl_drain(&bed.b, &options));
    assert_eq!((&job.report.state, refused), (&JobState::Done, None), "{job:#?}");
    let states = drain(&job).pods.iter().map(|pod| (&pod.verdict, &pod.state));
    assert!(states.into_iter().all(|(verdict, state)| match verdict {
        Verdict::Keep(_) => *state == PodState::Kept,
        _ => *state == PodState::Gone,
    }));
    let expected = labels(&["agent"]);
    assert_eq!((left(&bed.b, DRAINED).await, left(&bed.b, TWIN).await), (expected.clone(), expected));
    uncordon(bed).await;
}
