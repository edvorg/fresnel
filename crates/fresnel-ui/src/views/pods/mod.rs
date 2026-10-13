//! The Pods view: one table with the pods of each selected cluster (plan sections 6
//! and 11). A row carries the identity of its cluster, and a row from a store that is not
//! live has a freshness mark.
//!
//! - `rows`: the rows of the table. `follow`: the changes of the stores from frame to frame.
//! - `toolbar`: the count and the filters. `notes`: the state of the store of each cluster.
//! - `detail`: the panel of the selected pod.

mod detail;
mod follow;
mod notes;
mod rows;
mod toolbar;

use std::time::Duration;

use egui::{CentralPanel, Frame, Panel, Ui};
use fresnel_core::app::ClusterRow;
use fresnel_core::net::task::Tasks;

use self::detail::Subject;
use self::follow::Follower;
use self::rows::{ClusterMark, PodFilter, PodModel, PodScene};
use crate::format;
use crate::shell::Scene;
use crate::theme::{self, Palette};
use crate::widgets::table::worker::Wake;
use crate::widgets::table::{Act, Table};

/// The age of a pod changes with no change of a store, so the view asks for a repaint
/// each time this period ends while it shows a row.
const AGE_REPAINT: Duration = Duration::from_secs(1);

pub(crate) struct PodsView {
    table: Table<PodModel>,
    follower: Follower,
    filter: PodFilter,
    /// The pod of the detail panel. `None` while the panel is closed.
    detail: Option<Subject>,
}

impl PodsView {
    /// Starts the worker thread of the table. `wake` asks for a repaint for new rows.
    pub(crate) fn new(tasks: &Tasks, wake: Wake) -> Self {
        Self {
            table: Table::new(tasks, wake, "Pods", PodModel),
            follower: Follower::default(),
            filter: PodFilter::default(),
            detail: None,
        }
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, scene: &Scene<'_>) {
        let palette = theme::palette(ui);
        self.read_stores(scene);
        // The panel comes first, so it has the full height of the view.
        if let Some(subject) = &self.detail {
            let cluster = scene.view.clusters.iter().find(|row| row.id == subject.id.cluster);
            let panel = Panel::right("pod_detail").resizable(true).default_size(420.0).size_range(280.0..=720.0);
            let panel = panel.frame(Frame::new().fill(palette.bg_surface));
            if panel.show(ui, |ui| detail::show(ui, scene, subject, cluster)).inner {
                self.detail = None;
                self.table.clear_selection();
            }
        }
        toolbar::show(ui, palette, &mut self.filter, &self.table);
        // After the bar: a filter that the user changed in this frame asks for its rows now.
        self.table.request(&self.filter, self.follower.serial(), || self.follower.data());
        notes::show(ui, palette, scene);
        let cells = pod_scene(scene, palette);
        let acted =
            CentralPanel::no_frame().show(ui, |ui| self.table.show(ui, &cells, || empty_text(scene, &self.filter)));
        match acted.inner {
            Some(Act::Cursor) => {
                self.detail = self.table.cursor_row().map(Subject::of);
                // The panel is before the table in a frame, so the next frame shows it.
                ui.ctx().request_repaint();
            }
            Some(Act::Cleared) => self.detail = None,
            None => {}
        }
        if self.table.rows().is_some_and(|rows| !rows.rows.is_empty()) {
            ui.ctx().request_repaint_after(AGE_REPAINT);
        }
    }

    /// Reads the snapshot of each selected cluster for this frame, and drops each pod
    /// that went away from the selection. The number of the snapshots grows when the
    /// objects changed, so the table asks its worker for new rows then.
    fn read_stores(&mut self, scene: &Scene<'_>) {
        let selected = scene.view.clusters.iter().filter(|row| row.selected);
        let now = selected.filter_map(|row| Some((row.id.clone(), row.data.as_ref()?.pods.clone()))).collect();
        let gone = self.follower.step(now);
        if !gone.is_empty() {
            self.table.forget(|id| gone.has(id));
        }
        // The panel of a pod whose cluster left the view closes. A deleted pod stays, as deleted.
        match self.detail.as_mut().map(|subject| (self.follower.snapshot(&subject.id.cluster), subject)) {
            Some((Some(pods), subject)) => subject.refresh(pods),
            Some((None, _)) => self.detail = None,
            None => {}
        }
    }
}

/// What the cells show of each selected cluster: its color and how old its rows are.
fn pod_scene(scene: &Scene<'_>, palette: &'static Palette) -> PodScene {
    let mark = |row: &ClusterRow| {
        let old = match &row.data {
            Some(data) => notes::old_since(&data.pods, scene.clock),
            None => Some("old: the cluster is not connected".to_owned()),
        };
        (row.id.clone(), ClusterMark { color: scene.colors.color(&row.id, palette), old })
    };
    let marks = scene.view.clusters.iter().filter(|row| row.selected).map(mark).collect();
    PodScene { palette, clock: scene.clock.clone(), marks }
}

/// Why the table has no row. Only a store with a complete list knows that a cluster has
/// no pod, so the text never says "no pod" for a list that did not arrive (design rule 5).
fn empty_text(scene: &Scene<'_>, filter: &PodFilter) -> String {
    let selected: Vec<&ClusterRow> = scene.view.clusters.iter().filter(|row| row.selected).collect();
    let pods = |row: &&ClusterRow| row.data.as_ref().map_or(0, |data| data.pods.items.len());
    let total: usize = selected.iter().map(pods).sum();
    let loaded = selected.iter().all(|row| row.data.as_ref().is_some_and(|data| data.pods.listed));
    if selected.is_empty() {
        "No cluster is selected. Select a cluster in the top bar.".to_owned()
    } else if total > 0 && *filter != PodFilter::default() {
        format!("No pod passes the filters. The selected clusters have {}.", format::count(total as u64, "pod"))
    } else if loaded {
        "The selected clusters have no pod.".to_owned()
    } else {
        "The pods are not loaded. The notes above give the state of each cluster.".to_owned()
    }
}
