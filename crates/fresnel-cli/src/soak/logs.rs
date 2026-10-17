//! The log view of the soak: the pods of a Deployment write lines "soak <number>", and each
//! source must give each number of one run of its container. A number that does not come is
//! a lost line (plan section 15, M9). A container that starts again begins a new run.
//!
//! A task reads the view at each copy of the core: the ring of the soak is small, and a read
//! each minute would miss the lines that left it. A Deployment of the same name with a new
//! UID, as after a recreate of the cluster, gets a new view, as a user opens it again.

use std::collections::HashMap;
use std::convert::Infallible;
use std::ops::AddAssign;
use std::sync::{Arc, Mutex, PoisonError};

use fresnel_core::app::{Command, Core};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{LineKind, LogOptions, LogRequest, LogView};
use fresnel_core::net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use fresnel_core::store::ObjectKey;
use serde::Serialize;
use tokio::sync::watch;

use super::look::Look;

/// The start of each line of the pods of the soak.
const PREFIX: &str = "soak ";

/// What the lines of one minute gave.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct LogCounts {
    /// Lines with a number.
    pub(super) lines: u64,
    /// Marker lines of the app: a stream or a container that started again.
    pub(super) markers: u64,
    /// Numbers that did not come within one run of a container.
    pub(super) lost: u64,
    /// Numbers that came two times.
    pub(super) twice: u64,
    /// Containers that started again with a new count.
    pub(super) runs: u64,
    /// Lines that left the ring of the view before the soak read them.
    pub(super) unread: u64,
}

impl AddAssign for LogCounts {
    fn add_assign(&mut self, more: Self) {
        self.lines += more.lines;
        self.markers += more.markers;
        self.lost += more.lost;
        self.twice += more.twice;
        self.runs += more.runs;
        self.unread += more.unread;
    }
}

/// The counts since the last minute, which the task adds to.
pub(super) type Counts = Arc<Mutex<LogCounts>>;

/// The view and the counts that the task follows.
pub(super) struct Follow {
    pub(super) core: Arc<Core>,
    pub(super) cluster: ClusterId,
    /// The namespace and the name of the Deployment.
    pub(super) target: (String, String),
}

/// Starts the task of the log view. It stops when the handle drops.
pub(super) fn start(tasks: &Tasks, follow: Follow, looks: &watch::Receiver<Arc<Look>>) -> (TaskHandle, Counts) {
    let (counts, follow, looks) = (Counts::default(), Arc::new(follow), looks.clone());
    let shared = Arc::clone(&counts);
    let task = tasks.start(TaskName::new("soak log view"), Restart::Always(Backoff::default()), move || {
        read_each_copy(Arc::clone(&follow), looks.clone(), Arc::clone(&shared))
    });
    (task, counts)
}

