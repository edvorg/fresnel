//! The rollout status of a workload, with the rules and the words of `kubectl rollout
//! status` (research 08 section 4.4). The app reads it from the object in the store, so it
//! sends no request: the watch of the store brings each new state.

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use crate::kinds::Describe;

/// The type of the update strategy for which kubectl gives a rollout status.
const ROLLING_UPDATE: &str = "RollingUpdate";
/// The reason of the condition `Progressing` of a Deployment after its deadline.
const TIMED_OUT: &str = "ProgressDeadlineExceeded";

/// How far a rollout is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rollout {
    /// The rollout goes on; the text tells what it waits for.
    Waiting(String),
    /// The rollout completed.
    Complete(String),
    /// The rollout cannot complete, or kubectl gives no status for it.
    Failed(String),
}

impl Rollout {
    /// True when the state does not change by itself: a wait for it can end.
    pub fn is_final(&self) -> bool {
        !matches!(self, Self::Waiting(_))
    }
}

/// A workload with a rollout.
pub(crate) trait Rolls: Describe {
    /// The status of its rollout.
    fn rollout(&self) -> Rollout;
}

impl Rolls for Deployment {
    fn rollout(&self) -> Rollout {
        let name = name(&self.metadata);
        let status = self.status.clone().unwrap_or_default();
        if self.metadata.generation > status.observed_generation {
            return Rollout::Waiting("Waiting for deployment spec update to be observed...".to_owned());
        }
        let conditions = status.conditions.iter().flatten();
        if conditions
            .filter(|condition| condition.type_ == "Progressing")
            .any(|c| c.reason.as_deref() == Some(TIMED_OUT))
        {
            return Rollout::Failed(format!("deployment \"{name}\" exceeded its progress deadline"));
        }
        let updated = status.updated_replicas.unwrap_or(0);
        let wait =
            |text: String| Rollout::Waiting(format!("Waiting for deployment \"{name}\" rollout to finish: {text}..."));
        if let Some(wanted) = self.spec.as_ref().and_then(|spec| spec.replicas).filter(|wanted| updated < *wanted) {
            return wait(format!("{updated} out of {wanted} new replicas have been updated"));
        }
        let replicas = status.replicas.unwrap_or(0);
        if replicas > updated {
            return wait(format!("{} old replicas are pending termination", replicas - updated));
        }
        let available = status.available_replicas.unwrap_or(0);
        if available < updated {
            return wait(format!("{available} of {updated} updated replicas are available"));
        }
        Rollout::Complete(format!("deployment \"{name}\" successfully rolled out"))
    }
}

impl Rolls for DaemonSet {
    fn rollout(&self) -> Rollout {
        let name = name(&self.metadata);
        let strategy = self.spec.as_ref().and_then(|spec| spec.update_strategy.as_ref());
        if let Some(failed) = not_rolling(strategy.and_then(|strategy| strategy.type_.as_deref())) {
            return failed;
        }
        let status = self.status.clone().unwrap_or_default();
        if self.metadata.generation > status.observed_generation {
            return Rollout::Waiting("Waiting for daemon set spec update to be observed...".to_owned());
        }
        let (desired, updated) = (status.desired_number_scheduled, status.updated_number_scheduled.unwrap_or(0));
        let wait =
            |text: String| Rollout::Waiting(format!("Waiting for daemon set \"{name}\" rollout to finish: {text}..."));
        if updated < desired {
            return wait(format!("{updated} out of {desired} new pods have been updated"));
        }
        let available = status.number_available.unwrap_or(0);
        if available < desired {
            return wait(format!("{available} of {desired} updated pods are available"));
        }
        Rollout::Complete(format!("daemon set \"{name}\" successfully rolled out"))
    }
}

impl Rolls for StatefulSet {
    /// kubectl reads a partition whenever `rollingUpdate` is there, and the server always
    /// sets it. Thus the last two rules apply only to an object that a test made by hand.
    fn rollout(&self) -> Rollout {
        let spec = self.spec.as_ref();
        let strategy = spec.and_then(|spec| spec.update_strategy.as_ref());
        if let Some(failed) = not_rolling(strategy.and_then(|strategy| strategy.type_.as_deref())) {
            return failed;
        }
        let status = self.status.clone().unwrap_or_default();
        let observed = status.observed_generation.unwrap_or(0);
        if observed == 0 || self.metadata.generation.is_some_and(|generation| generation > observed) {
            return Rollout::Waiting("Waiting for statefulset spec update to be observed...".to_owned());
        }
        let wanted = spec.and_then(|spec| spec.replicas);
        let ready = status.ready_replicas.unwrap_or(0);
        if let Some(wanted) = wanted.filter(|wanted| ready < *wanted) {
            return Rollout::Waiting(format!("Waiting for {} pods to be ready...", wanted - ready));
        }
        let updated = status.updated_replicas.unwrap_or(0);
        if let Some(rolling) = strategy.and_then(|strategy| strategy.rolling_update.as_ref()) {
            if let (Some(wanted), Some(partition)) = (wanted, rolling.partition)
                && updated < wanted - partition
            {
                let text = format!("{updated} out of {} new pods have been updated", wanted - partition);
                return Rollout::Waiting(format!("Waiting for partitioned roll out to finish: {text}..."));
            }
            return Rollout::Complete(format!(
                "partitioned roll out complete: {updated} new pods have been updated..."
            ));
        }
        let (current, next) = (status.current_revision.unwrap_or_default(), status.update_revision.unwrap_or_default());
        if next != current {
            return Rollout::Waiting(format!(
                "waiting for statefulset rolling update to complete {updated} pods at revision {next}..."
            ));
        }
        let pods = status.current_replicas.unwrap_or(0);
        Rollout::Complete(format!("statefulset rolling update complete {pods} pods at revision {current}..."))
    }
}

/// kubectl gives no status for the strategy `OnDelete`: its pods change only when a person
/// deletes them. The server sets `RollingUpdate` when the spec has no type.
fn not_rolling(strategy: Option<&str>) -> Option<Rollout> {
    let strategy = strategy.unwrap_or(ROLLING_UPDATE);
    (strategy != ROLLING_UPDATE)
        .then(|| Rollout::Failed(format!("rollout status is only available for {ROLLING_UPDATE} strategy type")))
}

fn name(meta: &ObjectMeta) -> &str {
    meta.name.as_deref().unwrap_or_default()
}

#[cfg(test)]
mod tests;
