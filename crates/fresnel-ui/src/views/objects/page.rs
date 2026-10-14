//! The page of one kind: the bar, the notes, the table and the detail panel.
//!
//! The page opens the object of a link (plan section 11.5): it selects its row and shows
//! its panel. When a filter hides that row, a note says so and offers to show it.

use std::sync::Arc;
use std::time::Duration;

use egui::{CentralPanel, Context, Frame, Panel, RichText, Ui};
use fresnel_core::app::ClusterRow;
use fresnel_core::keys::Target;
use fresnel_core::kinds::{Action, Describe, Kind, Object};
use fresnel_core::net::task::Tasks;

use super::filter::ObjectFilter;
use super::follow::Follower;
use super::notes::{self, Facts, old_since};
use super::{ClusterMark, Link, Nouns, ObjectModel, Page, RowScene, menu, toolbar, uid};
use crate::shell::Scene;
use crate::theme::{self, Palette};
use crate::views::detail::{self, DetailPanel};
use crate::widgets::table::model::ObjectId;
use crate::widgets::table::{Act, Table};
use crate::widgets::tone::Tone;
use crate::widgets::worker::Wake;

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
    /// The object of a link, until the store of its cluster has a complete list. A note
    /// tells the wait, and a row or a panel that the user chooses ends it, so a list that
    /// arrives later does not take the panel from the choice of the user.
    wanted: Option<Link>,
    /// What the last link or action did not do, and why; or what it copied.
    note: Option<(Tone, String)>,
    /// The object of the panel after a change, until the shell takes it.
    opened: Option<Link>,
    /// True when the text filter takes the focus in the next frame.
    focus: bool,
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
            note: None,
            opened: None,
            focus: false,
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
        let waits = format!("The {name} of {} opens when its store has a complete list.", link.cluster);
        let Some(snapshot) = self.follower.snapshot(&link.cluster) else {
            if scene.view.clusters.iter().any(|row| row.id == link.cluster && row.selected) {
                self.note = Some((Tone::Info, waits));
            } else {
                let text = format!("The {name} is of {}, which is not selected.", link.cluster);
                (self.note, self.wanted) = (Some((Tone::Warn, text)), None);
            }
            return;
        };
        match snapshot.items.get(&link.key) {
            Some(object) => {
                let (cluster, key) = (link.cluster.clone(), link.key.clone());
                let id = ObjectId { cluster, key, uid: uid(object.meta()).into() };
                let object = Arc::clone(object);
                self.show_object(id, object);
            }
            None if snapshot.listed => {
                (self.note, self.wanted) = (Some((Tone::Warn, format!("{} has no {name}.", link.cluster))), None);
            }
            None => self.note = Some((Tone::Info, waits)),
        }
    }

    /// Selects an object, scrolls to its row and shows its panel. That ends the wait for
    /// the object of a link.
    fn show_object(&mut self, id: ObjectId, object: Arc<M::Object>) {
        self.table.select(id.clone());
        self.opened = Some(link_of::<M::Object>(&id));
        self.detail = Some(DetailPanel::new(id, object));
        (self.note, self.wanted) = (None, None);
    }

    /// The object of the menu of a row: it becomes the object of the panel, and the action
    /// acts on it. An object that went away since the frame of the menu gets a note.
    fn act_on_row(&mut self, ctx: &Context, id: ObjectId, action: Action) {
        let store = self.follower.snapshot(&id.cluster);
        let object = store.and_then(|store| store.items.get(&id.key)).filter(|object| uid(object.meta()) == &*id.uid);
        match object.map(Arc::clone) {
            Some(object) => {
                self.show_object(id, object);
                self.act(ctx, action);
            }
            None => {
                let text =
                    format!("The {} {} went away, so {} did not run.", self.nouns.one, id.key.name, action.label());
                self.note = Some((Tone::Warn, text));
            }
        }
    }

    /// The note of the last link or action.
    fn note_line(&self, ui: &mut Ui, palette: &Palette) {
        if let Some((tone, text)) = &self.note {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new(text).color(tone.color(palette)));
            });
        }
    }

    /// A note when the filters hide the row of the object of the panel, with an action
    /// that changes them so the row shows.
    fn hidden_line(&mut self, ui: &mut Ui, palette: &Palette, scene: &mut Scene<'_>) {
        let Some(panel) = &self.detail else {
            return;
        };
        if !panel.is_present() || !self.table.hides(panel.id()) {
            return;
        }
        let id = panel.id().clone();
        let mut show = false;
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let text = format!("The filters hide the row of this {}.", self.nouns.one);
            ui.label(RichText::new(text).color(palette.warn));
            show = ui.small_button("Show it").clicked();
        });
        if show {
            (self.filter.text, self.filter.problems) = (String::new(), false);
            if !scene.namespaces.is_empty() && !id.key.namespace.is_empty() {
                scene.namespaces.insert(Arc::clone(&id.key.namespace));
            }
            self.table.select(id);
            ui.ctx().request_repaint();
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
                    (self.detail, self.wanted) = (None, None);
                    self.table.clear_selection();
                }
                detail::Act::Open(target) => link = Some(target),
                detail::Act::None => {}
            }
        }
        if let Some(command) =
            toolbar::show(ui, palette, &self.nouns, &mut self.filter, &mut self.table, &mut self.focus)
        {
            scene.asked = Some(command);
        }
        // The namespace filter of the top bar is one filter for each page.
        if self.filter.namespaces != *scene.namespaces {
            self.filter.namespaces.clone_from(scene.namespaces);
        }
        // After the bar: a filter that the user changed in this frame asks for its rows now.
        self.table.request(&self.filter, self.follower.serial(), || self.follower.data());
        notes::show(ui, palette, scene, &self.nouns, |cluster| self.follower.snapshot(cluster).map(Facts::of));
        self.note_line(ui, palette);
        self.hidden_line(ui, palette, scene);
        let cells = self.row_scene(scene, palette);
        let (kind, keys) = (<M::Object as Describe>::ID, &scene.view.config.keys);
        let mut chosen = None;
        let mut menu = |ui: &mut Ui, id: &ObjectId| {
            if let Some(action) = menu::show(ui, kind, keys) {
                chosen = Some((id.clone(), action));
            }
        };
        let (filter, nouns, follower) = (&self.filter, &self.nouns, &self.follower);
        let empty = || empty_text(scene, filter, nouns, follower);
        let acted = CentralPanel::no_frame().show(ui, |ui| self.table.show(ui, &cells, empty, &mut menu));
        scene.commands.extend(self.table.save_layout(ui));
        match acted.inner {
            Some(Act::Cursor) => {
                let model = self.table.model();
                let row = self.table.cursor_row();
                self.detail = row.map(|row| DetailPanel::new(model.id(row).clone(), Arc::clone(model.object(row))));
                self.opened = self.detail.as_ref().map(|panel| link_of::<M::Object>(panel.id()));
                (self.note, self.wanted) = (None, None);
                // The panel is before the table in a frame, so the next frame shows it.
                ui.ctx().request_repaint();
            }
            Some(Act::Cleared) => (self.detail, self.wanted) = (None, None),
            None => {}
        }
        if let Some((id, action)) = chosen {
            self.act_on_row(ui.ctx(), id, action);
            ui.ctx().request_repaint();
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
        (self.detail, self.wanted, self.note, self.opened, self.focus) = (None, None, None, None, false);
    }

    fn open(&mut self, link: Link) {
        self.wanted = Some(link);
        self.note = None;
    }

    fn opened(&mut self) -> Option<Link> {
        self.opened.take()
    }

    fn act(&mut self, ctx: &Context, action: Action) {
        let Some(panel) = &mut self.detail else {
            let text = format!("No {} is open in the panel. {} acts on that object.", self.nouns.one, action.label());
            self.note = Some((Tone::Warn, text));
            return;
        };
        self.note = match panel.act(ctx, action) {
            Ok(done) => done.map(|done| (Tone::Info, done)),
            Err(reason) => Some((Tone::Warn, reason)),
        };
    }

    fn focus_filter(&mut self) {
        self.focus = true;
    }

    fn namespaces(&self) -> &[Arc<str>] {
        self.table.rows().map(|rows| rows.summary.names.as_slice()).unwrap_or_default()
    }

    fn keys(&self) -> Option<&Target> {
        self.detail.as_ref().and_then(DetailPanel::key_target)
    }
}

/// The link of an object of the kind `K`.
fn link_of<K: Describe>(id: &ObjectId) -> Link {
    Link { cluster: id.cluster.clone(), kind: K::ID, key: id.key.clone() }
}
