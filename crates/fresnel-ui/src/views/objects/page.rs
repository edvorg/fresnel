//! The page of one kind: the bar, the notes, the table and the detail panel.

use std::sync::Arc;
use std::time::Duration;

use egui::{CentralPanel, Frame, Panel, RichText, Ui};
use fresnel_core::app::ClusterRow;
use fresnel_core::kinds::{Describe, Kind, Object};
use fresnel_core::net::task::Tasks;

use super::filter::ObjectFilter;
use super::follow::Follower;
use super::notes::{self, Facts, old_since};
use super::{ClusterMark, Link, Nouns, ObjectModel, Page, RowScene, toolbar, uid};
use crate::shell::Scene;
use crate::theme::{self, Palette};
use crate::views::detail::{self, DetailPanel};
use crate::widgets::table::model::ObjectId;
use crate::widgets::table::worker::Wake;
use crate::widgets::table::{Act, Table};

/// The age of an object changes with no change of a store, so the page asks for a repaint
/// each time this period ends while it shows a row.
const AGE_REPAINT: Duration = Duration::from_secs(1);

pub(crate) struct ObjectPage<M: ObjectModel> {
    nouns: Nouns,
    table: Table<M>,
    follower: Follower<M::Object>,
    filter: ObjectFilter,
    /// The object of the detail panel. `None` while the panel is closed.
    detail: Option<DetailPanel<M::Object>>,
    /// The object of a link, until the store of its cluster has a complete list.
    wanted: Option<Link>,
    /// Why the object of the last link has no panel.
    missing: Option<String>,
}

impl<M: ObjectModel> ObjectPage<M> {
    /// Starts the worker thread of the table. `wake` asks for a repaint for new rows.
    pub(crate) fn new(tasks: &Tasks, wake: Wake, model: M) -> Self {
        let nouns = model.nouns();
        Self {
            nouns,
            table: Table::new(tasks, wake, nouns.title, model),
            follower: Follower::default(),
            filter: ObjectFilter::default(),
            detail: None,
            wanted: None,
            missing: None,
        }
    }

    /// Reads the snapshot of each selected cluster for this frame, and drops each object
    /// that went away from the selection. The number of the snapshots grows when the
    /// objects changed, so the table asks its worker for new rows then.
    fn read_stores(&mut self, scene: &Scene<'_>) {
        let selected = scene.view.clusters.iter().filter(|row| row.selected);
        let now = selected.filter_map(|row| Some((row.id.clone(), row.data.as_ref()?.snapshot()?))).collect();
        let gone = self.follower.step(now);
        if !gone.is_empty() {
            self.table.forget(|id| gone.has(id));
        }
        // The panel of an object whose cluster left the view closes. A deleted one stays.
        match self.detail.as_mut().map(|panel| (self.follower.snapshot(&panel.id().cluster), panel)) {
            Some((Some(snapshot), panel)) => panel.refresh(snapshot),
            Some((None, _)) => self.detail = None,
            None => {}
        }
        self.find_wanted(scene);
    }

    /// Opens the panel of the object of a link when its store has it.
    fn find_wanted(&mut self, scene: &Scene<'_>) {
        let Some(link) = &self.wanted else {
            return;
        };
        let name = format!("{} {}", <M::Object as Describe>::ID.info().name, link.key.name);
        let Some(snapshot) = self.follower.snapshot(&link.cluster) else {
            if !scene.view.clusters.iter().any(|row| row.id == link.cluster && row.selected) {
                self.missing = Some(format!("The {name} is of {}, which is not selected.", link.cluster));
                self.wanted = None;
            }
            return;
        };
        match snapshot.items.get(&link.key) {
            Some(object) => {
                let (cluster, key) = (link.cluster.clone(), link.key.clone());
                let id = ObjectId { cluster, key, uid: uid(object.meta()).into() };
                self.table.select(id.clone());
                self.detail = Some(DetailPanel::new(id, Arc::clone(object)));
                (self.wanted, self.missing) = (None, None);
            }
            None if snapshot.listed => {
                self.missing = Some(format!("{} has no {name}.", link.cluster));
                self.wanted = None;
            }
            None => {}
        }
    }

