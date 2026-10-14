//! The window layout (plan section 11.1): the top bar, the sidebar, the center with the
//! page of the sidebar, the dock with its tabs, and the status bar.
//!
//! [`Shell`] draws one frame from a copy of the state of the core and gives back what the
//! user asked for. It holds no handle to the core, so a view can start no network work.
//! It holds the page of the open kind only: a page that is not painted holds no rows.
//!
//! The shell runs the commands of the command table (plan section 11.5): their keys, the
//! quick search and the history of the visited objects for Back and Forward.
//!
//! - `top_bar`, `sidebar`, `dock`, `status_bar`: the parts of the window. `namespaces`:
//!   the namespace filter of each view. `keys`: the key of each command. `history`: Back
//!   and Forward.

mod dock;
mod history;
pub(crate) mod keys;
mod namespaces;
mod sidebar;
mod status_bar;
mod top_bar;

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{CentralPanel, Frame, Margin, Panel, Ui};
use fresnel_core::app::{Command, CoreView};
use fresnel_core::commands::CommandId;
use fresnel_core::kinds::Kind;
use fresnel_core::net::task::Tasks;

pub(crate) use dock::Tab;
use history::History;
use sidebar::Page;

use crate::format::Clock;
use crate::theme::{self, ClusterColors};
use crate::views::journal::JournalTab;
use crate::views::objects::{self, Link};
use crate::views::search::{Choice, QuickSearch};
use crate::views::{connections, kinds};
use crate::widgets::worker::Wake;

/// The state of the window that is not state of the core: the open page, the open tab,
/// the filters, the selection and the visited objects.
pub struct Shell {
    clock: Clock,
    page: Page,
    tab: Tab,
    journal: JournalTab,
    /// The page of the kind that the sidebar opened last. The Connections page keeps it.
    open: Box<dyn objects::Page>,
    /// The kinds whose full objects the core holds for the views, as far as the shell knows.
    shown: BTreeSet<Kind>,
    /// The namespaces whose objects each view shows. Empty: each namespace.
    namespaces: BTreeSet<Arc<str>>,
    search: QuickSearch,
    history: History,
    colors: ClusterColors,
    tasks: Tasks,
    wake: Wake,
}

/// What one frame can read and change. Each view gets the parts that it needs.
pub(crate) struct Scene<'a> {
    pub(crate) view: &'a CoreView,
    pub(crate) clock: &'a Clock,
    pub(crate) colors: &'a ClusterColors,
    /// The namespace filter of each view.
    pub(crate) namespaces: &'a mut BTreeSet<Arc<str>>,
    /// The commands of this frame. The caller gives them to the core.
    pub(crate) commands: &'a mut Vec<Command>,
}

impl Shell {
    /// Installs the theme of the app in `ctx`. `clock` gives the time and the time zone.
    /// `tasks` starts the worker threads of the tables and of the quick search: the journal
    /// of the app then has the report of a worker that ends, and the worker starts again.
    pub fn new(ctx: &egui::Context, clock: Clock, tasks: &Tasks) -> Self {
        theme::install(ctx);
        let wake: Wake = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        Self {
            clock,
            page: Page::Kind(Kind::Pod),
            tab: Tab::Journal,
            journal: JournalTab::default(),
            open: kinds::page(Kind::Pod, tasks, Arc::clone(&wake)),
            shown: BTreeSet::new(),
            namespaces: BTreeSet::new(),
            search: QuickSearch::new(tasks, Arc::clone(&wake)),
            history: History::default(),
            colors: ClusterColors::default(),
            tasks: tasks.clone(),
            wake,
        }
    }

