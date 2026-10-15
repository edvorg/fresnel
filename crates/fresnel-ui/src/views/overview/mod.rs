//! The overviews of the Cluster and the Workloads groups of the sidebar (plan sections 11.3
//! and 15; `research/01` section 3.4, `research/02` sections 5.1 and 5.2, `research/11`
//! section 8.3): a block for each selected cluster with the version of its server, its
//! nodes, its pods, its CPU and memory against the allocatable part of its nodes and its
//! warning Events; and the objects of each workload kind by health, in each cluster.
//!
//! A count reads each object of a kind, so a worker thread counts (plan section 7.3): a frame
//! with 10,000 pods in each cluster reads the counts only. The page asks for new counts when
//! a snapshot, a poll of the metrics or the namespace filter changes. The state of each store
//! and of the sampler comes from the view of the frame: a count of a store that is not live
//! has its mark, and a cluster that is not live changes no value of an other one.
//!
//! - `count`: what the worker counts in one cluster. `cluster`: the blocks of the Cluster
//!   overview. `workloads`: the table of the Workloads overview.

mod cluster;
mod count;
mod workloads;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Frame, Label, RichText, ScrollArea, Ui};
use fresnel_core::kinds::{Kind, Sidebar};
use fresnel_core::net::task::{TaskName, Tasks};

use self::count::{Counted, Input};
use crate::shell::Scene;
use crate::theme::{self, PAD, Palette};
use crate::views::detail::related::links_only;
use crate::views::objects::Link;
use crate::widgets::worker::{Cancel, Done, RequestId, SLOW, Wake, Worker};

/// The age of a sample and of an Event changes with no change of a store, so the page asks
/// for a repaint each time this period ends.
const AGE_REPAINT: Duration = Duration::from_secs(1);
/// The most characters of a name of the cluster in a line: a version, a reason, a kind or the
/// name of an object. The API gives most texts no limit, and the layout of a long one takes
/// longer than a frame (`format::short`).
const NAME_LIMIT: usize = 80;
/// The most characters of a longer text in a line: a message of an Event, or an error.
const LINE_LIMIT: usize = 160;

/// The overview of one group of the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Overview {
    Cluster,
    Workloads,
}

impl Overview {
    /// The overview of a group of the sidebar, for the groups that have one.
    pub(crate) fn of(group: Sidebar) -> Option<Self> {
        match group {
            Sidebar::Cluster => Some(Self::Cluster),
            Sidebar::Workloads => Some(Self::Workloads),
            Sidebar::Config | Sidebar::Network | Sidebar::AccessControl | Sidebar::Other => None,
        }
    }

    /// The title of the page, and the name of its entry for a screen reader: the entry
    /// shows "Overview" in its group.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Cluster => "Cluster overview",
            Self::Workloads => "Workloads overview",
        }
    }

    /// The kinds whose full objects the core holds for the page (`Command::ShowKinds`): a
    /// store that is always on costs nothing more, and a ReplicationController has a store
    /// of its metadata only, with no health. The pods are always there.
    pub(crate) fn kinds(self) -> &'static [Kind] {
        match self {
            Self::Cluster => &[Kind::Node, Kind::Event],
            Self::Workloads => &[
                Kind::Deployment,
                Kind::StatefulSet,
                Kind::DaemonSet,
                Kind::ReplicaSet,
                Kind::ReplicationController,
                Kind::Job,
                Kind::CronJob,
            ],
        }
    }
}

/// Where a link of an overview goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Jump {
    /// The panel of an object.
    Object(Link),
    /// The table of a kind with no text filter, and with the "problems" filter when
    /// `problems` is set: its rows are the objects of the count of the link.
    Table { kind: Kind, problems: bool },
}

/// One request to the worker.
#[derive(Clone)]
struct Request {
    inputs: Vec<Input>,
    namespaces: BTreeSet<Arc<str>>,
}

/// The page of the overviews, with its worker thread.
pub(crate) struct OverviewPage {
    worker: Worker<Request, Vec<Counted>>,
    /// The request of the last change, which holds its snapshots: a frame with the same ones
    /// needs no new count.
    asked: Option<Request>,
    id: RequestId,
    /// The counts on the screen, with the identity of their request.
    counted: Option<Done<Vec<Counted>>>,
    /// The moment from which the counts on the screen are not the counts of the last request.
    behind_since: Option<Instant>,
}

impl OverviewPage {
    /// Starts the worker thread. `wake` asks for a repaint for new counts.
    pub(crate) fn new(tasks: &Tasks, wake: Wake) -> Self {
        let compute =
            |request: &Request, cancel: &Cancel<'_>| count::count(&request.inputs, &request.namespaces, cancel);
        let worker = Worker::start(tasks, TaskName::new("overview counts"), wake, compute);
        Self { worker, asked: None, id: RequestId::default(), counted: None, behind_since: None }
    }

