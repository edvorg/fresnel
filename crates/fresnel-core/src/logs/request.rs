//! What a log view asks for, and the request of one stream of one container.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use http::Request;
use k8s_openapi::jiff::Timestamp;
use kube::core::subresource::LogParams;

use crate::cluster::ClusterId;
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// One log view: the logs of a pod, or of the pods of a workload (plan section 12.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRequest {
    /// The cluster of the object.
    pub cluster: ClusterId,
    /// A Pod, or a workload whose pods the owner index knows: Deployment, StatefulSet,
    /// DaemonSet, ReplicaSet, ReplicationController or Job.
    pub kind: Kind,
    /// The namespace and the name of the object.
    pub key: ObjectKey,
    /// The UID of the object that the user saw. A pod of the same name with an other UID
    /// is an other pod. Empty for any object of the name.
    pub uid: Arc<str>,
    /// The options of the streams.
    pub options: LogOptions,
}

/// The options of the streams of a view. The view itself shows or hides the times and
/// wraps the lines: each stream has the times, because the app needs them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogOptions {
    /// Which containers of each pod.
    pub containers: Containers,
    /// The logs of the instance of each container before its last restart, with no follow.
    pub previous: bool,
    /// The count of lines from the end of each log. `None` for the full log.
    pub tail: Option<u32>,
    /// The first time of the lines. `None` for each line.
    pub since: Option<Since>,
}

impl Default for LogOptions {
    /// The last 1,000 lines of the default container, and each new line (research 08,
    /// section 6.1).
    fn default() -> Self {
        Self { containers: Containers::Default, previous: false, tail: Some(1000), since: None }
    }
}

/// The containers of each pod of a view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Containers {
    /// The container that restarts most. With no restart, the container of the annotation
    /// `kubectl.kubernetes.io/default-container`, else the first one, as kubectl does.
    #[default]
    Default,
    /// The container of this name.
    Named(Arc<str>),
    /// Each container: the init containers, the containers and the ephemeral containers.
    All,
}

/// The first time of the lines of a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Since {
    /// The lines of this last time before the start of each stream.
    Last(Duration),
    /// The lines from this time.
    Time(SystemTime),
}

/// Where one stream starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    /// The options of the view.
    Options { tail: Option<u32>, since: Option<Since> },
    /// A time of the cluster, after a break or a restart of the container.
    Time(Timestamp),
    /// The full log, for a stream after a first one that had no line.
    Whole,
}

/// The request of one stream of one container.
pub(super) fn stream(
    pod: &ObjectKey,
    container: &str,
    previous: bool,
    from: Origin,
) -> Result<Request<Vec<u8>>, CoreError> {
    let mut params = LogParams {
        container: Some(container.to_owned()),
        // The log of an earlier instance does not grow.
        follow: !previous,
        previous,
        timestamps: true,
        ..LogParams::default()
    };
    match from {
        Origin::Options { tail, since } => {
            params.tail_lines = tail.map(i64::from);
            match since {
                Some(Since::Last(time)) => {
                    params.since_seconds = Some(i64::try_from(time.as_secs()).unwrap_or(i64::MAX).max(1))
                }
                Some(Since::Time(time)) => params.since_time = Some(full_second(time)?),
                None => {}
            }
        }
        Origin::Time(time) => params.since_time = Some(time),
        Origin::Whole => {}
    }
    let path = Kind::Pod.info().path_in(&pod.namespace);
    let request = kube::core::Request::new(path).logs(&pod.name, &params);
    request.map_err(|error| CoreError::invalid(format!("the request of a log cannot be built: {error}")))
}

/// The full second at or before a time of the user: kube rounds the time to the nearest
/// second, and a later second would lose lines.
fn full_second(time: SystemTime) -> Result<Timestamp, CoreError> {
    let out_of_range = |error| CoreError::invalid(format!("the time of the log is out of range: {error}"));
    let time = Timestamp::try_from(time).map_err(out_of_range)?;
    Timestamp::from_second(time.as_second()).map_err(out_of_range)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(previous: bool, from: Origin) -> String {
        stream(&ObjectKey::new("shop", "web-1"), "app", previous, from).expect("a request").uri().to_string()
    }

    #[test]
    fn a_first_stream_follows_with_times_and_the_options_of_the_view() {
        let options = |tail, since| Origin::Options { tail, since };
        assert_eq!(
            uri(false, options(Some(1000), None)),
            "/api/v1/namespaces/shop/pods/web-1/log?&container=app&follow=true&tailLines=1000&timestamps=true"
        );
        let hour = Some(Since::Last(Duration::from_secs(3600)));
        assert_eq!(
            uri(false, options(None, hour)),
            "/api/v1/namespaces/shop/pods/web-1/log?&container=app&follow=true&sinceSeconds=3600&timestamps=true"
        );
        let at = SystemTime::UNIX_EPOCH + Duration::from_millis(1_800_000_000_250);
        assert_eq!(
            uri(true, options(None, Some(Since::Time(at)))),
            "/api/v1/namespaces/shop/pods/web-1/log?&container=app&previous=true&sinceTime=2027-01-15T08%3A00%3A00Z&timestamps=true"
        );
    }

    /// kube rounds `sinceTime` to the nearest second, so the caller gives a full second
    /// (research 08, section 6.1): the request keeps it.
    #[test]
    fn a_later_stream_starts_at_its_second_with_no_tail_and_no_fraction() {
        let second = crate::logs::ingest::second_of(1_760_299_200_987_654_321);
        assert_eq!(
            uri(false, Origin::Time(second)),
            "/api/v1/namespaces/shop/pods/web-1/log?&container=app&follow=true&sinceTime=2025-10-12T20%3A00%3A00Z&timestamps=true"
        );
        assert_eq!(
            uri(false, Origin::Whole),
            "/api/v1/namespaces/shop/pods/web-1/log?&container=app&follow=true&timestamps=true"
        );
    }
}
