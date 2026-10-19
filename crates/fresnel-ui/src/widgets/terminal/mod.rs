//! The terminal widget (plan section 12.8): it paints the copy of a [`Terminal`] and gives
//! the terminal the keys, a paste, the pointer and its size.
//!
//! The widget copies the rows on the screen under the short lock of the terminal and paints
//! the copy with no lock. A new size of its area resizes the grid at once and waits in the
//! terminal for the session, which tells the remote TTY. Each row on the screen is a node of
//! the accessibility tree with its text, for a screen reader and for the tests.
//!
//! - `keymap`: the bytes of a key. `input`: the events of the keyboard. `mouse`: the pointer.
//! - `metrics`: the size of a cell. `paint`: the shapes. `fonts`: the faces of the system.
//! - `bell`: the flash of a bell, on the grid and on the tab.

pub(crate) mod bell;
pub(crate) mod fonts;
mod input;
mod keymap;
mod metrics;
mod mouse;
mod paint;

use egui::output::IMEOutput;
use egui::{EventFilter, IMEPurpose, Id, Rect, Sense, Ui};
use fresnel_core::terminal::{Frame, GridSize, Palette, Refused, Terminal};

use crate::theme;
use input::Action;
pub(crate) use keymap::KeyOptions;
use metrics::CellMetrics;
use mouse::{Mouse, Place};

/// The terminal keeps Tab, the arrows and Escape: egui must not move the focus with them.
const KEEP_KEYS: EventFilter = EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };
/// A larger paste waits for a choice of the user.
pub(crate) const ASK_ABOVE: usize = 64 * 1024;

/// What the settings give the widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
    pub(crate) font_size: f32,
    pub(crate) keys: KeyOptions,
}

/// What happened in one frame.
#[derive(Debug, Default)]
pub(crate) struct Shown {
    /// A paste above [`ASK_ABOVE`]: the caller asks the user, then calls `Terminal::paste`.
    pub(crate) paste: Option<String>,
    /// Input that did not fit.
    pub(crate) refused: Option<Refused>,
    /// True when some input went to the terminal.
    pub(crate) sent: bool,
    /// True while the widget has the focus of the keys.
    pub(crate) focused: bool,
}

/// The state of the widget of one terminal between frames.
#[derive(Debug, Default)]
pub(crate) struct TerminalWidget {
    frame: Frame,
    mouse: Mouse,
    /// The size that the terminal got last.
    sized: Option<(GridSize, (u16, u16))>,
    focused: bool,
    /// The widget takes the focus in its next frame.
    refocus: bool,
    bell: bell::Bell,
}

impl TerminalWidget {
    /// The size of the grid in the last frame.
    pub(crate) fn size(&self) -> GridSize {
        self.frame.size
    }

    /// The lines of history above the screen, and the lines of history, in the last frame.
    pub(crate) fn history(&self) -> (usize, usize) {
        (self.frame.offset, self.frame.history)
    }

    /// Gives the widget the focus of the keys again in its next frame, such as after a click
    /// on a button above it.
    pub(crate) fn refocus(&mut self) {
        self.refocus = true;
    }