    /// Draws the overview. Gives the target of a link that the user clicked.
    pub(crate) fn show(&mut self, ui: &mut Ui, scene: &Scene<'_>, overview: Overview) -> Option<Jump> {
        let palette = theme::palette(ui);
        self.request(scene);
        self.take_counts();
        let counted = self.counted.as_ref().map_or(&[][..], |done| done.output.as_slice());
        let jump = ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            Frame::new().inner_margin(PAD * 2.0).show(ui, |ui| {
                // The counts are links: a text to select would take the role of a link.
                links_only(ui);
                self.title(ui, palette, scene, overview);
                match overview {
                    Overview::Cluster => cluster::show(ui, scene, counted),
                    Overview::Workloads => workloads::show(ui, scene, counted),
                }
            })
        });
        // The frame at the end of `SLOW` tells that the counts take long.
        let slow = self.behind_since.and_then(|since| SLOW.checked_sub(since.elapsed()));
        ui.ctx().request_repaint_after(slow.map_or(AGE_REPAINT, |slow| slow.min(AGE_REPAINT)));
        jump.inner.inner
    }

    /// Takes the newest counts of the worker. Counts of an other filter, or of the time
    /// before a rest, do not fit the page; counts of this filter for older snapshots do.
    fn take_counts(&mut self) {
        if let Some(done) = self.worker.take() {
            let fits = done.id.query == self.id.query;
            let old = if fits { self.counted.replace(done).map(|old| old.output) } else { Some(done.output) };
            if let Some(old) = old {
                self.worker.discard(old);
            }
        }
        let behind = self.counted.as_ref().is_none_or(|done| done.id != self.id);
        self.behind_since = behind.then(|| self.behind_since.unwrap_or_else(Instant::now));
    }

    /// True when the title tells that new counts are on their way: at once while the page
    /// has no counts of its namespace filter, and after `SLOW` for new snapshots. A busy
    /// cluster gives new snapshots each few frames, and a note at each one would flicker.
    fn counting(&self) -> bool {
        match &self.counted {
            Some(done) if done.id.query == self.id.query => {
                self.behind_since.is_some_and(|since| since.elapsed() >= SLOW)
            }
            Some(_) | None => true,
        }
    }

    /// Asks for counts of the snapshots of this frame when they changed. A new namespace
    /// filter is a new query: the count in progress stops, as its counts have no use.
    fn request(&mut self, scene: &Scene<'_>) {
        let inputs: Vec<Input> = scene
            .view
            .clusters
            .iter()
            .filter(|row| row.selected)
            .filter_map(|row| {
                let data = row.data.as_ref()?;
                let (pods, objects, metrics) = (data.pods.clone(), data.objects.clone(), data.metrics.clone());
                Some(Input { cluster: row.id.clone(), pods, objects, metrics })
            })
            .collect();
        let (same_filter, same_data) = self.asked.as_ref().map_or((true, false), |asked| {
            let same = asked.inputs.len() == inputs.len()
                && asked.inputs.iter().zip(&inputs).all(|(before, now)| before.is_same(now));
            (asked.namespaces == *scene.namespaces, same)
        });
        self.id.query += u64::from(!same_filter);
        if !same_filter || !same_data {
            self.id.data += 1;
            let request = Request { inputs, namespaces: scene.namespaces.clone() };
            self.worker.submit(self.id, request.clone());
            self.asked = Some(request);
        }
    }

    /// The title, the state of the counts and the namespace filter.
    fn title(&self, ui: &mut Ui, palette: &Palette, scene: &Scene<'_>, overview: Overview) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(overview.title()).size(15.0).strong());
            // A worker that ended has a state of its own, so old counts never look current.
            match self.worker.failure().filter(|_| self.behind_since.is_some()) {
                Some(failure) => {
                    let text = format!("The counts are not current: the worker thread {failure}");
                    ui.add(Label::new(RichText::new(text).color(palette.error)).truncate());
                }
                None if self.counting() => {
                    ui.label(RichText::new("counting…").color(palette.text_faint));
                }
                None => {}
            }
        });
        if !scene.namespaces.is_empty() {
            let names: Vec<&str> = scene.namespaces.iter().map(|name| &**name).collect();
            let text = format!("The objects of the namespaces {} only: the namespace filter.", names.join(", "));
            ui.label(RichText::new(text).color(palette.text_muted));
        }
        ui.add_space(PAD);
    }

    /// Gives up the counts and the snapshots, for a page that is not painted. The next
    /// request has a new query, so no count of before fits it: the page drops the result of
    /// a count that was in progress at the rest.
    pub(crate) fn rest(&mut self) {
        if self.asked.is_some() || self.counted.is_some() {
            self.worker.clear();
            if let Some(old) = self.counted.take() {
                self.worker.discard(old.output);
            }
            (self.asked, self.behind_since) = (None, None);
            self.id.query += 1;
        }
    }
}
