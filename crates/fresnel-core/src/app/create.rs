//! The create of a new Secret (plan section 12.2).
//!
//! One task sends the creates, one at a time in their sequence. A create that goes out
//! runs to its answer: the stop of a write in progress would leave its result unknown. The
//! view shows the last create.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use tokio::sync::Notify;

use super::repaint::Repaint;
use super::{Inner, Phase};
use crate::cluster::Registry;
use crate::error::CoreError;
use crate::keys::{self, NewSecret, WriteState};

/// The most creates that wait for the task. The user makes one in a dialog.
const QUEUE_LIMIT: usize = 16;

impl Inner {
    /// Gives a create to the task of the creates, which starts with the first one.
    pub(super) fn create_secret(&self, registry: &Registry, secret: NewSecret) {
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        // A read-only cluster gets no write, also from a dialog that did not close its gate.
        if let Some(refusal) = state.config.refusal(&secret.cluster) {
            drop(state);
            let text = format!("the create of Secret {}/{} did not go: {refusal}", secret.namespace, secret.name);
            self.journal.warning(secret.cluster.as_str(), text);
            return;
        }
        let start = || SecretWriter::start(&self.tasks, registry.clone(), self.repaint.clone());
        state.creates.get_or_insert_with(start).ask(secret);
        drop(state);
        self.repaint.request();
    }
}

/// One create of a Secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateJob {
    /// A number that grows with each create.
    pub number: u64,
    /// What the view asked for.
    pub secret: NewSecret,
    /// How far the create is.
    pub state: WriteState,
}

/// The creates that wait, and the last create for the view.
#[derive(Default)]
struct Queue {
    waiting: VecDeque<CreateJob>,
    last: Option<Arc<CreateJob>>,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    asked: Notify,
}

impl Shared {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Shows the end of a job, when it is the last one.
    fn end(&self, job: CreateJob) {
        let mut queue = self.queue();
        if queue.last.as_ref().is_some_and(|last| last.number == job.number) {
            queue.last = Some(Arc::new(job));
        }
    }
}

/// The task of the creates, with the last create. The task stops when this value drops.
pub(super) struct SecretWriter {
    _task: TaskHandle,
    shared: Arc<Shared>,
    number: u64,
}

impl SecretWriter {
    /// Starts the task. It sends through the gateway of the cluster of each Secret.
    pub(super) fn start(tasks: &Tasks, registry: Registry, repaint: Repaint) -> Self {
        let shared = Arc::new(Shared::default());
        let mut runs = 0_u64;
        let task = tasks.start(TaskName::new("secret writer"), Restart::Always(Backoff::default()), {
            let shared = Arc::clone(&shared);
            move || {
                runs += 1;
                serve(Arc::clone(&shared), registry.clone(), repaint.clone(), runs > 1)
            }
        });
        Self { _task: task, shared, number: 0 }
    }

    /// Gives a create to the task. The view shows it as pending at once, or as not sent
    /// when too many creates wait.
    pub(super) fn ask(&mut self, secret: NewSecret) {
        self.number += 1;
        let mut job = CreateJob { number: self.number, secret, state: WriteState::Pending };
        let mut queue = self.shared.queue();
        if queue.waiting.len() < QUEUE_LIMIT {
            queue.waiting.push_back(job.clone());
        } else {
            job.state = WriteState::NotSent(CoreError::invalid(format!("{QUEUE_LIMIT} creates wait for the server")));
        }
        queue.last = Some(Arc::new(job));
        drop(queue);
        self.shared.asked.notify_one();
    }

    /// The last create, for the view.
    pub(super) fn last(&self) -> Option<Arc<CreateJob>> {
        self.shared.queue().last.clone()
    }
}

/// The body of the task: sends each create that waits, in its sequence.
async fn serve(shared: Arc<Shared>, registry: Registry, repaint: Repaint, restarted: bool) -> Result<(), CoreError> {
    // The run before this one can have ended during a create, which can be in effect: the
    // create does not go again, and the list of Secrets shows if it is (design rule 6).
    if restarted {
        let mut queue = shared.queue();
        let Queue { waiting, last } = &mut *queue;
        let sent = |job: &&mut Arc<CreateJob>| {
            job.state == WriteState::Pending && waiting.iter().all(|wait| wait.number != job.number)
        };
        if let Some(last) = last.as_mut().filter(sent) {
            let error = CoreError::invalid("the secret writer started again during the create");
            *last = Arc::new(CreateJob { state: WriteState::Unknown(error), ..CreateJob::clone(last) });
        }
    }
    loop {
        let next = shared.queue().waiting.pop_front();
        let Some(mut job) = next else {
            shared.asked.notified().await;
            continue;
        };
        job.state = match registry.link(&job.secret.cluster) {
            None => WriteState::NotSent(CoreError::invalid(format!("{} is not connected", job.secret.cluster))),
            Some(link) => WriteState::of_create(keys::create(&link.gateway, &job.secret).await),
        };
        shared.end(job);
        repaint.request();
    }
}
