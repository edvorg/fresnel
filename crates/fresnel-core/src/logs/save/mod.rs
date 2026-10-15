//! A log view as a file (plan section 12.7, download): the lines that the view holds, or
//! the full log of each of its sources, which the server sends again.
//!
//! Each save is a task of its own with a state that the view shows. It writes into a new
//! file of the folder of downloads and never over a file that exists. Each write runs on a
//! thread for blocking calls, so a slow disk holds no task of the runtime. A cancel ends the
//! save and removes its file: a part of a log would look like a full one.
//!
//! - `text`: a line of a file and the name of a file. `write`: the task of one save.

mod text;
mod write;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use fresnel_net::WallClock;
use fresnel_net::task::{Restart, TaskHandle, TaskName, Tasks};
use tokio::sync::watch;

use super::request::LogRequest;
use super::source::Opener;
use super::view::LogView;
use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::published::{Bin, Held, Notice, Publisher};
use write::{Content, Job};

/// The most saves of one cluster in the list. The oldest save that ended leaves it first; a
/// save that writes stays.
const KEPT: usize = 16;

/// A save that the user asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogSave {
    /// The view whose log the file gets.
    pub request: LogRequest,
    /// What the file gets.
    pub part: SavePart,
    /// True to write the time of each line, as the view shows the times.
    pub times: bool,
}

/// What a file gets of a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavePart {
    /// The lines that the view holds now, with the markers of the app.
    Buffer,
    /// The full log of each source as the server keeps it now: one request with no follow
    /// and no tail for each container.
    Full,
}

/// One save with its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveView {
    /// A number that grows with each save of the cluster.
    pub number: u64,
    /// What the user asked for.
    pub save: LogSave,
    /// The file. `None` before the task made it.
    pub file: Option<PathBuf>,
    /// How far the save is.
    pub state: SaveState,
}

/// How far a save is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    /// The file grows.
    Writing {
        /// The bytes in the file now.
        bytes: u64,
    },
    /// The file is complete.
    Done {
        /// The size of the file.
        bytes: u64,
        /// Each source whose log the server did not give, with the reason. The file has a
        /// line that tells it in the place of that log.
        skipped: Vec<String>,
    },
    /// The file could not be made or written. A file that the save made is gone.
    Failed(CoreError),
    /// The user stopped the save, and its file is gone.
    Cancelled,
}

impl SaveState {
    /// True for a state that does not change any more.
    pub fn is_final(&self) -> bool {
        !matches!(self, Self::Writing { .. })
    }
}

/// What the saves of one cluster need.
#[derive(Clone)]
pub(crate) struct SaveParts {
    pub(crate) tasks: Tasks,
    /// The folder of the files. The first save makes it.
    pub(crate) folder: PathBuf,
    /// The time in the name of a file.
    pub(crate) clock: WallClock,
    pub(crate) bin: Bin,
    /// Hears each new state of a save, for the repaint.
    pub(crate) notice: Notice,
}

/// The saves of one cluster. Their tasks stop when this value drops.
pub(crate) struct Saves {
    parts: SaveParts,
    last: u64,
    saves: VecDeque<Running>,
}

/// One save in the list. The task stops when this value drops.
struct Running {
    publisher: Publisher<SaveView>,
    cancel: watch::Sender<bool>,
    /// `None` for a save that the core refused at once.
    _task: Option<TaskHandle>,
}

impl Saves {
    pub(crate) fn new(parts: SaveParts) -> Self {
        Self { parts, last: 0, saves: VecDeque::new() }
    }

    /// Starts a save of the view of this moment: its lines, or the full logs of its sources
    /// from the server of `gateway`.
    pub(crate) fn start(&mut self, save: LogSave, view: &LogView, gateway: &Gateway) {
        self.start_with(save, view, gateway.clone());
    }

    /// The same with an other opener of streams: a test gives a script.
    pub(in crate::logs) fn start_with<O: Opener>(&mut self, save: LogSave, view: &LogView, opener: O) {
        self.last += 1;
        let content = match save.part {
            SavePart::Buffer => Content::Buffer { lines: view.lines.clone(), sources: Arc::clone(&view.sources) },
            SavePart::Full => {
                Content::Full { sources: Arc::clone(&view.sources), previous: save.request.options.previous }
            }
        };
        let first =
            SaveView { number: self.last, save: save.clone(), file: None, state: SaveState::Writing { bytes: 0 } };
        let publisher = Publisher::new(first, self.parts.bin.clone(), self.parts.notice.clone());
        let (cancel, cancelled) = watch::channel(false);
        let name = TaskName::scoped("log save", save.request.cluster.as_str());
        let job = Job { save, content, opener, folder: self.parts.folder.clone() };
        let at = self.parts.clock.now();
        let task = self.parts.tasks.start(name, Restart::Never, {
            let publisher = publisher.clone();
            move || write::run(job.clone(), at, publisher.clone(), cancelled.clone())
        });
        self.push(Running { publisher, cancel, _task: Some(task) });
    }

    /// Lists a save that cannot start, with the reason: the view is not open in the cluster.
    pub(crate) fn refuse(&mut self, save: LogSave, reason: CoreError) {
        self.last += 1;
        let view = SaveView { number: self.last, save, file: None, state: SaveState::Failed(reason) };
        let publisher = Publisher::new(view, self.parts.bin.clone(), self.parts.notice.clone());
        self.push(Running { publisher, cancel: watch::Sender::new(false), _task: None });
    }

    fn push(&mut self, running: Running) {
        self.saves.push_back(running);
        while self.saves.len() > KEPT {
            let Some(ended) = self.saves.iter().position(|running| running.publisher.read().state.is_final()) else {
                break;
            };
            drop(self.saves.remove(ended));
        }
    }

    /// Stops the save with this number. A save that ended keeps its state.
    pub(crate) fn cancel(&self, number: u64) {
        let running = self.saves.iter().find(|running| running.publisher.read().number == number);
        if let Some(running) = running {
            running.cancel.send_replace(true);
        }
    }

    /// The saves, the oldest first.
    pub(crate) fn views(&self) -> Arc<[Held<SaveView>]> {
        self.saves.iter().map(|running| running.publisher.read()).collect()
    }
}

#[cfg(test)]
mod tests;
