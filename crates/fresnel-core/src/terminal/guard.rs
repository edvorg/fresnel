//! The limits in front of the parser for the data that the remote program controls.
//!
//! `vte` keeps the text of an OSC sequence (a title, a link, a color) with no limit: 200 MiB
//! after `ESC ] 0 ;` with no end took 198 MiB of memory (spike review, attack A6). The model
//! keeps up to 4,096 titles on its stack, so the guard cuts each OSC sequence at 4 KiB: the
//! stack holds 16 MiB at most. The model also puts each mark of zero width, such as an
//! accent, on the character before it with no limit; the guard passes 16 marks in a row.
//!
//! The guard follows the states of the parser that matter for this: the text, an escape, a
//! CSI sequence, an OSC sequence and the other strings, which the parser ignores.

use unicode_width::UnicodeWidthChar;

/// The most bytes of one OSC sequence. The rest of it does not reach the parser.
pub(super) const OSC_LIMIT: usize = 4096;
/// The most marks of zero width in a row.
pub(super) const MARKS_LIMIT: usize = 16;

const ESC: u8 = 0x1b;
/// CAN and SUB end each sequence.
const CANCEL: [u8; 2] = [0x18, 0x1a];
const BEL: u8 = 0x07;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Text,
    Escape,
    /// The intermediate bytes of an escape, such as `(` of `ESC ( B`.
    Intermediate,
    Csi,
    Osc,
    /// DCS, SOS, PM and APC, up to ESC.
    Ignored,
}

#[derive(Debug, Default)]
pub(super) struct Guard {
    state: State,
    /// The bytes of the OSC sequence that passed.
    osc: usize,
    /// The marks of zero width since the last character with a width.
    marks: usize,
    /// The bytes of a character of more than one byte, so far.
    utf8: [u8; 4],
    have: usize,
    need: usize,
}

impl Guard {
    /// Copies the bytes of `input` that may reach the parser to `output`.
    pub(super) fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            match self.state {
                State::Text => self.text(byte, output),
                State::Osc => self.osc(byte, output),
                state => {
                    output.push(byte);
                    self.state = next(state, byte);
                    if self.state == State::Osc {
                        self.osc = 0;
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
            (self.have, self.need, self.marks) = (0, 0, 0);
        }
        match byte {
            ESC => {
                output.push(byte);
                self.state = State::Escape;
            }
            0x20..=0x7e => {
                self.marks = 0;
                output.push(byte);
            }
            0x00..=0x7f => output.push(byte),
            0xc2..=0xdf => self.start(byte, 2),
            0xe0..=0xef => self.start(byte, 3),
            0xf0..=0xf4 => self.start(byte, 4),
            _ => {
                self.marks = 0;
                output.push(byte);
            }
        }
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
            Some(Some(0)) => {
                self.marks += 1;
                output.extend_from_slice(self.pending());
            }
            // A control character of more than one byte keeps the count.
            Some(None) => output.extend_from_slice(self.pending()),
            _ => {
                self.marks = 0;
                output.extend_from_slice(self.pending());
            }
        }
        (self.have, self.need) = (0, 0);
    }

    fn osc(&mut self, byte: u8, output: &mut Vec<u8>) {
        match byte {
            BEL => self.state = State::Text,
            ESC => self.state = State::Escape,
            _ if CANCEL.contains(&byte) => self.state = State::Text,
            _ if self.osc >= OSC_LIMIT => return,
            _ => self.osc += 1,
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
        (State::Escape | State::Intermediate, 0x20..=0x2f) => State::Intermediate,
        (State::Csi, 0x30..=0x3f) => State::Csi,
        (State::Escape | State::Intermediate | State::Csi, 0x30..=0x7e) => State::Text,
        (other, _) => other,
    }
}

#[cfg(test)]
mod tests;
