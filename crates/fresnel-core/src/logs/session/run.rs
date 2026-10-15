//! The task of one log view.
//!
//! The task reads the pods of the view and starts a source for each new stream, up to the
//! limit of the view. It takes the reports of the sources and moves their lines through
//! the merge into the ring. It publishes the view 30 ms after a change at most, so the
//! lines of one second cost some publishes and not one for each line.

use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use fresnel_net::task::TaskHandle;
use k8s_openapi::api::core::v1::Pod;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

use super::SessionParts;
use super::targets::{Want, wanted};
use crate::error::CoreError;
use crate::index::{Index, Owners};
use crate::logs::line::LineKind;
use crate::logs::merge::Merge;
use crate::logs::request::{LogRequest, Origin};
use crate::logs::ring::Ring;
use crate::logs::source::{self, Opener, Report, SourceParts, Stream};
use crate::logs::view::{LogView, PodsState, SourceState, SourceView};
use crate::published::Publisher;
use crate::store::Snapshot;

/// The most time between a change and its publish.
const PUBLISH_PERIOD: Duration = Duration::from_millis(30);
/// The reports that wait for the task of the view. A source waits when they are full, and
/// its stream then waits in the flow control of its connection.
const REPORTS: usize = 64;
/// The most sources of one view over its life. A workload whose pods come and go gets a new
/// source for each new pod; the sources that ended stay in the view with their state.
const SOURCE_LIMIT: usize = 4096;
/// The lines that wait in the merge hold one eighth of the bytes of the ring at most: they
/// are a part of the memory of the view, and a fast source must not fill it while a silent
/// one holds the merge.
const MERGE_SHARE: usize = 8;

/// One run of the task.
pub(super) async fn run<O: Opener>(
    request: Arc<LogRequest>,
    parts: Arc<SessionParts>,
    opener: O,
    publisher: Publisher<LogView>,
    restarts: u32,
) -> Result<(), CoreError> {
    // The view drops before the channel: each source stops before its sender would fail.
    let (sender, mut reports) = mpsc::channel(REPORTS);
    let (mut pods, mut owners) = (parts.pods.clone(), parts.owners.clone());
    let mut view = View::new(&request, &parts, opener, sender, restarts);
    loop {
        if view.follow_again {
            view.follow(&parts, &pods.read(), &owners.read());
        }
        view.drain(Instant::now());
        view.publish(&publisher, Instant::now());
        let wake = [view.merge.deadline(), view.publish_at()].into_iter().flatten().min();
        let timer = async {
            match wake {
                Some(at) => sleep_until(at).await,
                None => pending().await,
            }
        };
        tokio::select! {
            report = reports.recv() => {
                if let Some(report) = report {
                    view.take(report, Instant::now());
                }
            }
            () = pods.changed() => view.follow_again = true,
            () = owners.changed() => view.follow_again = true,
            () = timer => {}
        }
        // Each report that waits goes into the same publish.
        while let Ok(report) = reports.try_recv() {
            view.take(report, Instant::now());
        }
    }
}

/// One source of the view.
struct Running {
    want: Want,
    view: SourceView,
    /// The task of the source. `None` after its final state: the task stopped then.
    task: Option<TaskHandle>,
}

/// The state of one run of the task of a view.
struct View<'a, O> {
    /// The first field: the tasks of the sources stop first.
    sources: Vec<Running>,
    request: &'a LogRequest,
    opener: O,
    sender: mpsc::Sender<Report>,
    merge: Merge,
    ring: Ring,
    /// The default container of the view, from the first time that its pods are known.
    default: Option<Arc<str>>,
    pods: PodsState,
    unstreamed: usize,
    /// True when the pods changed or a source ended: the streams of the view are new then.
    follow_again: bool,
    /// True when the view has a change that it did not publish.
    changed: bool,
    published: Option<Instant>,
    restarts: u32,
}

impl<'a, O: Opener> View<'a, O> {
    fn new(
        request: &'a LogRequest,
        parts: &SessionParts,
        opener: O,
        sender: mpsc::Sender<Report>,
        restarts: u32,
    ) -> Self {
        Self {
            sources: Vec::new(),
            request,
            opener,
            sender,
            merge: Merge::new(parts.settings.hold, parts.settings.ring.bytes / MERGE_SHARE),
            ring: Ring::new(parts.settings.ring),
            default: None,
            pods: PodsState::Loading,
            unstreamed: 0,
            follow_again: true,
            changed: true,
            published: None,
            restarts,
        }
    }

