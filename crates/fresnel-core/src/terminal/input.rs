//! The bytes for the stdin of the remote program and the size for its TTY.
//!
//! The keys, a paste and the answers of the parser wait in one queue with a limit, oldest
//! first. The session takes them in pieces of 16 KiB at most: a paste of 1 MiB in one frame
//! of the WebSocket made the API server lose the end of a shell (spike review, attack A2),
//! and in pieces it arrived whole.

use std::collections::VecDeque;
use std::fmt;

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::{Dimensions, Scroll as GridScroll};
use alacritty_terminal::term::TermMode;

use super::palette::Palette;
use super::session::SessionState;
use super::{GridSize, Terminal};

/// The most bytes that wait for the session. A larger paste is refused whole.
pub const INPUT_LIMIT: usize = 8 * 1024 * 1024;
/// The most bytes that the session takes at once.
pub const PIECE: usize = 16 * 1024;

/// Input that the terminal refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The input did not fit: the queue had room for `room` bytes only.
    Full { bytes: usize, room: usize },
    /// The remote program has no input, as an attach to a container with no stdin.
    NoInput,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full { bytes, room } => {
                write!(f, "the input has {room} free bytes of {INPUT_LIMIT}, too few for {bytes}")
            }
            Self::NoInput => f.write_str("the program has no input: its container has no stdin in its spec"),
        }
    }
}

/// What the two sides of a terminal share besides the screen.
pub(super) struct Io {
    input: VecDeque<u8>,
    /// False for a remote program with no input: each key goes nowhere, so it is refused.
    takes_input: bool,
    /// Bytes of answers of the parser that did not fit.
    refused: u64,
    /// A size that the remote TTY did not get yet.
    size: Option<GridSize>,
    /// The size for an answer to `CSI 14 t`, with the cell size of the widget in pixels.
    pub(super) window: WindowSize,
    /// The colors for an answer to a color request.
    pub(super) palette: Palette,
    pub(super) state: SessionState,
    pub(super) title: Option<String>,
    /// The bells of the remote program since the start.
    pub(super) bells: u64,
}

impl Io {
    pub(super) fn new(size: GridSize) -> Self {
        Self {
            input: VecDeque::new(),
            takes_input: true,
            refused: 0,
            size: None,
            window: window(size, (0, 0)),
            palette: Palette::DARK,
            state: SessionState::Connecting,
            title: None,
            bells: 0,
        }
    }

    pub(super) fn has_input(&self) -> bool {
        !self.input.is_empty() || self.size.is_some()
    }

    /// An answer of the parser. One that does not fit is lost and counted: the remote program
    /// does not read its input.
    pub(super) fn answer(&mut self, bytes: &[u8]) {
        if self.push(bytes).is_err() {
            self.refused = self.refused.saturating_add(bytes.len() as u64);
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), Refused> {
        if !self.takes_input {
            return Err(Refused::NoInput);
        }
        let room = INPUT_LIMIT.saturating_sub(self.input.len());
        if bytes.len() > room {
            return Err(Refused::Full { bytes: bytes.len(), room });
        }
        self.input.extend(bytes);
        Ok(())
    }
}

impl Terminal {
    /// Keys of the user. The view goes to the bottom of the history, where the input shows.
    pub fn send(&self, bytes: &[u8]) -> Result<(), Refused> {
        self.to_bottom();
        self.io().push(bytes)?;
        self.shared.wanted.notify_one();
        Ok(())
    }

    /// A paste of the user, by the rules of alacritty: in bracketed paste mode the text goes
    /// between the brackets with no ESC and no Ctrl-C, which could end them early; else each
    /// line end is a carriage return, as Enter gives. Gives the count of bytes.
    pub fn paste(&self, text: &str) -> Result<usize, Refused> {
        let bracketed = self.shared.screen.lock().term.mode().contains(TermMode::BRACKETED_PASTE);
        let bytes = encode_paste(text, bracketed);
        self.send(&bytes)?;
        Ok(bytes.len())
    }

    /// The next piece of input for the session, of [`PIECE`] bytes at most.
    pub fn take_input(&self) -> Option<Vec<u8>> {
        let mut io = self.io();
        let count = io.input.len().min(PIECE);
        (count > 0).then(|| io.input.drain(..count).collect())
    }

    /// A size that the remote TTY must get, once for each change: during a drag of the
    /// window only the newest one waits.
    pub fn take_size(&self) -> Option<GridSize> {
        self.io().size.take()
    }

    /// The bytes of input that wait for the session.
    pub fn waiting_input(&self) -> usize {
        self.io().input.len()
    }

    /// Drops the input that waits, such as the rest of a long paste. Gives its count of bytes.
    pub fn drop_input(&self) -> usize {
        let mut io = self.io();
        let count = io.input.len();
        io.input.clear();
        count
    }

    /// The remote program has no input: the session tells it before its stream. The keys
    /// that wait and each later key and paste are refused, and the answers of the parser
    /// are counted as lost.
    pub(crate) fn refuse_input(&self) {
        let mut io = self.io();
        io.takes_input = false;
        io.input.clear();
    }

    /// The bytes of answers of the parser that did not fit, or that the remote program could
    /// not take, since the start.
    pub fn refused_answers(&self) -> u64 {
        self.io().refused
    }

    /// The widget has room for `size` with cells of `cell` pixels. A new grid size changes
    /// the grid at once, and waits for the session.
    pub fn resize(&self, size: GridSize, cell: (u16, u16)) {
        let changed = {
            let mut screen = self.shared.screen.lock();
            let changed = (screen.term.columns(), screen.term.screen_lines()) != (size.columns(), size.screen_lines());
            if changed {
                screen.resize(size);
            }
            changed
        };
        let mut io = self.io();
        io.window = window(size, cell);
        if changed {
            io.size = Some(size);
            drop(io);
            self.shared.wanted.notify_one();
        }
    }

    fn to_bottom(&self) {
        let mut screen = self.shared.screen.lock();
        if screen.term.grid().display_offset() > 0 {
            screen.term.scroll_display(GridScroll::Bottom);
            screen.changed();
        }
    }
}

fn window(size: GridSize, (cell_width, cell_height): (u16, u16)) -> WindowSize {
    WindowSize { num_lines: size.rows, num_cols: size.columns, cell_width, cell_height }
}

/// The bytes of a paste.
fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let safe = text.replace(['\x1b', '\x03'], "");
        return [b"\x1b[200~".as_slice(), safe.as_bytes(), b"\x1b[201~"].concat();
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_with_no_brackets_sends_carriage_returns() {
        assert_eq!(encode_paste("ls\nrm x\r\n", false), b"ls\rrm x\r");
    }

    #[test]
    fn a_bracketed_paste_has_no_escape_and_no_ctrl_c_inside() {
        assert_eq!(encode_paste("a\x1b[201~b\x03c\nd", true), b"\x1b[200~a[201~bc\nd\x1b[201~");
    }
}
