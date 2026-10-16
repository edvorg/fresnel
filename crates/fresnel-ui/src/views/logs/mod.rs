//! The log views of the dock (plan section 12.7): one tab for each object whose logs the
//! user opened, with the color of its cluster in its title.
//!
//! The shell keeps the tabs and names their requests to the core (`Command::ShowLogs`); the
//! core runs one session for each one and gives its lines in the data of the cluster. New
//! options give a new request: the core starts a new session, and the tab starts again.
//!
//! - `tab`: one view. `bar`: its options, its search and its saves. `sources`: the state of
//!   each source and the notes of the view. `pane`: the rows of the lines.
//! - `find`, `needle`: the search and the filter on a worker thread; the editor finds with
//!   `needle` too. `copy`: a copy of the selection. `full`: the window of a long line.

mod bar;
mod copy;
mod find;
mod full;
pub(crate) mod needle;
mod pane;
mod sources;
mod tab;

use std::sync::Arc;

use egui::Ui;
use fresnel_core::app::{Command, CoreView};
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{Containers, LogOptions, LogRequest};
use fresnel_core::net::task::Tasks;
use fresnel_core::ops::Target;

use crate::shell::{Scene, Title};
use crate::widgets::worker::Wake;
use tab::{LogTab, Threads};

/// The most tabs of logs. A new one takes the place of the oldest: each tab holds the
/// streams of its pods.
const MOST: usize = 8;

/// What the panel asks for: the logs of its object, of one container, of the instance
/// before the last restart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LogAsk {
    /// One container. `None` for the default container of the view.
    pub(crate) container: Option<Arc<str>>,
    pub(crate) previous: bool,
}

impl LogAsk {
    /// The request of the logs of an object.
    pub(crate) fn request(&self, target: &Target) -> LogRequest {
        let containers = self.container.clone().map_or(Containers::Default, Containers::Named);
        LogRequest {
            cluster: target.cluster.clone(),
            kind: target.kind,
            key: target.key.clone(),
            uid: Arc::clone(&target.uid),
            options: LogOptions { containers, previous: self.previous, ..LogOptions::default() },
        }
    }
}

/// The tabs of logs.
pub(crate) struct LogTabs {
    tabs: Vec<LogTab>,
    last: u64,
    /// The requests that the core got last.
    sent: Vec<LogRequest>,
    tasks: Tasks,
    wake: Wake,
}

impl LogTabs {
    pub(crate) fn new(tasks: &Tasks, wake: Wake) -> Self {
        Self { tabs: Vec::new(), last: 0, sent: Vec::new(), tasks: tasks.clone(), wake }
    }

    /// Opens the view of a request, or the tab that has it. Gives the identity of its tab.
    pub(crate) fn open(&mut self, request: LogRequest) -> u64 {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.request == request) {
            return tab.id;
        }
        if self.tabs.len() >= MOST {
            self.tabs.remove(0);
        }
        self.last += 1;
        self.tabs.push(LogTab::new(self.last, request));
        self.last
    }

    pub(crate) fn close(&mut self, id: u64) {
        self.tabs.retain(|tab| tab.id != id);
    }

    pub(crate) fn has(&self, id: u64) -> bool {
        self.tabs.iter().any(|tab| tab.id == id)
    }

    /// The title of each tab, in the sequence of their open.
    pub(crate) fn titles(&self, view: &CoreView) -> Vec<Title> {
        self.tabs.iter().map(|tab| tab.title(view)).collect()
    }

    /// Draws the tab `id`.
    pub(crate) fn show(&mut self, id: u64, ui: &mut Ui, scene: &mut Scene<'_>) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.show(ui, scene, &Threads { tasks: &self.tasks, wake: &self.wake });
        }
    }

    /// The command that names the views of the tabs to the core, when they changed.
    pub(crate) fn command(&mut self) -> Option<Command> {
        let requests: Vec<LogRequest> = self.tabs.iter().map(|tab| tab.request.clone()).collect();
        if requests == self.sent {
            return None;
        }
        self.sent.clone_from(&requests);
        Some(Command::ShowLogs(requests))
    }
}

/// "Logs web-1", "Logs Deployment web · app (previous)": the words of a title.
fn title_text(request: &LogRequest) -> String {
    let object = match request.kind {
        Kind::Pod => request.key.name.to_string(),
        kind => format!("{} {}", kind.info().name, request.key.name),
    };
    let container = match &request.options.containers {
        Containers::Default => String::new(),
        Containers::Named(name) => format!(" · {name}"),
        Containers::All => " · all containers".to_owned(),
    };
    let previous = if request.options.previous { " (previous)" } else { "" };
    format!("Logs {}{container}{previous}", crate::format::short(&object, 40))
}
