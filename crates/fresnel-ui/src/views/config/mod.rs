//! The detail sections of the kinds of the Config group besides the keys of the ConfigMaps
//! and the Secrets (plan section 11.3): HorizontalPodAutoscaler, PodDisruptionBudget,
//! ResourceQuota, LimitRange, PriorityClass, RuntimeClass, Lease and the two webhook
//! configurations. The content of each one is the union of Headlamp and Lens (`research/01`
//! section 3.4.5, `research/02` section 4.3).
//!
//! - `autoscaler`: the target, the replicas and each metric of a HorizontalPodAutoscaler.
//! - `disruption`: the budget of a PodDisruptionBudget and its pods.
//! - `quota`: the use of a ResourceQuota and the limits of a LimitRange, which the view of
//!   a Namespace shows too.
//! - `classes`: the PriorityClass and the RuntimeClass. `lease`: the holder of a Lease.
//! - `webhooks`: each webhook of a configuration, with its Service as a link.

pub(crate) mod autoscaler;
pub(crate) mod classes;
pub(crate) mod disruption;
pub(crate) mod lease;
pub(crate) mod quota;
pub(crate) mod webhooks;
