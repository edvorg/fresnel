//! The limits in front of the parser for the data that the remote program controls. The
//! numbers and the reasons are in `docs/decisions.md`.
//!
//! - OSC: `vte` keeps its text with no limit (200 MiB after `ESC ] 0 ;` took 198 MiB), and the
//!   model keeps up to 4,096 titles: the guard passes 4 KiB of each sequence, so the stack
//!   holds 16 MiB at most. Of a link (OSC 8), whose copy each cell after it keeps in the
//!   history, it passes only the start: the widget opens no link.
//! - Marks: the model puts each mark of zero width, such as an accent, on the character
//!   before it with no limit; the guard passes 16 in a row.
//! - Repeat (`CSI Ps b`): the model repeats the last character up to 65,535 times for 8
//!   bytes. The guard gives each repeat a limit, and drops the repeat of a mark, as xterm.
//!
//! The guard follows the states of the parser that matter for this: the text, an escape, a
//! CSI sequence, an OSC sequence and the other strings, which the parser ignores.

use unicode_width::UnicodeWidthChar;

/// The most bytes of one OSC sequence. The rest of it does not reach the parser.
pub(super) const OSC_LIMIT: usize = 4096;
/// The start of a link. Only this start of a link passes, and the parser ignores it.
const LINK: [u8; 2] = *b"8;";
/// The most marks of zero width in a row.
pub(super) const MARKS_LIMIT: usize = 16;
/// The most characters of one repeat. ncurses repeats a character within one line, and a
/// line of the widest screen has fewer cells. 4 KiB of such repeats took 4.5 ms.
pub(super) const REPEAT_LIMIT: u16 = 2048;

const ESC: u8 = 0x1b;
/// CAN and SUB end each sequence.
const CANCEL: [u8; 2] = [CAN, 0x1a];
const CAN: u8 = 0x18;
const BEL: u8 = 0x07;
/// The end of a string in 8 bits. The parser ends DCS data with it.
const ST: u8 = 0x9c;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Text,
    Escape,
    /// The intermediate bytes of an escape, such as `(` of `ESC ( B`.
    Intermediate,
    Csi,
    Osc,
    /// DCS, SOS, PM and APC, up to ESC or ST.
    Ignored,
}

#[derive(Debug, Default)]
pub(super) struct Guard {
    state: State,
    /// The bytes of the OSC sequence that passed.
    osc: usize,
    /// The first two bytes of the OSC sequence, which tell a link.
    osc_start: [u8; 2],
    /// The marks of zero width since the last character with a width.
    marks: usize,
    /// The bytes of a character of more than one byte, so far.
    utf8: [u8; 4],
    have: usize,
    need: usize,
    /// The last character that the parser printed has no width: a repeat would repeat it.
    after_mark: bool,
    /// The CSI sequence so far.
    csi: Csi,
}

/// What a repeat needs of a CSI sequence: its first parameter, and if it can be a repeat.
#[derive(Debug, Default)]
struct Csi {
    /// The first parameter, as the parser reads it: 65,535 at most.
    first: u16,
    /// A separator ended the first parameter.
    after_first: bool,
    /// A private mark (`?`) or an intermediate byte: no repeat.
    other: bool,
}

