//! One tab of logs: the view of one request, its options, its search and its pane.
//!
//! The tab finds its view in the data of its cluster by its request. After a restart of the
//! task of the view in the core, the view numbers its lines again; the tab then starts its
//! search, its selection and its place again, as for new options.

use std::sync::Arc;

use egui::{Frame, Margin, RichText, Ui};
use fresnel_core::app::{ClusterData, Command, CoreView};
use fresnel_core::cluster::ClusterId;
use fresnel_core::logs::{LogRequest, LogSave, LogView};
use fresnel_core::net::task::Tasks;

use super::bar::{self, Asked};
use super::copy::{Copier, Job};
use super::find::Find;
use super::full;
use super::pane::select::Columns;
use super::pane::{Input, Pane};
use super::{sources, title_text};
use crate::shell::{Scene, Tab, Title};
use crate::theme::{self, PAD};
use crate::widgets::tone::Tone;
use crate::widgets::worker::Wake;

/// The state of one tab.
pub(super) struct LogTab {
    pub(super) id: u64,
    pub(super) request: LogRequest,
    pane: Pane,
    find: Find,
    copy: Copier,
    wrap: bool,
    times: bool,
    /// The count of restarts of the view that the tab saw last.
    restarts: Option<u32>,
    /// The line of the window of a long line.
    full: Option<u64>,
    /// The note of the last copy.
    note: Option<(Tone, String)>,
}

/// The worker threads of the views and their wake.
pub(super) struct Threads<'a> {
    pub(super) tasks: &'a Tasks,
    pub(super) wake: &'a Wake,
}

impl LogTab {
    pub(super) fn new(id: u64, request: LogRequest) -> Self {
        Self {
            id,
            request,
            pane: Pane::default(),
            find: Find::default(),
            copy: Copier::default(),
            wrap: false,
            times: true,
            restarts: None,
            full: None,
            note: None,
        }
    }

    pub(super) fn title(&self, view: &CoreView) -> Title {
        let found = data(view, &self.request.cluster).and_then(|data| log_view(data, &self.request));
        let tone = found.map_or(Tone::Faint, |view| sources::tone(&view.sources));
        Title { tab: Tab::Logs(self.id), text: title_text(&self.request), cluster: self.request.cluster.clone(), tone }
    }

    pub(super) fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>, threads: &Threads<'_>) {
        let data = data(scene.view, &self.request.cluster);
        let view = data.and_then(|data| log_view(data, &self.request));
        if let Some(view) = view {
            if self.restarts.replace(view.restarts).is_some_and(|before| before != view.restarts) {
                self.start_again();
            }
            self.find.update(&view.lines, threads.tasks, threads.wake);
        }
        if let Some(note) = self.copy.poll(ui.ctx()) {
            self.note = Some(note);
        }
        let palette = theme::palette(ui);
        let save = data.and_then(|data| data.saves.iter().rev().find(|save| save.save.request == self.request));
        let mut asked = Vec::new();
        let margin = Margin { left: PAD as i8, right: PAD as i8, top: 2, bottom: 2 };
        Frame::new().inner_margin(margin).show(ui, |ui| {
            let parts = bar::Parts { data, view, save: save.map(|save| &**save) };
            let selected = self.pane.selection.is_some_and(|selection| !selection.is_empty());
            let mut state = bar::State {
                wrap: &mut self.wrap,
                times: &mut self.times,
                follow: &mut self.pane.follow,
                find: &mut self.find,
                selected,
            };
            asked = bar::show(ui, &mut state, &self.request, &parts);
            if let Some(view) = view {
                sources::show(ui, view, scene.clock);
            }
            for (tone, text) in [self.copy.state(), self.note.clone()].into_iter().flatten() {
                ui.label(RichText::new(text).color(tone.color(palette)));
            }
        });
        let copy = self.act(asked, scene);
        let Some(view) = view else {
            Frame::new().inner_margin(Margin::same(PAD as i8)).show(ui, |ui| {
                ui.label(RichText::new(sources::absent(scene.view, &self.request)).color(palette.text_muted));
            });
            // The dock keeps the height of its content: the tab takes the rest of it.
            ui.allocate_space(ui.available_size());
            return;
        };
        let input = Input {
            view,
            kept: self.find.kept(),
            wrap: self.wrap,
            times: self.times,
            search: self.find.search(),
            current: self.find.current(),
            clock: scene.clock,
        };
        let output = self.pane.show(ui, &input);
        self.full = output.full_line.or(self.full);
        if copy || output.copy {
            self.copy_selection(ui.ctx(), view, threads);
        }
        full::show(ui.ctx(), self.id, &mut self.full, view);
    }

    /// Runs what the user asked for in the bar. Gives true for a copy.
    fn act(&mut self, asked: Vec<Asked>, scene: &mut Scene<'_>) -> bool {
        let mut copy = false;
        for ask in asked {
            match ask {
                Asked::Options(options) => {
                    self.request.options = options;
                    self.start_again();
                }
                Asked::Go(forward) => {
                    if let Some(line) = self.find.go(forward, self.pane.top_line()) {
                        self.pane.reveal(line);
                    }
                }
                Asked::Copy => copy = true,
                Asked::Save(part) => {
                    let save = LogSave { request: self.request.clone(), part, times: self.times };
                    scene.commands.push(Command::SaveLog(save));
                }
                Asked::Cancel(number) => {
                    scene.commands.push(Command::CancelSave { cluster: self.request.cluster.clone(), number });
                }
            }
        }
        copy
    }

    /// Copies the selection with the columns that the pane shows.
    fn copy_selection(&mut self, ctx: &egui::Context, view: &LogView, threads: &Threads<'_>) {
        let Some(selection) = self.pane.selection.filter(|selection| !selection.is_empty()) else {
            return;
        };
        let sources = (view.sources.len() > 1).then(|| Arc::clone(&view.sources));
        let columns = Columns { times: self.times, sources };
        let job = Job { lines: view.lines.clone(), kept: self.find.kept_shared(), selection, columns };
        self.note = self.copy.start(ctx, job, threads.tasks, threads.wake);
    }

    /// A new view: no line of the earlier one is in it.
    fn start_again(&mut self) {
        self.find.restart();
        (self.pane, self.full, self.note) = (Pane::default(), None, None);
    }
}

/// The data of a connected cluster.
fn data<'a>(view: &'a CoreView, cluster: &ClusterId) -> Option<&'a ClusterData> {
    view.clusters.iter().find(|row| &row.id == cluster).and_then(|row| row.data.as_ref())
}

/// The view of a request in the data of its cluster.
fn log_view<'a>(data: &'a ClusterData, request: &LogRequest) -> Option<&'a LogView> {
    data.logs.iter().find(|view| view.request == *request).map(|view| &**view)
}
