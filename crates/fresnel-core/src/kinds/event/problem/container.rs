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

use super::quiet::RETRY;
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
    let now = if names(&["Readiness probe"]) {
        readiness(pod, subject, at)
    } else if names(&["Liveness probe", "Startup probe"]) {
        container(pod, subject, at)
    } else {
        one_part(subject).err().unwrap_or_else(|| ProblemNow::unknown("the message names no probe"))
    };
    // The kubelet tries a report at each run of a failed probe: the pace of the reports
    // tells nothing of a probe that runs seldom, or of one that the app cannot find.
    if period(pod, subject).is_some_and(|period| period <= RETRY) { now } else { now.settled() }
}

/// The time between two runs of the probe that the message names, of the container of the
/// field path. `None` when the spec of the pod does not give it.
fn period(pod: &Pod, subject: &Subject<'_>) -> Option<Duration> {
    let name = part_name(subject)?;
    let spec = pod.spec.as_ref()?;
    let mut containers = spec.containers.iter().chain(spec.init_containers.iter().flatten());
    let container = containers.find(|container| container.name == name)?;
    let probes = [
        ("Liveness probe", &container.liveness_probe),
        ("Readiness probe", &container.readiness_probe),
        ("Startup probe", &container.startup_probe),
    ];
    let (_, probe) = probes.into_iter().find(|(text, _)| subject.message.starts_with(text))?;
    let probe = probe.as_ref()?;
    // The API gives a probe with no period 10 s and with no time limit 1 s. A run starts after
    // the end of the one before it, so a longer time limit sets the pace.
    let seconds = probe.period_seconds.unwrap_or(10).max(probe.timeout_seconds.unwrap_or(1));
    Some(Duration::from_secs(u64::try_from(seconds).ok()?))
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
                return ProblemNow::undecided(format!("{the} ended, and the kubelet starts it again"));
            }
            let started = ended.started_at.as_ref().or(ended.finished_at.as_ref()).map(|time| time.0);
            after(started, subject.last, format!("{the} ended well"))
        }
        Some((None, None, Some(ended))) => {
            ProblemNow::ongoing(format!("{the} ended with exit code {}", ended.exit_code))
        }
        Some((None, None, None)) | None => ProblemNow::undecided(format!("the pod gives no state of {the}")),
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
            "" => ProblemNow::undecided(format!("{the} is ready, and the pod is not")),
            reason => ProblemNow::undecided(format!("{the} is ready, and the pod is not: {reason}")),
        },
        None => ProblemNow::undecided(format!("the pod has no condition {}", READY.name)),
    }
}

/// The status of the container that the field path names, as `spec.containers{web}`. A
/// pod has one name for each container, init container and ephemeral container.
fn named<'a>(pod: &'a Pod, subject: &Subject<'_>) -> Result<&'a ContainerStatus, ProblemNow> {
    one_part(subject)?;
    let name = part_name(subject).ok_or_else(|| ProblemNow::undecided("the Event names no container"))?;
    let status = pod.status.as_ref();
    let lists = status.into_iter().flat_map(|status| {
        [&status.container_statuses, &status.init_container_statuses, &status.ephemeral_container_statuses]
    });
    let found = lists.flatten().flatten().find(|container| container.name == name);
    found.ok_or_else(|| ProblemNow::undecided(format!("the pod gives no state of the container {name}")))
}

/// The name of the container in the field path of the Event, as `spec.containers{web}`.
fn part_name<'a>(subject: &Subject<'a>) -> Option<&'a str> {
    subject.part?.split_once('{')?.1.strip_suffix('}')
}

/// "Unknown" for an Event that combines the reports of some parts of its object.
fn one_part(subject: &Subject<'_>) -> Result<(), ProblemNow> {
    if subject.message.starts_with(COMBINED) {
        return Err(ProblemNow::undecided("the Event combines similar reports, which can be of an other container"));
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
        ProblemNow::undecided(format!(
            "the container {} runs again after {restarts}, for less than 10 min: a crash loop runs it between \
             its waits",
            container.name
        ))
    })
}
