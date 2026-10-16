//! The pointer in the terminal: the selection of the user, the wheel, or reports for a
//! remote program that asked for them (modes 1000, 1002 and 1003 of xterm).
//!
//! With a mouse mode on, a press, a release, a move and the wheel go to the program; with
//! Shift they stay with the user, as in xterm. A drag selects text; with Alt, a rectangle.
//! Two clicks select a word, three a line, and one click ends the selection.

use egui::{Event, Modifiers, PointerButton, Pos2, Response, Ui};
use fresnel_core::terminal::{Frame, MouseEncoding, MouseMode, Scroll, SelectKind, Spot, Terminal};

use super::metrics::CellMetrics;

/// The button codes of a report.
const RELEASE: u8 = 3;
const MOTION: u8 = 32;
const WHEEL_UP: u8 = 64;
const WHEEL_DOWN: u8 = 65;

/// The state of the pointer between frames.
#[derive(Debug, Default)]
pub(super) struct Mouse {
    /// The part of a line of the wheel that did not move the view yet.
    wheel_rest: f32,
    /// The button that the program got a press of.
    held: Option<u8>,
    /// The cell of the last report of a move: a program gets one report for each cell.
    last: Option<(usize, usize)>,
}

/// Where the grid is, and how big its cells are.
pub(super) struct Place {
    pub(super) origin: Pos2,
    pub(super) metrics: CellMetrics,
}

impl Mouse {
    /// Handles the pointer of one frame. Gives the bytes for the program.
    pub(super) fn handle(
        &mut self,
        ui: &Ui,
        (response, place): (&Response, &Place),
        terminal: &Terminal,
        frame: &Frame,
    ) -> Vec<u8> {
        let modifiers = ui.input(|input| input.modifiers);
        let reports = frame.modes.mouse != MouseMode::Off && !modifiers.shift;
        let mut bytes = Vec::new();
        if reports {
            self.report(ui, response, place, frame, &mut bytes);
        } else {
            (self.held, self.last) = (None, None);
            select(ui, response, place, terminal, modifiers);
        }
        if let Some(pointer) = response.hover_pos() {
            self.wheel_rest += ui.input(|input| input.smooth_scroll_delta.y) / place.metrics.height;
            let lines = self.wheel_rest.trunc();
            self.wheel_rest -= lines;
            let spot = clamp(place.metrics.spot(place.origin, pointer), frame);
            wheel(lines as i32, (reports, modifiers, spot), terminal, frame, &mut bytes);
        }
        bytes
    }

    /// The reports of the presses, releases and moves of this frame over the grid.
    fn report(&mut self, ui: &Ui, response: &Response, place: &Place, frame: &Frame, bytes: &mut Vec<u8>) {
        let events = ui.input(|input| input.events.clone());
        for event in events {
            match event {
                Event::PointerButton { pos, button, pressed, modifiers } if response.rect.contains(pos) || !pressed => {
                    let Some(code) = button_code(button) else { continue };
                    let spot = clamp(place.metrics.spot(place.origin, pos), frame);
                    if pressed {
                        self.held = Some(code);
                    } else if self.held.take().is_none() {
                        continue;
                    }
                    let release = !pressed;
                    bytes.extend(encode(frame.modes.mouse_encoding, (code, release), modifiers, spot));
                    self.last = Some((spot.row, spot.column));
                }
                Event::PointerMoved(pos) if response.rect.contains(pos) => {
                    let spot = clamp(place.metrics.spot(place.origin, pos), frame);
                    let wanted = match (frame.modes.mouse, self.held) {
                        (MouseMode::Motion, held) => Some(held.unwrap_or(RELEASE)),
                        (MouseMode::Drag, Some(held)) => Some(held),
                        _ => None,
                    };
                    if let Some(code) = wanted
                        && self.last != Some((spot.row, spot.column))
                    {
                        let modifiers = ui.input(|input| input.modifiers);
                        bytes.extend(encode(frame.modes.mouse_encoding, (code + MOTION, false), modifiers, spot));
                        self.last = Some((spot.row, spot.column));
                    }
                }
                _ => {}
            }
        }
    }
}

/// The wheel: reports for a program that asked for them, arrow keys for a program on the
/// alternate screen, or else a move through the history.
fn wheel(
    lines: i32,
    (reports, modifiers, spot): (bool, Modifiers, Spot),
    terminal: &Terminal,
    frame: &Frame,
    bytes: &mut Vec<u8>,
) {
    if lines == 0 {
        return;
    }
    let count = lines.unsigned_abs() as usize;
    if reports {
        let code = if lines > 0 { WHEEL_UP } else { WHEEL_DOWN };
        for _ in 0..count {
            bytes.extend(encode(frame.modes.mouse_encoding, (code, false), modifiers, spot));
        }
    } else if frame.modes.alt_screen {
        if frame.modes.alternate_scroll {
            let arrow: &[u8] = match (lines > 0, frame.modes.app_cursor) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            bytes.extend(arrow.repeat(count));
        }
    } else {
        terminal.scroll(Scroll::Lines(lines));
    }
}

