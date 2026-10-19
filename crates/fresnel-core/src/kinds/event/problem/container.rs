//! The rules of a container of a pod: the one that the field path of an Event names, and its
//! failed probes.
//!
//! A crash loop starts its container again after each wait of its back-off, and the kubelet
//! reports the loop only during a wait. So a run that began after the last report proves no
//! end of a container that restarted: the kubelet ends the back-off only after a run of 10
//! minutes, twice its longest wait. For the same reason a good end proves one only for a
//! container that the kubelet does not start again.

use std::time::{Duration, SystemTime};

use k8s_openapi::api::core::v1::{ContainerStatus, Pod};
use k8s_openapi::jiff::Timestamp;

use super::rules::{READY, after, conditions};
use super::{ProblemNow, Subject};
use crate::kinds::health::{Condition, condition};

/// The run after which the kubelet forgets the back-off of a container: twice its longest
/// wait of 5 minutes.
const BACK_OFF_RESET: Duration = Duration::from_secs(600);
/// The start of the message of an Event in which client-go combines the reports of one
/// object and reason, after 10 messages in 10 minutes. The Event keeps the field path of the
/// first report, so its newer ones can be of an other container.
const COMBINED: &str = "(combined from similar events): ";

/// The rule of a failed probe, by the probe that the message names: a readiness probe makes
/// its container not ready, a liveness or a startup probe restarts it.
pub(super) fn probe(pod: &Pod, subject: &Subject<'_>, at: SystemTime) -> ProblemNow {
    let names = |probes: &[&str]| probes.iter().any(|probe| subject.message.starts_with(probe));
    if names(&["Readiness probe"]) {
        readiness(pod, subject, at)
    } else if names(&["Liveness probe", "Startup probe"]) {
        container(pod, subject, at)
    } else {
        one_part(subject).err().unwrap_or_else(|| ProblemNow::unknown("the message names no probe"))
    }
}

/// The rule of a container that cannot start, that crashed, or whose probe restarted it.
pub(super) fn container(pod: &Pod, subject: &Subject<'_>, at: SystemTime) -> ProblemNow {
    let found = match named(pod, subject) {
        Ok(found) => found,
        Err(now) => return now,
    };
    let the = format!("the container {}", found.name);
    let state = found.state.as_ref();
    match state.map(|state| (&state.waiting, &state.running, &state.terminated)) {
        Some((Some(waiting), _, _)) => match &waiting.reason {
            Some(reason) => ProblemNow::ongoing(format!("{the} waits: {reason}")),
            None => ProblemNow::ongoing(format!("{the} waits")),
        },
        Some((None, Some(_), _)) => runs(found, subject.last, at).unwrap_or_else(|| {
            let since = started(found);
            let again = found.restart_count > 0 && since.zip(subject.last).is_some_and(|(since, last)| since > last);
            let healthy = if again { "runs again, for 10 min or more" } else { "runs" };
            after(since, subject.last, format!("{the} {healthy}"))
        }),
        Some((None, None, Some(ended))) if ended.exit_code == 0 => {
            if starts_again(pod, &found.name) {
                return ProblemNow::unknown(format!("{the} ended, and the kubelet starts it again"));
            }
            let started = ended.started_at.as_ref().or(ended.finished_at.as_ref()).map(|time| time.0);
            after(started, subject.last, format!("{the} ended well"))
        }
        Some((None, None, Some(ended))) => {
            ProblemNow::ongoing(format!("{the} ended with exit code {}", ended.exit_code))
        }
        Some((None, None, None)) | None => ProblemNow::unknown(format!("the pod gives no state of {the}")),
    }
}