    /// Draws one frame into the root `Ui` and returns the commands of the user.
    pub fn show(&mut self, ui: &mut Ui, view: &CoreView) -> Vec<Command> {
        self.colors.assign(view.clusters.iter().map(|row| &row.id));
        // The keys act before the widgets of the frame, which would take them.
        if !self.search.is_open()
            && let Some(command) = keys::pressed(ui.ctx(), &view.config.keys)
        {
            self.run(command, ui.ctx());
        }
        let mut commands = Vec::new();
        let on_page = self.open.namespaces().to_vec();
        let mut scene = Scene {
            view,
            clock: &self.clock,
            colors: &self.colors,
            namespaces: &mut self.namespaces,
            commands: &mut commands,
        };
        let palette = theme::palette(ui);

        let chrome = Frame::new().fill(palette.bg_chrome);
        let bar = chrome.inner_margin(Margin::symmetric(8, 0));
        let top = Panel::top("top_bar").exact_size(top_bar::HEIGHT).frame(bar);
        if top.show(ui, |ui| top_bar::show(ui, &mut scene, &on_page)).inner {
            self.search.open();
        }
        Panel::bottom("status_bar")
            .exact_size(status_bar::HEIGHT)
            .frame(bar)
            .show(ui, |ui| status_bar::show(ui, &scene, &mut self.tab));
        Panel::left("sidebar")
            .exact_size(sidebar::WIDTH)
            .frame(chrome.inner_margin(Margin::symmetric(6, 0)))
            .show(ui, |ui| sidebar::show(ui, view, &mut self.page));
        follow_page(&mut self.open, self.page, &self.tasks, &self.wake);
        let dock = Panel::bottom("dock").resizable(true).default_size(dock::HEIGHT).min_size(dock::SMALLEST);
        dock.frame(Frame::new().fill(palette.bg_sunken)).show(ui, |ui| {
            dock::tab_bar(ui, &mut self.tab);
            // The center can show the same view, so the two have widgets with different identities.
            ui.push_id("dock", |ui| match self.tab {
                Tab::Journal => self.journal.show(ui, &view.journal, &self.clock),
                Tab::Connections => connections::show(ui, &mut scene),
            });
        });
        let link =
            CentralPanel::default().frame(Frame::new().fill(palette.bg_surface)).show(ui, |ui| match self.page {
                Page::Kind(_) => self.open.show(ui, &mut scene),
                Page::Connections => {
                    self.open.rest();
                    connections::show(ui, &mut scene);
                    None
                }
            });
        let chosen = self.search.show(ui.ctx(), &scene, &view.config.keys);
        if let Some(visited) = self.open.opened() {
            self.history.visit(visited);
        }
        if let Some(link) = link.inner {
            self.follow_link(link);
            ui.ctx().request_repaint();
        }
        match chosen {
            Some(Choice::Open(link)) => self.follow_link(link),
            Some(Choice::Run(command)) => self.run(command, ui.ctx()),
            None => {}
        }
        commands.extend(self.show_kinds());
        commands
    }

    /// Runs a command of the command table.
    fn run(&mut self, command: CommandId, ctx: &egui::Context) {
        match command {
            CommandId::QuickSearch => self.search.open(),
            CommandId::Back => {
                if let Some(link) = self.history.back() {
                    self.follow_link(link);
                }
            }
            CommandId::Forward => {
                if let Some(link) = self.history.forward() {
                    self.follow_link(link);
                }
            }
            CommandId::Filter => self.open.focus_filter(),
            CommandId::Object(action) => self.open.act(ctx, action),
        }
        ctx.request_repaint();
    }

    /// Opens the page of the kind of a link, and the panel of its object there.
    fn follow_link(&mut self, link: Link) {
        self.page = Page::Kind(link.kind);
        follow_page(&mut self.open, self.page, &self.tasks, &self.wake);
        self.open.open(link);
    }

    /// The command that names the kind of the open page to the core, when it changed. The
    /// core holds the store of its full objects; the Pods need none. The Connections page
    /// keeps the store of the page before it, which shows again at once.
    fn show_kinds(&mut self) -> Option<Command> {
        let wanted = match self.page {
            Page::Kind(Kind::Pod) => BTreeSet::new(),
            Page::Kind(kind) => BTreeSet::from([kind]),
            Page::Connections => return None,
        };
        if wanted == self.shown {
            return None;
        }
        self.shown.clone_from(&wanted);
        Some(Command::ShowKinds(wanted))
    }
}

/// Makes `open` the page of the kind of `page`. A page of an other kind gives up its rows,
/// and its worker thread ends.
fn follow_page(open: &mut Box<dyn objects::Page>, page: Page, tasks: &Tasks, wake: &Wake) {
    let Page::Kind(kind) = page else {
        return;
    };
    if open.kind() != kind {
        let mut old = std::mem::replace(open, kinds::page(kind, tasks, Arc::clone(wake)));
        // The worker thread frees the rows of the old page before it ends.
        old.rest();
    }
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell").field("page", &self.page).field("tab", &self.tab).finish_non_exhaustive()
    }
}
