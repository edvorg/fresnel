//! The terminals of the dock (plan section 12.8): a tab for each terminal of the core, with
//! the line of the state of its session, the widget, and a question before a long paste.
//!
//! The core holds the terminals, and the tabs follow its list: a tab keeps the state of its
//! widget while its terminal is in the list. A close asks the core to close the terminal.
//!
//! - `line`: the line of the state.

mod line;

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Button, Context, Frame, Margin, RichText, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::config::Config;
use fresnel_core::config::table::Setting;
use fresnel_core::kinds::Action;
use fresnel_core::ops::Target;
use fresnel_core::terminal::{PIECE, Program, Scroll, TerminalRequest};

use crate::format::{self, short};
use crate::shell::{Scene, Tab, Title};
use crate::theme::{self, PAD, SMALL};
use crate::widgets::terminal::bell::Bell;
use crate::widgets::terminal::{KeyOptions, Settings, TerminalWidget};
use line::Asked;

/// What the panel asks for: a program on a container of a pod, or a shell on a node. The
/// container is `None` for the default container of a pod, and empty for a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalAsk {
    pub(crate) program: Program,
    pub(crate) container: Option<Arc<str>>,
}

impl TerminalAsk {
    /// The ask of an action that the command table names, with no container yet.
    pub(crate) fn of(action: Action) -> Option<Self> {
        let program = match action {
            Action::Terminal => Program::Shell,
            Action::Attach => Program::Attach,
            Action::DebugContainer => Program::Debug,
            Action::NodeShell => Program::NodeShell,
            _ => return None,
        };
        Some(Self { program, container: None })
    }

    /// The action of the command table for this program.
    pub(crate) fn action(&self) -> Action {
        match self.program {
            Program::Shell => Action::Terminal,
            Program::Attach => Action::Attach,
            Program::Debug => Action::DebugContainer,
            Program::NodeShell => Action::NodeShell,
        }
    }

    /// The request of a terminal on the object of `target`: a pod, or a node for a node shell.
    pub(crate) fn request(&self, target: &Target) -> TerminalRequest {
        TerminalRequest {
            cluster: target.cluster.clone(),
            object: target.key.clone(),
            uid: Arc::clone(&target.uid),
            container: self.container.clone().unwrap_or_default(),
            program: self.program,
        }
    }
}

/// The state of the tab of one terminal.
#[derive(Debug)]
struct TerminalTab {
    id: u64,
    widget: TerminalWidget,
    /// A long paste that waits for a choice of the user.
    paste: Option<String>,
    /// The note of the last input that did not fit, until some input goes.
    note: Option<String>,
    /// True when the widget had the focus of the keys in its last frame.
    focused: bool,
}

/// The tabs of the terminals.
#[derive(Debug, Default)]
pub(crate) struct TerminalTabs {
    tabs: Vec<TerminalTab>,
    /// The newest terminal that the tabs know: a newer one is new in the core.
    newest: Option<u64>,
    /// The flash of each terminal, also of one that the dock does not show: its tab and its
    /// grid read it.
    bells: BTreeMap<u64, Bell>,
}

impl TerminalTabs {
    /// The title of each terminal of the core, in the sequence of the core. A tab flashes for
    /// a new bell of its terminal.
    pub(crate) fn titles(&mut self, view: &CoreView, ctx: &Context) -> Vec<Title> {
        self.bells.retain(|id, _| view.terminals.iter().any(|terminal| terminal.id == *id));
        let mut titles = Vec::new();
        for terminal in view.terminals.iter() {
            let bell = self.bells.entry(terminal.id).or_default().flashes(ctx, terminal.terminal.bells());
            let tone = line::tone(&terminal.terminal.state());
            let (tab, cluster) = (Tab::Terminal(terminal.id), terminal.request.cluster.clone());
            titles.push(Title { tab, text: title_text(&terminal.request), cluster, tone, bell });
        }
        titles
    }

    /// Keeps the tabs of the terminals that the core has, and gives a terminal that the core
    /// opened since the last frame, whose tab then opens. The first frame only notes the
    /// newest one: a tab that paints before the fonts of the terminal bind, in the next frame,
    /// would make the layout panic.
    pub(crate) fn follow(&mut self, view: &CoreView) -> Option<u64> {
        self.tabs.retain(|tab| view.terminals.iter().any(|terminal| terminal.id == tab.id));
        let newest = view.terminals.iter().map(|terminal| terminal.id).max().unwrap_or(0);
        let opened = self.newest.filter(|known| newest > *known).map(|_| newest);
        self.newest = Some(self.newest.map_or(newest, |known| known.max(newest)));
        opened
    }

