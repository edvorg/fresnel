//! What a source does next, from the pod in its store: a stream of which instance, a wait
//! for a change of the pod, or its end.
//!
//! The end of a stream has two causes: the container ended, or the connection closed. Only
//! the state of the pod tells which one (research 05, section 5.1).

use std::sync::Arc;

use k8s_openapi::api::core::v1::Pod;

use super::Stream;
use crate::error::CoreError;
use crate::logs::container::{self, Served};
use crate::logs::view::SourceState;
use crate::store::Snapshot;

/// The next step of a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Look {
    /// A stream of the log of the instance with this ID.
    Open(Arc<str>),
    /// No log to read now. The source waits for a change of its pod, with the reason.
    Wait(String),
    /// The final state of the source.
    End(SourceState),
}

/// The next step of the source of `stream`. `complete` is the instance whose log a stream
/// read to its end.
pub(super) fn look(pods: &Snapshot<Pod>, stream: &Stream, previous: bool, complete: Option<&str>) -> Look {
    if !pods.listed {
        return Look::Wait("the pods of the cluster load".to_owned());
    }
    let same = |pod: &&Arc<Pod>| stream.uid.is_empty() || pod.metadata.uid.as_deref() == Some(&*stream.uid);
    let Some(pod) = pods.items.get(&stream.pod).filter(same) else {
        return Look::End(SourceState::Ended { reason: "the pod is gone".into() });
    };
    let Some(run) = container::run_of(pod, &stream.container, previous) else {
        let reason = CoreError::invalid(format!("the pod has no container {}", stream.container));
        return Look::End(SourceState::Failed { reason });
    };
    match run.served {
        Served::Nothing(reason) if previous => Look::End(SourceState::Ended { reason: reason.into() }),
        Served::Nothing(reason) => Look::Wait(reason),
        Served::Instance { id, running: false } if complete == Some(&*id) => {
            let end = run.end.unwrap_or_else(|| "no reason".to_owned());
            if run.starts_again {
                Look::Wait(format!("the container ended ({end}) and starts again"))
            } else {
                Look::End(SourceState::Ended { reason: format!("the container ended: {end}").into() })
            }
        }
        Served::Instance { id, .. } => Look::Open(id),
    }
}
