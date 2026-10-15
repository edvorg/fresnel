//! The state of a job (plan section 7.3): pending, done, failed, not sent, result not known.
//! A long job also has its steps and can end with a cancel.

use std::sync::Arc;

use super::drain::Drain;
use super::history::Revision;
use super::rollout::Rollout;
use crate::cluster::{Fate, WriteError};
use crate::error::CoreError;
use crate::store::Written;

/// How far a job is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    /// The job runs: a request is on its way, or a step waits.
    Pending,
    /// The job completed.
    Done,
    /// The server or a check of the app refused it, and nothing changed.
    Failed(CoreError),
    /// The request did not go to the server, for example with no connection. One more
    /// send is safe.
    NotSent(CoreError),
    /// A write went out, and no answer came back: it can be in effect. The app does not
    /// send it again; a last step reads the object (design rule 6).
    Unknown(CoreError),
    /// The user stopped the job. A write in progress ran to its end first.
    Cancelled,
}

impl JobState {
    /// The state of a job after a write that gave no result.
    pub(crate) fn of_write(error: WriteError) -> Self {
        match error.fate {
            Fate::NotSent => Self::NotSent(error.error),
            Fate::Refused => Self::Failed(error.error),
            Fate::Unknown => Self::Unknown(error.error),
        }
    }

    /// True for a state that does not change any more.
    pub fn is_final(&self) -> bool {
        !matches!(self, Self::Pending)
    }

    /// The error of the state, when it has one.
    pub fn error(&self) -> Option<&CoreError> {
        match self {
            Self::Failed(error) | Self::NotSent(error) | Self::Unknown(error) => Some(error),
            Self::Pending | Self::Done | Self::Cancelled => None,
        }
    }
}

/// How far one step of a job is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepState {
    /// The step runs.
    Running,
    /// The step completed. The text tells its result.
    Done(String),
    /// The step failed; the state of the job tells why.
    Failed,
    /// The job ended before this step completed.
    Stopped,
}

/// One step of a job, for example "delete the old Job".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// What the step does.
    pub text: String,
    /// How far it is.
    pub state: StepState,
}

/// What a job shows while it runs: its state, its steps and what it gave.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// How far the job is.
    pub state: JobState,
    /// The steps that started, in their sequence.
    pub steps: Vec<Step>,
    /// What the job gave.
    pub outcome: Outcome,
}

impl Default for Report {
    fn default() -> Self {
        Self { state: JobState::Pending, steps: Vec::new(), outcome: Outcome::None }
    }
}

/// What a job gives besides its state.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Outcome {
    /// Nothing yet.
    #[default]
    None,
    /// The result of a write: the object shows "change pending" until its watch event
    /// arrives (plan section 9.10). `None` for an answer with no identity.
    Written(Option<Written>),
    /// The revisions of a workload.
    Revisions(Arc<[Revision]>),
    /// The last state of a rollout.
    Rollout(Rollout),
    /// The manifest of "run again" after the delete of the old Job, as JSON text: the user
    /// can copy it when its create fails.
    Manifest(Arc<str>),
    /// The cordon and each pod of a drain.
    Drain(Drain),
}

impl Outcome {
    /// The result of a write of the job, for the mark "change pending" of its object.
    pub fn written(&self) -> Option<&Written> {
        match self {
            Self::Written(written) => written.as_ref(),
            Self::Drain(drain) => drain.written.as_ref(),
            Self::None | Self::Revisions(_) | Self::Rollout(_) | Self::Manifest(_) => None,
        }
    }
}
