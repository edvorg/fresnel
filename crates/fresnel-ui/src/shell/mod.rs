//! The window layout (plan section 11.1): the top bar, the sidebar, the center with the
//! page of the sidebar, the dock with its tabs, and the status bar.
//!
//! [`Shell`] draws one frame from a copy of the state of the core and gives back what the
//! user asked for. It holds no handle to the core, so a view can start no network work.
//! It holds the page of the open kind only, and the overviews: a page that is not painted
//! holds no rows and no counts.
//!
//! The shell runs the commands of the command table (plan section 11.5): their keys, the
//! quick search, the history of the visited objects for Back and Forward, and the dialog of
//! a new Secret. It names the Secret or the ConfigMap of the open panel to the core, which
//! reads its values while the panel shows it.
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
use fresnel_core::keys::Target;
use fresnel_core::kinds::Kind;
use fresnel_core::logs::LogRequest;
use fresnel_core::net::task::Tasks;

pub(crate) use dock::Tab;
use history::History;
use sidebar::Page;

use crate::format::Clock;
use crate::theme::{self, ClusterColors};
use crate::views::detail::sections;
use crate::views::journal::JournalTab;
use crate::views::keys::new_secret::NewSecretDialog;
use crate::views::logs::LogTabs;
use crate::views::objects::{self, Link};
use crate::views::operations::OperationsTab;
use crate::views::overview::{Jump, OverviewPage};
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
    operations: OperationsTab,
    logs: LogTabs,
    /// The page of the kind that the sidebar opened last. The other pages keep it.
    open: Box<dyn objects::Page>,
    overview: OverviewPage,
    /// The kinds whose full objects the core holds for the views, as far as the shell knows.
    shown: BTreeSet<Kind>,
    /// The namespaces whose objects each view shows. Empty: each namespace.
    namespaces: BTreeSet<Arc<str>>,
    search: QuickSearch,
    new_secret: NewSecretDialog,
    /// The object whose values the core reads for the key view, as far as the shell knows.
    keys: Option<Target>,
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
    /// A command of the command table that a button of a view asks for. The shell runs it
    /// after the frame of the view.
    pub(crate) asked: Option<CommandId>,
    /// A log view that a view asks for. The shell opens its tab after the frame.
    pub(crate) logs: Option<LogRequest>,
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
            operations: OperationsTab::default(),
            logs: LogTabs::new(tasks, Arc::clone(&wake)),
            open: kinds::page(Kind::Pod, tasks, Arc::clone(&wake)),
            overview: OverviewPage::new(tasks, Arc::clone(&wake)),
            shown: BTreeSet::new(),
            namespaces: BTreeSet::new(),
            search: QuickSearch::new(tasks, Arc::clone(&wake)),
            new_secret: NewSecretDialog::default(),
            keys: None,
            history: History::default(),
            colors: ClusterColors::default(),
            tasks: tasks.clone(),
            wake,
        }
    }

    /// Draws one frame into the root `Ui` and returns the commands of the user.
    pub fn show(&mut self, ui: &mut Ui, view: &CoreView) -> Vec<Command> {
        self.colors.assign(view.clusters.iter().map(|row| &row.id));
        // The keys act before the widgets of the frame, which would take them. Back and
        // Forward take one step for each press; each other command acts one time.
        // A dialog takes the keys of the user.
        if !self.search.is_open()
            && !self.new_secret.is_open()
            && !self.open.has_dialog()
            && let Some((command, presses)) = keys::pressed(ui.ctx(), &view.config.keys)
        {
            let times = if matches!(command, CommandId::Back | CommandId::Forward) { presses } else { 1 };
            for _ in 0..times {
                self.run(command, ui.ctx());
            }
        }
        let mut commands = Vec::new();
        let on_page = self.open.namespaces().to_vec();
        let mut scene = Scene {
            view,
            clock: &self.clock,
            colors: &self.colors,
            namespaces: &mut self.namespaces,
            commands: &mut commands,
            asked: None,
            logs: None,
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
        let titles = self.logs.titles(view);
        // A dock with a log view has a height of its own: a log needs more rows.
        let (name, height) = match self.tab {
            Tab::Logs(_) => ("dock with logs", dock::LOG_HEIGHT),
            _ => ("dock", dock::HEIGHT),
        };
        let dock = Panel::bottom(name).resizable(true).default_size(height).min_size(dock::SMALLEST);
        dock.frame(Frame::new().fill(palette.bg_sunken)).show(ui, |ui| {
            if let Some(closed) = dock::tab_bar(ui, &mut self.tab, &titles, scene.colors) {
                self.logs.close(closed);
            }
            // The center can show the same view, so the two have widgets with different identities.
            ui.push_id("dock", |ui| match self.tab {
                Tab::Operations => self.operations.show(ui, &mut scene),
                Tab::Journal => self.journal.show(ui, &view.journal, &self.clock),
                Tab::Connections => connections::show(ui, &mut scene),
                Tab::Logs(id) => self.logs.show(id, ui, &mut scene),
            });
        });
        if let Tab::Logs(id) = self.tab
            && !self.logs.has(id)
        {
            self.tab = Tab::Journal;
        }
        let jump =
            CentralPanel::default().frame(Frame::new().fill(palette.bg_surface)).show(ui, |ui| match self.page {
                Page::Kind(_) => {
                    self.overview.rest();
                    self.open.show(ui, &mut scene).map(Jump::Object)
                }
                Page::Overview(overview) => {
                    self.open.rest();
                    self.overview.show(ui, &scene, overview)
                }
                Page::Connections => {
                    self.open.rest();
                    self.overview.rest();
                    connections::show(ui, &mut scene);
                    None
                }
            });
        let chosen = self.search.show(ui.ctx(), &scene, &view.config.keys);
        let created = self.new_secret.show(ui.ctx(), &mut scene, &on_page);
        let asked = scene.asked.take();
        if let Some(request) = scene.logs.take() {
            self.tab = Tab::Logs(self.logs.open(request));
            ui.ctx().request_repaint();
        }
        if let Some(visited) = self.open.opened() {
            self.history.visit(visited);
        }
        if let Some(jump) = jump.inner {
            match jump {
                Jump::Object(link) => self.follow_link(link),
                Jump::Table { kind, problems } => {
                    self.page = Page::Kind(kind);
                    follow_page(&mut self.open, self.page, &self.tasks, &self.wake);
                    self.open.show_problems(problems);
                }
            }
            ui.ctx().request_repaint();
        }
        match chosen {
            Some(Choice::Open(link)) => self.follow_link(link),
            Some(Choice::Run(command)) => self.run(command, ui.ctx()),
            None => {}
        }
        if let Some(command) = asked {
            self.run(command, ui.ctx());
        }
        if let Some(link) = created {
            self.follow_link(link);
            ui.ctx().request_repaint();
        }
        commands.extend(self.show_kinds());
        commands.extend(self.show_keys());
        commands.extend(self.logs.command());
        // A job that the user started shows its steps in the Operations tab.
        if commands.iter().any(|command| matches!(command, Command::Run(_))) {
            self.tab = Tab::Operations;
        }
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
            CommandId::NewSecret => self.new_secret.open(),
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

    /// The command that names the kinds of the open page to the core, when they changed.
    /// The core holds the store of their full objects: the kind of a table and the kinds
    /// that its panel reads, or the kinds that an overview counts; the Pods need none. The
    /// Connections page keeps the stores of the page before it, which shows again at once.
    fn show_kinds(&mut self) -> Option<Command> {
        let wanted = match self.page {
            Page::Kind(Kind::Pod) => BTreeSet::new(),
            Page::Kind(kind) => std::iter::once(kind).chain(sections::also_shown(kind).iter().copied()).collect(),
            Page::Overview(overview) => overview.kinds().iter().copied().collect(),
            Page::Connections => return None,
        };
        if wanted == self.shown {
            return None;
        }
        self.shown.clone_from(&wanted);
        Some(Command::ShowKinds(wanted))
    }

    /// The command that names the Secret or the ConfigMap of the open panel to the core, when
    /// it changed. A closed panel names none, and the Connections page has no panel, so the
    /// core drops the values (plan section 9.4).
    fn show_keys(&mut self) -> Option<Command> {
        let wanted = self.open.keys().cloned();
        if wanted == self.keys {
            return None;
        }
        self.keys.clone_from(&wanted);
        Some(Command::ShowKeys(wanted))
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