impl Guard {
    /// Copies the bytes of `input` that may reach the parser to `output`.
    pub(super) fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            match self.state {
                State::Text => self.text(byte, output),
                State::Osc => self.osc(byte, output),
                State::Csi => self.csi(byte, output),
                state => {
                    output.push(byte);
                    self.state = next(state, byte);
                    match self.state {
                        State::Osc => (self.osc, self.osc_start) = (0, [0; 2]),
                        State::Csi => self.csi = Csi::default(),
                        _ => {}
                    }
                }
            }
        }
    }

    fn text(&mut self, byte: u8, output: &mut Vec<u8>) {
        if self.need > 0 {
            if byte & 0xc0 == 0x80 {
                if let Some(slot) = self.utf8.get_mut(self.have) {
                    *slot = byte;
                }
                self.have += 1;
                if self.have >= self.need {
                    self.character(output);
                }
                return;
            }
            // A cut sequence: the parser shows a replacement character, which has a width.
            output.extend_from_slice(self.pending());
            (self.have, self.need) = (0, 0);
            self.printed(1);
        }
        match byte {
            ESC => {
                output.push(byte);
                self.state = State::Escape;
            }
            0x20..=0x7e => {
                self.printed(1);
                output.push(byte);
            }
            // A control, also one of 8 bits from 0x80 on, and DEL: the parser prints nothing.
            0x00..=0x9f => output.push(byte),
            0xc2..=0xdf => self.start(byte, 2),
            0xe0..=0xef => self.start(byte, 3),
            0xf0..=0xf4 => self.start(byte, 4),
            // A byte that no character starts with: the parser shows a replacement character.
            _ => {
                self.printed(1);
                output.push(byte);
            }
        }
    }

    /// The parser printed a character of this width.
    fn printed(&mut self, width: usize) {
        if width == 0 {
            self.marks += 1;
        } else {
            self.marks = 0;
        }
        self.after_mark = width == 0;
    }

    fn start(&mut self, byte: u8, need: usize) {
        self.utf8[0] = byte;
        (self.have, self.need) = (1, need);
    }

    fn pending(&self) -> &[u8] {
        self.utf8.get(..self.have.min(self.utf8.len())).unwrap_or_default()
    }

    /// A whole character of more than one byte.
    fn character(&mut self, output: &mut Vec<u8>) {
        let character = std::str::from_utf8(self.pending()).ok().and_then(|text| text.chars().next());
        match character.map(|character| character.width()) {
            Some(Some(0)) if self.marks >= MARKS_LIMIT => {}
            // A control character of more than one byte keeps the count.
            Some(None) => output.extend_from_slice(self.pending()),
            width => {
                // A sequence that is no character shows as a replacement character.
                self.printed(width.flatten().unwrap_or(1));
                output.extend_from_slice(self.pending());
            }
        }
        (self.have, self.need) = (0, 0);
    }

    /// A byte of a CSI sequence. The final byte of a repeat goes as CAN, which ends the
    /// sequence with no effect, and a repeat with the limit follows; none after a mark.
    fn csi(&mut self, byte: u8, output: &mut Vec<u8>) {
        let csi = &mut self.csi;
        match byte {
            b'0'..=b'9' if !csi.after_first => {
                csi.first = csi.first.saturating_mul(10).saturating_add(u16::from(byte - b'0'));
            }
            b';' | b':' => csi.after_first = true,
            0x20..=0x2f | 0x3c..=0x3f => csi.other = true,
            b'b' if !csi.other => {
                output.push(CAN);
                if !self.after_mark {
                    let count = csi.first.clamp(1, REPEAT_LIMIT);
                    output.extend_from_slice(format!("\x1b[{count}b").as_bytes());
                }
                self.state = State::Text;
                return;
            }
            _ => {}
        }
        output.push(byte);
        self.state = next(State::Csi, byte);
    }

    fn osc(&mut self, byte: u8, output: &mut Vec<u8>) {
        match byte {
            BEL => self.state = State::Text,
            ESC => self.state = State::Escape,
            _ if CANCEL.contains(&byte) => self.state = State::Text,
            _ if self.osc >= OSC_LIMIT || self.osc_start == LINK => return,
            _ => {
                if let Some(slot) = self.osc_start.get_mut(self.osc) {
                    *slot = byte;
                }
                self.osc += 1;
            }
        }
        output.push(byte);
    }
}

/// The state after a byte in a state of an escape or a sequence that the guard passes whole.
fn next(state: State, byte: u8) -> State {
    if byte == ESC {
        return State::Escape;
    }
    if CANCEL.contains(&byte) {
        return State::Text;
    }
    match (state, byte) {
        (State::Escape, b'[') => State::Csi,
        (State::Escape, b']') => State::Osc,
        (State::Escape, b'P' | b'X' | b'^' | b'_') => State::Ignored,
        // ST ends DCS data in the parser, but no other string: in one of those the guard then
        // counts marks that the parser ignores anyway.
        (State::Ignored, ST) => State::Text,
        (State::Escape | State::Intermediate, 0x20..=0x2f) => State::Intermediate,
        (State::Csi, 0x30..=0x3f) => State::Csi,
        (State::Escape | State::Intermediate | State::Csi, 0x30..=0x7e) => State::Text,
        (other, _) => other,
    }
}

#[cfg(test)]
mod tests;
