//! The reads of the editors (plan section 12.9): the text of each editor that the UI shows.
//!
//! One task reads the object of each new editor, all at the same time: a cluster that does
//! not answer holds no editor of an other cluster. A read runs once; the text then belongs to
//! the editor, and a later version of the object does not change it. An editor that closes
//! drops its read, and an answer for it has no reader.

use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use tokio::sync::watch;

use super::repaint::Repaint;
use super::{Inner, Phase};
use crate::cluster::Registry;
use crate::error::CoreError;
use crate::manifest::{self, EditText};
use crate::ops::Target;

/// The text that an editor asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRequest {
    /// The identity of the request, which the view gives: a new one reads again.
    pub number: u64,
    /// The object of the editor.
    pub target: Target,
}

/// The read of the text of one editor.
#[derive(Clone, Debug)]
pub struct EditRead {
    pub request: EditRequest,
    pub state: EditState,
}

/// How far the read of an editor is.
#[derive(Clone, Debug)]
pub enum EditState {
    /// The request is on its way, with the time limit of a GET.
    Reading,
    Done(Arc<EditText>),
    Failed(CoreError),
}

impl Inner {
    /// Reads the text of each editor that the UI shows and has no text yet. The task of the
    /// reads starts with the first editor.
    pub(super) fn show_editors(&self, registry: &Registry, requests: Vec<EditRequest>) {
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        let start = || EditReader::start(&self.tasks, registry.clone(), self.repaint.clone());
        state.edits.get_or_insert_with(start).show(requests);
        drop(state);
        self.repaint.request();
    }
}

/// The reads of the editors, in the sequence of their requests.
type Reads = Arc<Mutex<Vec<Arc<EditRead>>>>;

fn lock(reads: &Reads) -> MutexGuard<'_, Vec<Arc<EditRead>>> {
    reads.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The task that reads the texts of the editors. It stops when this value drops.
pub(super) struct EditReader {
    _task: TaskHandle,
    wanted: watch::Sender<Vec<EditRequest>>,
    reads: Reads,
}

impl EditReader {
    fn start(tasks: &Tasks, registry: Registry, repaint: Repaint) -> Self {
        let wanted = watch::Sender::new(Vec::new());
        let reads = Reads::default();
        let task = tasks.start(TaskName::new("editor reads"), Restart::Always(Backoff::default()), {
            let (wanted, reads) = (wanted.clone(), Arc::clone(&reads));
            move || serve(wanted.subscribe(), Arc::clone(&reads), registry.clone(), repaint.clone())
        });
        Self { _task: task, wanted, reads }
    }

    /// Keeps the read of each of these requests and drops each other one.
    fn show(&self, requests: Vec<EditRequest>) {
        {
            let mut reads = lock(&self.reads);
            let kept: Vec<Arc<EditRead>> = requests
                .iter()
                .map(|request| {
                    let read = reads.iter().find(|read| read.request == *request);
                    read.map_or_else(
                        || Arc::new(EditRead { request: request.clone(), state: EditState::Reading }),
                        Arc::clone,
                    )
                })
                .collect();
            *reads = kept;
        }
        self.wanted.send_replace(requests);
    }

    /// The reads, for the view.
    pub(super) fn reads(&self) -> Vec<Arc<EditRead>> {
        lock(&self.reads).clone()
    }
}

/// A read of one editor in progress, with its answer.
type Pending = Pin<Box<dyn Future<Output = (EditRequest, EditState)> + Send>>;

/// The body of the task: starts a read for each request that has no text, and keeps each
/// answer of a request that the UI still shows.
async fn serve(
    mut wanted: watch::Receiver<Vec<EditRequest>>,
    reads: Reads,
    registry: Registry,
    repaint: Repaint,
) -> Result<(), CoreError> {
    let mut started: Vec<EditRequest> = Vec::new();
    let mut pending: FuturesUnordered<Pending> = FuturesUnordered::new();
    loop {
        let requests = wanted.borrow_and_update().clone();
        started.retain(|request| requests.contains(request));
        for request in requests {
            let waits =
                lock(&reads).iter().any(|read| read.request == request && matches!(read.state, EditState::Reading));
            if waits && !started.contains(&request) {
                started.push(request.clone());
                pending.push(Box::pin(read(registry.clone(), request)));
            }
        }
        tokio::select! {
            changed = wanted.changed() => changed.map_err(|_| CoreError::invalid("the core stopped"))?,
            Some((request, state)) = pending.next() => {
                let mut kept = lock(&reads);
                if let Some(slot) = kept.iter_mut().find(|read| read.request == request) {
                    *slot = Arc::new(EditRead { request, state });
                }
                drop(kept);
                repaint.request();
            }
        }
    }
}

/// Reads the text of one editor through the gateway of its cluster.
async fn read(registry: Registry, request: EditRequest) -> (EditRequest, EditState) {
    let Target { cluster, kind, key, uid } = &request.target;
    let state = match registry.link(cluster) {
        None => EditState::Failed(CoreError::invalid(format!("{cluster} is not connected"))),
        Some(link) => match manifest::read_for_edit(&link.gateway, *kind, key, uid).await {
            Ok(text) => EditState::Done(Arc::new(text)),
            Err(error) => EditState::Failed(error),
        },
    };
    (request, state)
}
