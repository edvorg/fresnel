//! The window layout (plan section 11.1): the top bar, the sidebar, the center with the
//! page of the sidebar, the dock with its tabs, and the status bar.
//!
//! [`Shell`] draws one frame from a copy of the state of the core and gives back what the
//! user asked for. It holds no handle to the core, so a view can start no network work.

mod dock;
mod sidebar;
mod status_bar;
mod top_bar;

use std::sync::Arc;

use egui::{CentralPanel, Frame, Margin, Panel, Ui};
use fresnel_core::app::{Command, CoreView};
use fresnel_core::net::task::Tasks;

pub(crate) use dock::Tab;
use sidebar::Page;

use crate::format::Clock;
use crate::theme::{self, ClusterColors};
use crate::views::connections;
use crate::views::journal::JournalTab;
use crate::views::pods::PodsView;

/// The state of the window that is not state of the core: the open page, the open tab,
/// the filters and the selection.
pub struct Shell {
    clock: Clock,
    page: Page,
    tab: Tab,
    journal: JournalTab,
    pods: PodsView,
    colors: ClusterColors,
}

/// What one frame can read and change. Each view gets the parts that it needs.
pub(crate) struct Scene<'a> {
    pub(crate) view: &'a CoreView,
    pub(crate) clock: &'a Clock,
    pub(crate) colors: &'a ClusterColors,
    /// The commands of this frame. The caller gives them to the core.
    pub(crate) commands: &'a mut Vec<Command>,
}

impl Shell {
    /// Installs the theme of the app in `ctx`. `clock` gives the time and the time zone.
    /// `tasks` starts the worker thread of each table: the journal of the app then has
    /// the report of a worker that ends, and the worker starts again.
    pub fn new(ctx: &egui::Context, clock: Clock, tasks: &Tasks) -> Self {
        theme::install(ctx);
        let wake = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        Self {
            clock,
            page: Page::Pods,
            tab: Tab::Journal,
            journal: JournalTab::default(),
            pods: PodsView::new(tasks, wake),
            colors: ClusterColors::default(),
        }
    }

    /// Draws one frame into the root `Ui` and returns the commands of the user.
    pub fn show(&mut self, ui: &mut Ui, view: &CoreView) -> Vec<Command> {
        self.colors.assign(view.clusters.iter().map(|row| &row.id));
        let mut commands = Vec::new();
        let mut scene = Scene { view, clock: &self.clock, colors: &self.colors, commands: &mut commands };
        let palette = theme::palette(ui);

        let chrome = Frame::new().fill(palette.bg_chrome);
        let bar = chrome.inner_margin(Margin::symmetric(8, 0));
        Panel::top("top_bar").exact_size(top_bar::HEIGHT).frame(bar).show(ui, |ui| top_bar::show(ui, &mut scene));
        Panel::bottom("status_bar")
            .exact_size(status_bar::HEIGHT)
            .frame(bar)
            .show(ui, |ui| status_bar::show(ui, &scene, &mut self.tab));
        Panel::left("sidebar")
            .exact_size(sidebar::WIDTH)
            .frame(chrome.inner_margin(Margin::symmetric(6, 0)))
            .show(ui, |ui| sidebar::show(ui, &mut self.page));
        let dock = Panel::bottom("dock").resizable(true).default_size(dock::HEIGHT).min_size(dock::SMALLEST);
        dock.frame(Frame::new().fill(palette.bg_sunken)).show(ui, |ui| {
            dock::tab_bar(ui, &mut self.tab);
            // The center can show the same view, so the two have widgets with different identities.
            ui.push_id("dock", |ui| match self.tab {
                Tab::Journal => self.journal.show(ui, &view.journal, &self.clock),
                Tab::Connections => connections::show(ui, &mut scene),
            });
        });
        CentralPanel::default().frame(Frame::new().fill(palette.bg_surface)).show(ui, |ui| match self.page {
            Page::Pods => self.pods.show(ui, &scene),
            Page::Connections => {
                self.pods.rest();
                connections::show(ui, &mut scene);
            }
        });
        commands
    }
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell").field("page", &self.page).field("tab", &self.tab).finish_non_exhaustive()
    }
}