    /// Draws the terminal in the rest of `ui` and handles its input.
    pub(crate) fn show(&mut self, ui: &mut Ui, terminal: &Terminal, settings: Settings) -> Shown {
        let faces = fonts::faces(ui.ctx(), settings.font_size);
        let metrics = CellMetrics::measure(ui, &faces.regular, &faces.bold);
        let area = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        ui.advance_cursor_after_rect(area);
        let id = ui.id().with("terminal");
        let response = ui.interact(area, id, Sense::click_and_drag());
        crate::widgets::name(&response, "Terminal");
        let size = (metrics.grid_size(area.size()), metrics.pixels());
        if self.sized != Some(size) {
            terminal.resize(size.0, size.1);
            self.sized = Some(size);
        }
        // The press takes the focus: a program that reads the pointer gets the focus report
        // before the press.
        if response.is_pointer_button_down_on() || std::mem::take(&mut self.refocus) {
            response.request_focus();
        }
        let focused = response.has_focus();
        let mut shown = Shown { focused, ..Shown::default() };
        let mut bytes = Vec::new();
        if focused != self.focused && self.frame.modes.focus_reports {
            bytes.extend_from_slice(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
        self.focused = focused;
        let place = Place { origin: area.min, metrics };
        bytes.extend(self.mouse.handle(ui, (&response, &place), terminal, &self.frame));
        if focused {
            ui.memory_mut(|memory| memory.set_focus_lock_filter(id, KEEP_KEYS));
            let held = ui.input(|input| input.modifiers);
            let actions = ui.input(|input| input::translate(&input.events, self.frame.modes, settings.keys, held));
            for action in actions {
                act(ui, terminal, action, (&mut bytes, &mut shown));
            }
        }
        if !bytes.is_empty() {
            match terminal.send(&bytes) {
                Ok(()) => shown.sent = true,
                Err(refused) => shown.refused = Some(refused),
            }
        }
        // The colors of the terminal follow the theme of the app.
        let palette = if ui.visuals().dark_mode { Palette::DARK } else { Palette::LIGHT };
        terminal.frame(&mut self.frame, &palette);
        let look = paint::Look {
            regular: faces.regular,
            bold: faces.bold,
            bold_face: faces.bold_face,
            background: paint::color(palette.background),
            selection: theme::palette(ui).accent_bg,
            cursor: paint::color(palette.cursor),
            focused,
        };
        paint::paint(&ui.painter_at(area), &self.frame, (area, metrics), &look);
        if self.bell.flashes(ui.ctx(), terminal.bells()) {
            ui.painter_at(area).rect_filled(area, 0.0, theme::palette(ui).text.gamma_multiply(0.15));
            bell::describe(&response);
        }
        if focused {
            self.ime(ui, area, metrics);
        }
        self.rows(ui, id, area, metrics);
        shown
    }

    /// Tells egui where the cursor is, so an input method of the system can compose text.
    fn ime(&self, ui: &Ui, area: Rect, metrics: CellMetrics) {
        let cursor = self.frame.cursor.map_or(area, |at| metrics.cells_rect(area.min, at.column, at.row, 1));
        let ime = IMEOutput {
            purpose: IMEPurpose::Terminal,
            rect: area,
            cursor_rect: cursor,
            should_interrupt_composition: false,
        };
        ui.ctx().output_mut(|output| output.ime = Some(ime));
    }

    /// A node of the accessibility tree for each row on the screen, with its text.
    fn rows(&self, ui: &Ui, id: Id, area: Rect, metrics: CellMetrics) {
        for row in 0..usize::from(self.frame.size.rows) {
            let rect = metrics.cells_rect(area.min, 0, row, usize::from(self.frame.size.columns)).intersect(area);
            crate::widgets::row_node(ui, rect, id.with(row), &self.frame.text(row));
        }
    }
}

/// Runs one action of the keyboard.
fn act(ui: &Ui, terminal: &Terminal, action: Action, (bytes, shown): (&mut Vec<u8>, &mut Shown)) {
    match action {
        Action::Send(more) => bytes.extend(more),
        Action::Paste(text) if text.len() > ASK_ABOVE => shown.paste = Some(text),
        Action::Paste(text) => {
            // The keys before the paste go first.
            match terminal.send(&std::mem::take(bytes)).and_then(|()| terminal.paste(&text)) {
                Ok(_) => shown.sent = true,
                Err(refused) => shown.refused = Some(refused),
            }
        }
        Action::Copy => {
            if let Some(text) = terminal.selection_text() {
                ui.ctx().copy_text(text);
            }
        }
        Action::CopyOr(otherwise) => match terminal.selection_text() {
            Some(text) => ui.ctx().copy_text(text),
            None => bytes.extend(otherwise),
        },
        Action::Scroll(scroll) => terminal.scroll(scroll),
    }
}
