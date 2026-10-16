//! The terminals of the dock (plan section 12.8): a tab for each terminal of the core, with
//! the line of the state of its session, the widget, and a question before a long paste.
//!
//! The core holds the terminals, and the tabs follow its list: a tab keeps the state of its
//! widget while its terminal is in the list. A close asks the core to close the terminal.
//!
//! - `line`: the line of the state.

mod line;

use egui::{Button, Frame, Margin, RichText, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::config::Config;
use fresnel_core::config::table::Setting;
use fresnel_core::terminal::{PIECE, Program, Scroll, TerminalRequest, TerminalView};

use crate::format::{self, short};
use crate::shell::{Scene, Tab, Title};
use crate::theme::{self, PAD, SMALL};
use crate::widgets::terminal::{KeyOptions, Settings, TerminalWidget};
use line::Asked;

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
}

impl TerminalTabs {
    /// The title of each terminal of the core, in the sequence of the core.
    pub(crate) fn titles(view: &CoreView) -> Vec<Title> {
        let title = |terminal: &TerminalView| {
            let tone = line::tone(&terminal.terminal.state());
            let (tab, cluster) = (Tab::Terminal(terminal.id), terminal.request.cluster.clone());
            Title { tab, text: title_text(&terminal.request), cluster, tone }
        };
        view.terminals.iter().map(title).collect()
    }

    /// Keeps the tabs of the terminals that the core has.
    pub(crate) fn follow(&mut self, view: &CoreView) {
        self.tabs.retain(|tab| view.terminals.iter().any(|terminal| terminal.id == tab.id));
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
        let shown = tab.widget.show(ui, &view.terminal, settings(&scene.view.config));
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

/// "Shell web-1 · app", "Attach web-1 · app": the words of a title.
fn title_text(request: &TerminalRequest) -> String {
    let program = match request.program {
        Program::Shell => "Shell",
        Program::Attach => "Attach",
    };
    format!("{program} {} · {}", short(&request.pod.name, 40), short(&request.container, 24))
}