    /// True when the terminal `id` had the focus of the keys in its last frame: it takes each
    /// key that the app would take, besides those with Cmd.
    pub(crate) fn focused(&self, id: u64) -> bool {
        self.tabs.iter().any(|tab| tab.id == id && tab.focused)
    }

    /// Draws the tab of the terminal `id`.
    pub(crate) fn show(&mut self, id: u64, ui: &mut Ui, scene: &mut Scene<'_>) {
        let Some(view) = scene.view.terminals.iter().find(|terminal| terminal.id == id) else {
            return;
        };
        if !self.tabs.iter().any(|tab| tab.id == id) {
            let widget = TerminalWidget::default();
            self.tabs.push(TerminalTab { id, widget, paste: None, note: None, focused: false });
        }
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return;
        };
        let palette = theme::palette(ui);
        let margin = Margin { left: PAD as i8, right: PAD as i8, top: 2, bottom: 2 };
        Frame::new().inner_margin(margin).show(ui, |ui| {
            let widget = line::Widget { size: tab.widget.size(), history: tab.widget.history() };
            match line::show(ui, &view.terminal, &widget, scene.clock) {
                Some(Asked::DropInput) => {
                    view.terminal.drop_input();
                }
                Some(Asked::ToEnd) => view.terminal.scroll(Scroll::Bottom),
                None => {}
            }
            if let Some(text) = &tab.paste
                && let Some(paste) = ask(ui, text.len())
            {
                let text = tab.paste.take().unwrap_or_default();
                if paste {
                    tab.note = view.terminal.paste(&text).err().map(|refused| refused.to_string());
                }
                // The keys go on to the terminal, not to the button.
                tab.widget.refocus();
            }
            if let Some(note) = &tab.note {
                ui.label(RichText::new(format!("The input did not go: {note}.")).size(SMALL).color(palette.warn));
            }
        });
        let ringing = self.bells.get(&id).is_some_and(|bell| bell.lit(ui.ctx()));
        // The grid has the side margin of the line of the state, as the text of each tab of the
        // dock: with none, its text touched the edge of the dock.
        let (sides, chosen) = (Margin { top: 0, bottom: 0, ..margin }, settings(&scene.view.config));
        let shown =
            Frame::new().inner_margin(sides).show(ui, |ui| tab.widget.show(ui, &view.terminal, chosen, ringing)).inner;
        tab.focused = shown.focused;
        if shown.paste.is_some() {
            tab.paste = shown.paste;
        }
        if shown.sent {
            tab.note = None;
        }
        if let Some(refused) = shown.refused {
            tab.note = Some(refused.to_string());
        }
    }
}

/// The question before a long paste: a paste goes as it is, also a command that the user
/// did not mean. Gives the choice of this frame: true to paste.
fn ask(ui: &mut Ui, bytes: usize) -> Option<bool> {
    let palette = theme::palette(ui);
    let mut choice = None;
    ui.horizontal_wrapped(|ui| {
        let size = format::count(bytes as u64, "byte");
        let piece = format::count(PIECE as u64, "byte");
        let text = format!("Paste {size} into the terminal? They go in pieces of {piece}.");
        ui.label(RichText::new(text).size(SMALL).color(palette.warn));
        if ui.add(Button::new(RichText::new("Paste").size(SMALL)).small()).clicked() {
            choice = Some(true);
        }
        if ui.add(Button::new(RichText::new("Cancel").size(SMALL)).small()).clicked() {
            choice = Some(false);
        }
    });
    choice
}

/// The settings of the widget.
fn settings(config: &Config) -> Settings {
    let font_size = u16::try_from(config.number(Setting::TerminalFont)).map_or(13.0, f32::from);
    Settings { font_size, keys: KeyOptions { option_as_meta: config.flag(Setting::OptionAsMeta) } }
}

/// The words of a title, for a note and for the tab. Public for the note of a confirm.
pub(crate) fn title_of(request: &TerminalRequest) -> String {
    title_text(request)
}

/// "Shell web-1 · app", "Attach web-1", "Node n1": the words of a title.
fn title_text(request: &TerminalRequest) -> String {
    let program = match request.program {
        Program::Shell => "Shell",
        Program::Attach => "Attach",
        Program::Debug => "Debug",
        Program::NodeShell => "Node",
    };
    let object = short(&request.object.name, 40);
    if request.container.is_empty() {
        format!("{program} {object}")
    } else {
        format!("{program} {object} · {}", short(&request.container, 24))
    }
}