/// A failed readiness probe makes its container not ready until the probe works again. The
/// pod is ready when each of its containers and each of its readiness gates is: its
/// condition gives the time of the end.
fn readiness(pod: &Pod, subject: &Subject<'_>, at: SystemTime) -> ProblemNow {
    let found = match named(pod, subject) {
        Ok(found) => found,
        Err(now) => return now,
    };
    let the = format!("the container {}", found.name);
    if !found.ready {
        return ProblemNow::ongoing(format!("{the} is not ready"));
    }
    if let Some(again) = runs(found, subject.last, at) {
        return again;
    }
    match condition(conditions(pod), READY.name) {
        Some(ready) if ready.status() == "True" => after(ready.since(), subject.last, format!("{the} is ready")),
        Some(ready) => match ready.reason() {
            "" => ProblemNow::unknown(format!("{the} is ready, and the pod is not")),
            reason => ProblemNow::unknown(format!("{the} is ready, and the pod is not: {reason}")),
        },
        None => ProblemNow::unknown(format!("the pod has no condition {}", READY.name)),
    }
}

/// The status of the container that the field path names, as `spec.containers{web}`. A
/// pod has one name for each container, init container and ephemeral container.
fn named<'a>(pod: &'a Pod, subject: &Subject<'_>) -> Result<&'a ContainerStatus, ProblemNow> {
    one_part(subject)?;
    let name = subject.part.and_then(|part| part.split_once('{')).and_then(|(_, rest)| rest.strip_suffix('}'));
    let name = name.ok_or_else(|| ProblemNow::unknown("the Event names no container"))?;
    let status = pod.status.as_ref();
    let lists = status.into_iter().flat_map(|status| {
        [&status.container_statuses, &status.init_container_statuses, &status.ephemeral_container_statuses]
    });
    let found = lists.flatten().flatten().find(|container| container.name == name);
    found.ok_or_else(|| ProblemNow::unknown(format!("the pod gives no state of the container {name}")))
}

/// "Unknown" for an Event that combines the reports of some parts of its object.
fn one_part(subject: &Subject<'_>) -> Result<(), ProblemNow> {
    if subject.message.starts_with(COMBINED) {
        return Err(ProblemNow::unknown("the Event combines similar reports, which can be of an other container"));
    }
    Ok(())
}

/// True unless the spec proves that the kubelet does not start the container again after a
/// good end: a restart policy `Never` or `OnFailure` of the pod or of the container, an init
/// container that is not a sidecar (its own policy `Always`), or an ephemeral container.
fn starts_again(pod: &Pod, name: &str) -> bool {
    let Some(spec) = pod.spec.as_ref() else {
        return true;
    };
    if let Some(container) = spec.containers.iter().find(|container| container.name == name) {
        let policy = container.restart_policy.as_deref().or(spec.restart_policy.as_deref());
        return !matches!(policy, Some("Never" | "OnFailure"));
    }
    if let Some(init) = spec.init_containers.iter().flatten().find(|container| container.name == name) {
        return init.restart_policy.as_deref() == Some("Always");
    }
    !spec.ephemeral_containers.iter().flatten().any(|container| container.name == name)
}

/// The start of the run of a container that runs.
fn started(container: &ContainerStatus) -> Option<Timestamp> {
    let running = container.state.as_ref()?.running.as_ref()?;
    running.started_at.as_ref().map(|time| time.0)
}

/// "Unknown" for a container that restarted and runs since after the last report, for less
/// than the reset of its back-off at `at`: a crash loop runs it so between its waits.
fn runs(container: &ContainerStatus, last: Option<Timestamp>, at: SystemTime) -> Option<ProblemNow> {
    let since = started(container).zip(last).filter(|(started, last)| started > last)?.0;
    let reset = at.duration_since(SystemTime::from(since)).is_ok_and(|ran| ran >= BACK_OFF_RESET);
    let restarts = match container.restart_count {
        1 => "1 restart".to_owned(),
        count => format!("{count} restarts"),
    };
    (container.restart_count > 0 && !reset).then(|| {
        ProblemNow::unknown(format!(
            "the container {} runs again after {restarts}, for less than 10 min: a crash loop runs it between \
             its waits",
            container.name
        ))
    })
}
