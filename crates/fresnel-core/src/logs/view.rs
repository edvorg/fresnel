//! What a log view reads: its lines, and the state of each source (design rule 1).

use std::fmt::{self, Display};
use std::sync::Arc;
use std::time::SystemTime;

use super::request::LogRequest;
use super::ring::Lines;
use crate::error::CoreError;
use crate::store::ObjectKey;

/// One log view at one moment. A copy costs some pointers.
#[derive(Clone, Debug, PartialEq)]
pub struct LogView {
    /// What the view asked for.
    pub request: LogRequest,
    /// How far the view knows its pods.
    pub pods: PodsState,
    /// Each source of the view, in the sequence of their start. A line names its source by
    /// its index here. A source stays after its end, with its final state.
    pub sources: Arc<[SourceView]>,
    /// The lines of all sources.
    pub lines: Lines,
    /// The count of lines that left the ring at its limits.
    pub dropped: u64,
    /// The bytes of the lines in the ring, with their text and styles.
    pub bytes: usize,
    /// The count of containers with no stream, because the view has its most streams, or
    /// its most sources over its life.
    pub unstreamed: usize,
    /// The count of runs of the task of the view that ended with an internal error of the
    /// app. Each new run starts with no line and new sources: the earlier lines are gone.
    pub restarts: u32,
}

/// How far a view knows the pods that it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PodsState {
    /// The store of the pods or the owner index has no complete list yet: "no source"
    /// means "not known", not "none".
    Loading,
    /// The view follows the pods: a pod that comes gets its sources.
    Following,
    /// The pod of the view is gone. Its sources show what they read.
    Gone,
}

/// One source of a view: one container of one pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceView {
    /// The pod.
    pub pod: ObjectKey,
    /// The container.
    pub container: Arc<str>,
    /// What the stream does.
    pub state: SourceState,
    /// The count of lines of the source that went into the view.
    pub lines: u64,
    /// The count of lines that a new stream sent again and that the view has one time.
    pub repeated: u64,
}

/// What the stream of one source does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceState {
    /// The request is on its way.
    Connecting,
    /// The stream is open. A stream with no new line is open too: a log can be quiet.
    Streaming,
    /// The container has no log to read now: it did not start, or it ended and starts
    /// again. The stream starts when the pod changes.
    Waiting {
        /// Why, for example "the container did not start: ImagePullBackOff".
        reason: Arc<str>,
    },
    /// The stream broke or failed. A new stream starts at `at` from the latest line.
    Reconnecting {
        /// Why.
        reason: CoreError,
        /// The wall clock time of the next attempt.
        at: SystemTime,
    },
    /// The log ended for good: the container ended and does not start again, the pod is
    /// gone, or the view read the full log of an earlier instance.
    Ended {
        /// Why.
        reason: Arc<str>,
    },
    /// The source cannot read the log, for example because the rules forbid it.
    Failed {
        /// Why.
        reason: CoreError,
    },
}

impl SourceState {
    /// True while lines can come: the merge of a view waits for such a source.
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Connecting | Self::Streaming | Self::Reconnecting { .. })
    }

    /// True for a final state: the source holds no stream of the view.
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Ended { .. } | Self::Failed { .. })
    }
}

impl Display for SourceState {
    /// The state in words, with no time: the UI gives a time in the form of the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connecting => f.write_str("connecting"),
            Self::Streaming => f.write_str("streaming"),
            Self::Waiting { reason } => write!(f, "waiting: {reason}"),
            Self::Reconnecting { reason, .. } => write!(f, "reconnecting after: {reason}"),
            Self::Ended { reason } => write!(f, "ended: {reason}"),
            Self::Failed { reason } => write!(f, "failed: {reason}"),
        }
    }
}