async fn read_each_copy(
    follow: Arc<Follow>,
    mut looks: watch::Receiver<Arc<Look>>,
    counts: Counts,
) -> Result<(), Infallible> {
    let (mut shown, mut check): (Option<Arc<str>>, LogCheck) = (None, LogCheck::default());
    loop {
        let look = Arc::clone(&looks.borrow_and_update());
        let uid = deployment(&look, &follow.target);
        if let Some(uid) = uid.filter(|uid| shown.as_ref() != Some(uid)) {
            let (namespace, name) = &follow.target;
            let request = LogRequest {
                cluster: follow.cluster.clone(),
                kind: Kind::Deployment,
                key: ObjectKey::new(namespace, name),
                uid: Arc::clone(&uid),
                options: LogOptions::default(),
            };
            follow.core.send(Command::ShowLogs(vec![request]));
            (shown, check) = (Some(uid), LogCheck::default());
        }
        let data = look.row.as_ref().and_then(|row| row.data.as_ref());
        let views = data.into_iter().flat_map(|data| data.logs.iter());
        if let Some(view) = views.into_iter().find(|view| Some(&view.request.uid) == shown.as_ref()) {
            let read = check.read(view);
            *counts.lock().unwrap_or_else(PoisonError::into_inner) += read;
        }
        // The sender lives as long as the soak, and the soak stops this task first.
        if looks.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// The UID of the Deployment in its store now.
fn deployment(look: &Look, (namespace, name): &(String, String)) -> Option<Arc<str>> {
    let store = look.store(Kind::Deployment)?;
    let version = store.versions.get(&(namespace.as_str().into(), name.as_str().into()))?;
    Some(Arc::clone(&version.uid))
}

/// The lines that the soak read of one view.
#[derive(Debug, Default)]
pub(super) struct LogCheck {
    /// The number of the next line of the view to read. `None` before the first read.
    next: Option<u64>,
    /// The last number of each source.
    last: HashMap<u32, u64>,
}

impl LogCheck {
    /// Reads the lines that came since the last read.
    pub(super) fn read(&mut self, view: &LogView) -> LogCounts {
        let lines = &view.lines;
        let wanted = self.next.unwrap_or(lines.first());
        let from = wanted.max(lines.first());
        let mut counts = LogCounts { unread: from - wanted, ..LogCounts::default() };
        // The lines that left the ring before this read can be of each source: a number after
        // them that does not follow the last one is no lost line.
        if counts.unread > 0 {
            self.last.clear();
        }
        for (_, line) in lines.range(from, lines.end()) {
            if line.kind == LineKind::Marker {
                counts.markers += 1;
                continue;
            }
            let Some(number) = line.text.strip_prefix(PREFIX).and_then(|number| number.trim().parse::<u64>().ok())
            else {
                continue;
            };
            counts.lines += 1;
            match self.last.insert(line.source, number) {
                Some(last) if number == last + 1 => {}
                Some(last) if number > last => counts.lost += number - last - 1,
                Some(last) if number == last => counts.twice += 1,
                Some(_) => counts.runs += 1,
                None => {}
            }
        }
        self.next = Some(lines.end());
        counts
    }
}

#[cfg(test)]
mod tests {
    use fresnel_core::logs::{Lines, LogLine, LogRequest, PodsState};

    use super::*;

    fn line(source: u32, text: &str, kind: LineKind) -> LogLine {
        LogLine { time: 0, source, text: text.into(), spans: None, kind, late: false, cut: false }
    }

    fn view(first: u64, lines: Vec<LogLine>) -> LogView {
        LogView {
            request: LogRequest {
                cluster: fresnel_core::cluster::ClusterId::new("k3s"),
                kind: fresnel_core::kinds::Kind::Deployment,
                key: fresnel_core::store::ObjectKey::new("soak", "logger"),
                uid: "u-1".into(),
                options: fresnel_core::logs::LogOptions::default(),
            },
            pods: PodsState::Following,
            sources: Vec::new().into(),
            lines: Lines::new(first, lines),
            dropped: 0,
            bytes: 0,
            unstreamed: 0,
            restarts: 0,
        }
    }

    #[test]
    fn each_source_counts_its_lost_numbers_its_runs_and_the_markers() {
        let log = |source, number: u64| line(source, &format!("soak {number}"), LineKind::Log);
        let mut check = LogCheck::default();
        let first =
            vec![log(0, 7), log(1, 0), log(0, 8), line(0, "marker: the stream started again", LineKind::Marker)];
        assert_eq!(check.read(&view(0, first.clone())), LogCounts { lines: 3, markers: 1, ..LogCounts::default() });

        let mut more = first;
        more.extend([log(0, 11), log(1, 1), log(1, 1), log(0, 0), line(1, "other text", LineKind::Log)]);
        let counts = LogCounts { lines: 4, lost: 2, twice: 1, runs: 1, ..LogCounts::default() };
        assert_eq!(check.read(&view(0, more.clone())), counts);
        assert_eq!(check.read(&view(0, more)), LogCounts::default(), "no new line");
    }

    #[test]
    fn lines_that_left_the_ring_before_the_read_count_as_unread_and_not_as_lost() {
        let mut check = LogCheck::default();
        let log = |number: u64| line(0, &format!("soak {number}"), LineKind::Log);
        check.read(&view(0, vec![log(0), log(1)]));
        let counts = check.read(&view(5, vec![log(5), log(6)]));
        assert_eq!((counts.unread, counts.lost, counts.lines), (3, 0, 2));
        let counts = check.read(&view(5, vec![log(5), log(6), log(9)]));
        assert_eq!((counts.unread, counts.lost, counts.lines), (0, 2, 1), "a gap after the read");
    }
}