    /// What the cells show of each selected cluster: its color and how old its rows are.
    fn row_scene(&self, scene: &Scene<'_>, palette: &'static Palette) -> RowScene {
        let mark = |row: &ClusterRow| {
            let old = match (&row.data, self.follower.snapshot(&row.id)) {
                (Some(_), Some(snapshot)) => old_since(snapshot, scene.clock),
                (Some(_), None) => Some("old: the store of the full objects starts".to_owned()),
                (None, _) => Some("old: the cluster is not connected".to_owned()),
            };
            (row.id.clone(), ClusterMark { color: scene.colors.color(&row.id, palette), old })
        };
        let marks = scene.view.clusters.iter().filter(|row| row.selected).map(mark).collect();
        RowScene { palette, clock: scene.clock.clone(), marks }
    }
}

/// Why the table has no row. Only a store with a complete list knows that a cluster has no
/// object, so the text never says "none" for a list that did not arrive (design rule 5).
fn empty_text<T: Object>(scene: &Scene<'_>, filter: &ObjectFilter, nouns: &Nouns, follower: &Follower<T>) -> String {
    let Nouns { one, many, .. } = *nouns;
    let selected: Vec<&ClusterRow> = scene.view.clusters.iter().filter(|row| row.selected).collect();
    let store = |row: &&ClusterRow| follower.snapshot(&row.id);
    let total: usize = selected.iter().map(|row| store(row).map_or(0, |snapshot| snapshot.items.len())).sum();
    let loaded = selected.iter().all(|row| store(row).is_some_and(|snapshot| snapshot.listed));
    if selected.is_empty() {
        "No cluster is selected. Select a cluster in the top bar.".to_owned()
    } else if total > 0 && *filter != ObjectFilter::default() {
        format!("No {one} passes the filters. The selected clusters have {}.", nouns.count(total as u64))
    } else if loaded {
        format!("The selected clusters have no {one}.")
    } else {
        format!("The {many} are not loaded. The notes above give the state of each cluster.")
    }
}

impl<M: ObjectModel> Page for ObjectPage<M> {
    fn kind(&self) -> Kind {
        <M::Object as Describe>::ID
    }

    fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>) -> Option<Link> {
        let palette = theme::palette(ui);
        self.read_stores(scene);
        self.table.restore(scene.view.tables.as_deref());
        let mut link = None;
        // The panel comes first, so it has the full height of the page.
        if let Some(panel) = &mut self.detail {
            let view = scene.view;
            let cluster = view.clusters.iter().find(|row| row.id == panel.id().cluster);
            let frame = Panel::right("detail").resizable(true).default_size(420.0).size_range(280.0..=720.0);
            let frame = frame.frame(Frame::new().fill(palette.bg_surface));
            match frame.show(ui, |ui| panel.show(ui, scene, cluster, &self.nouns)).inner {
                detail::Act::Close => {
                    self.detail = None;
                    self.table.clear_selection();
                }
                detail::Act::Open(target) => link = Some(target),
                detail::Act::None => {}
            }
        }
        toolbar::show(ui, palette, &self.nouns, &mut self.filter, &mut self.table);
        // After the bar: a filter that the user changed in this frame asks for its rows now.
        self.table.request(&self.filter, self.follower.serial(), || self.follower.data());
        notes::show(ui, palette, scene, &self.nouns, |cluster| self.follower.snapshot(cluster).map(Facts::of));
        if let Some(missing) = &self.missing {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new(missing).color(palette.warn));
            });
        }
        let cells = self.row_scene(scene, palette);
        let (filter, nouns, follower) = (&self.filter, &self.nouns, &self.follower);
        let empty = || empty_text(scene, filter, nouns, follower);
        let acted = CentralPanel::no_frame().show(ui, |ui| self.table.show(ui, &cells, empty));
        scene.commands.extend(self.table.save_layout(ui));
        match acted.inner {
            Some(Act::Cursor) => {
                let model = self.table.model();
                let row = self.table.cursor_row();
                self.detail = row.map(|row| DetailPanel::new(model.id(row).clone(), Arc::clone(model.object(row))));
                self.missing = None;
                // The panel is before the table in a frame, so the next frame shows it.
                ui.ctx().request_repaint();
            }
            Some(Act::Cleared) => self.detail = None,
            None => {}
        }
        if self.table.rows().is_some_and(|rows| !rows.rows.is_empty()) {
            ui.ctx().request_repaint_after(AGE_REPAINT);
        }
        link
    }

    /// Only the first call after a paint has work.
    fn rest(&mut self) {
        self.table.rest();
        self.follower.rest();
        (self.detail, self.wanted, self.missing) = (None, None, None);
    }

    fn open(&mut self, link: Link) {
        self.wanted = Some(link);
        self.missing = None;
    }
}