/// The selection of the user.
fn select(ui: &Ui, response: &Response, place: &Place, terminal: &Terminal, modifiers: Modifiers) {
    let Some(pos) = response.interact_pointer_pos() else { return };
    let spot = place.metrics.spot(place.origin, pos);
    if response.triple_clicked() {
        terminal.select(SelectKind::Lines, spot);
    } else if response.double_clicked() {
        terminal.select(SelectKind::Words, spot);
    } else if response.drag_started() {
        // egui starts a drag after some points of move. The selection starts at the press.
        let start = ui.input(|input| input.pointer.press_origin()).unwrap_or(pos);
        let kind = if modifiers.alt { SelectKind::Block } else { SelectKind::Cells };
        terminal.select(kind, place.metrics.spot(place.origin, start));
        terminal.select_to(spot);
    } else if response.dragged() {
        terminal.select_to(spot);
    } else if response.clicked() {
        terminal.clear_selection();
    }
}

fn button_code(button: PointerButton) -> Option<u8> {
    match button {
        PointerButton::Primary => Some(0),
        PointerButton::Middle => Some(1),
        PointerButton::Secondary => Some(2),
        PointerButton::Extra1 | PointerButton::Extra2 => None,
    }
}

/// A place inside the grid of the frame.
fn clamp(spot: Spot, frame: &Frame) -> Spot {
    let last = |count: u16| usize::from(count).saturating_sub(1);
    Spot { row: spot.row.min(last(frame.size.rows)), column: spot.column.min(last(frame.size.columns)), ..spot }
}

/// One report: `code` with the release flag, the modifiers and the cell, in the form that
/// the program asked for. The normal form cannot tell a cell after column 222: none then.
pub(super) fn encode(encoding: MouseEncoding, (code, release): (u8, bool), modifiers: Modifiers, at: Spot) -> Vec<u8> {
    let held = u8::from(modifiers.shift) * 4 + u8::from(modifiers.alt) * 8 + u8::from(modifiers.ctrl) * 16;
    let (column, row) = (at.column + 1, at.row + 1);
    match encoding {
        MouseEncoding::Sgr => {
            let end = if release { 'm' } else { 'M' };
            format!("\x1b[<{};{column};{row}{end}", code + held).into_bytes()
        }
        MouseEncoding::Normal | MouseEncoding::Utf8 => {
            let code = if release { RELEASE } else { code } + held;
            let mut out = vec![0x1b, b'[', b'M', 32 + code];
            for value in [column, row] {
                let value = u32::try_from(value + 32).unwrap_or(u32::MAX);
                match (encoding, u8::try_from(value), char::from_u32(value)) {
                    (MouseEncoding::Normal, Ok(byte), _) => out.push(byte),
                    (MouseEncoding::Utf8, _, Some(c)) if value < 2048 => {
                        out.extend(c.encode_utf8(&mut [0; 4]).as_bytes());
                    }
                    _ => return Vec::new(),
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(row: usize, column: usize) -> Spot {
        Spot { row, column, right: false }
    }

    #[test]
    fn a_report_has_the_form_that_the_program_asked_for() {
        let none = Modifiers::NONE;
        assert_eq!(encode(MouseEncoding::Sgr, (0, false), none, at(4, 9)), b"\x1b[<0;10;5M");
        assert_eq!(encode(MouseEncoding::Sgr, (0, true), Modifiers::CTRL, at(4, 9)), b"\x1b[<16;10;5m");
        assert_eq!(encode(MouseEncoding::Sgr, (WHEEL_DOWN, false), none, at(0, 0)), b"\x1b[<65;1;1M");
        assert_eq!(encode(MouseEncoding::Normal, (2, false), none, at(0, 0)), b"\x1b[M\x22\x21\x21");
        assert_eq!(encode(MouseEncoding::Normal, (0, true), Modifiers::SHIFT, at(1, 2)), b"\x1b[M\x27\x23\x22");
        assert_eq!(encode(MouseEncoding::Normal, (0, false), none, at(0, 300)), b"", "no form for column 301");
        assert_eq!(encode(MouseEncoding::Utf8, (0, false), none, at(0, 300)), "\x1b[M \u{14d}!".as_bytes());
    }

    #[test]
    fn the_wheel_sends_arrows_on_the_alternate_screen_and_moves_the_history_of_the_normal_one() {
        let terminal = Terminal::new(fresnel_core::terminal::Options::default());
        let copy = |terminal: &Terminal| {
            let mut frame = Frame::default();
            terminal.frame(&mut frame, &fresnel_core::terminal::Palette::DARK);
            frame
        };
        let mut bytes = Vec::new();
        terminal.feed(&b"line\r\n".repeat(40));
        wheel(3, (false, Modifiers::NONE, at(0, 0)), &terminal, &copy(&terminal), &mut bytes);
        assert_eq!((bytes.as_slice(), copy(&terminal).offset), (b"".as_slice(), 3));
        terminal.feed(b"\x1b[?1049h");
        wheel(2, (false, Modifiers::NONE, at(0, 0)), &terminal, &copy(&terminal), &mut bytes);
        terminal.feed(b"\x1b[?1h");
        wheel(-1, (false, Modifiers::NONE, at(0, 0)), &terminal, &copy(&terminal), &mut bytes);
        assert_eq!(bytes, b"\x1b[A\x1b[A\x1bOB");
        wheel(1, (true, Modifiers::NONE, at(2, 4)), &terminal, &copy(&terminal), &mut bytes);
        assert!(bytes.ends_with(b"\x1b[M\x60\x25\x23"), "a report in the normal form: {bytes:?}");
    }
}
