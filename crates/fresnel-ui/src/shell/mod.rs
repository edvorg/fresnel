//! The window layout: top bar, the dock with its tabs, and the status bar.
//!
//! [`Shell`] draws one frame from a copy of the state of the core and gives back what the
//! user asked for. It holds no handle to the core, so a view can start no network work.

mod dock;
mod status_bar;
mod top_bar;

use egui::{CentralPanel, Frame, Margin, Panel, Ui};
use fresnel_core::app::{Command, CoreView};

pub(crate) use dock::Tab;

use crate::format::Clock;
use crate::theme::{self, ClusterColors};
use crate::views::connections;
use crate::views::journal::JournalTab;

/// The state of the window that is not state of the core: the open tab and the filters.
#[derive(Debug)]
pub struct Shell {
    clock: Clock,
    tab: Tab,
    journal: JournalTab,
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
    pub fn new(ctx: &egui::Context, clock: Clock) -> Self {
        theme::install(ctx);
        Self { clock, tab: Tab::Connections, journal: JournalTab::default(), colors: ClusterColors::default() }
    }

    /// Draws one frame into the root `Ui` and returns the commands of the user.
    pub fn show(&mut self, ui: &mut Ui, view: &CoreView) -> Vec<Command> {
        self.colors.assign(view.clusters.iter().map(|row| &row.id));
        let mut commands = Vec::new();
        let mut scene = Scene { view, clock: &self.clock, colors: &self.colors, commands: &mut commands };

        let chrome = |ui: &Ui| Frame::new().fill(theme::palette(ui).bg_chrome).inner_margin(Margin::symmetric(8, 0));
        Panel::top("top_bar")
            .exact_size(top_bar::HEIGHT)
            .frame(chrome(ui))
            .show(ui, |ui| top_bar::show(ui, &mut scene));
        Panel::bottom("status_bar")
            .exact_size(status_bar::HEIGHT)
            .frame(chrome(ui))
            .show(ui, |ui| status_bar::show(ui, &scene, &mut self.tab));
        CentralPanel::no_frame().show(ui, |ui| {
            dock::tab_bar(ui, &mut self.tab);
            ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, theme::palette(ui).bg_sunken);
            match self.tab {
                Tab::Connections => connections::show(ui, &mut scene),
                Tab::Journal => self.journal.show(ui, &view.journal, &self.clock),
            }
        });
        commands
    }
}