    /// Starts a source for each stream that the view wants and does not have, while the
    /// view has free streams. A source that the pods do not want any more ends by itself:
    /// its pod is gone.
    fn follow(&mut self, parts: &SessionParts, pods: &Snapshot<Pod>, owners: &Index<Owners>) {
        self.follow_again = false;
        let wanted = wanted(self.request, pods, owners, &mut self.default);
        self.changed |= self.pods != wanted.pods;
        self.pods = wanted.pods;
        let mut unstreamed = 0;
        for want in wanted.streams {
            if self.sources.iter().any(|running| running.want == want) {
                continue;
            }
            let streams = self.sources.iter().filter(|running| running.task.is_some()).count();
            if streams >= parts.settings.streams || self.sources.len() >= SOURCE_LIMIT {
                unstreamed += 1;
                continue;
            }
            self.start(parts, want);
        }
        self.changed |= self.unstreamed != unstreamed;
        self.unstreamed = unstreamed;
    }

    fn start(&mut self, parts: &SessionParts, want: Want) {
        let Ok(index) = u32::try_from(self.sources.len()) else {
            return;
        };
        let options = &self.request.options;
        let stream =
            Stream { index, pod: want.pod.clone(), uid: Arc::clone(&want.uid), container: Arc::clone(&want.container) };
        let source = SourceParts {
            stream,
            previous: options.previous,
            first: Origin::Options { tail: options.tail, since: options.since },
            opener: self.opener.clone(),
            pods: parts.pods.clone(),
            signal: parts.signal.clone(),
            reports: self.sender.clone(),
            retry: parts.settings.retry,
            clock: parts.clock.clone(),
        };
        let task = source::start(&parts.tasks, &self.request.cluster, parts.restart, parts.settings.line_bytes, source);
        self.merge.set_open(index, true);
        let view = SourceView {
            pod: want.pod.clone(),
            container: Arc::clone(&want.container),
            state: SourceState::Connecting,
            lines: 0,
            repeated: 0,
        };
        self.sources.push(Running { want, view, task: Some(task) });
        self.changed = true;
    }

    /// Takes one report of a source.
    fn take(&mut self, report: Report, now: Instant) {
        self.changed = true;
        match report {
            Report::Lines { source, lines, repeated } => {
                if let Some(running) = self.running(source) {
                    running.view.lines += lines.iter().filter(|line| line.kind == LineKind::Log).count() as u64;
                    running.view.repeated = repeated;
                }
                self.merge.push(source, lines, now);
            }
            Report::State { source, state } => {
                self.merge.set_open(source, state.is_open());
                // The source waits with no end in a final state. Its task stops, and its
                // stream is free for an other source.
                self.follow_again |= state.is_final();
                if let Some(running) = self.running(source) {
                    if state.is_final() {
                        running.task = None;
                    }
                    running.view.state = state;
                }
            }
        }
    }

    fn running(&mut self, source: u32) -> Option<&mut Running> {
        self.sources.get_mut(usize::try_from(source).ok()?)
    }

    /// Moves each line that can go into the ring.
    fn drain(&mut self, now: Instant) {
        let (ring, changed) = (&mut self.ring, &mut self.changed);
        self.merge.drain(now, &mut |line| {
            ring.push(line);
            *changed = true;
        });
    }

    /// The time of the next publish of a change.
    fn publish_at(&self) -> Option<Instant> {
        self.changed.then(|| self.published.map_or_else(Instant::now, |at| at + PUBLISH_PERIOD))
    }

    fn publish(&mut self, publisher: &Publisher<LogView>, now: Instant) {
        if !self.changed || self.published.is_some_and(|at| now < at + PUBLISH_PERIOD) {
            return;
        }
        publisher.publish(LogView {
            request: self.request.clone(),
            pods: self.pods.clone(),
            sources: self.sources.iter().map(|running| running.view.clone()).collect(),
            lines: self.ring.lines(),
            dropped: self.ring.dropped(),
            bytes: self.ring.bytes(),
            unstreamed: self.unstreamed,
            restarts: self.restarts,
        });
        (self.changed, self.published) = (false, Some(now));
    }
}
