//! The terminal of a pod (plan section 12.8): its screen, its input and the state of its
//! session.
//!
//! A [`Terminal`] is one screen of `alacritty_terminal` with its parser, and the bytes for
//! the stdin of the remote program. Two sides share it:
//!
//! - The session (a stream of an exec or an attach) feeds the output of the remote program
//!   with [`Terminal::feed`], sends what [`Terminal::take_input`] and
//!   [`Terminal::take_size`] give after [`Terminal::wanted`], ends a synchronized update at
//!   [`Terminal::sync_deadline`] and tells its state with [`Terminal::set_state`].
//! - The terminal widget copies the rows on the screen with [`Terminal::frame`], and gives the
//!   keys, a paste, a new size, a scroll and a selection of the user.
//!
//! The parser runs in the session task, not in a frame: egui draws no hidden tab, and a hidden
//! terminal must still take its output. The screen is behind one fair lock that each side
//! holds for a short time: the session for one chunk of 4 KiB, a frame for the copy of the
//! rows on the screen. The painter then works on the copy, with no lock.
//!
//! - `screen`: the model and its parser. `guard`: the limits in front of the parser for the
//!   data that the remote program controls. `events`: its answers, the title and the bell.
//! - `frame`: the copy of a frame. `palette`: the colors. `select`: the selection and the
//!   scroll. `input`: the bytes for stdin and the size for the remote TTY. `session`: the
//!   state of the session and the request of a terminal.
//! - `remote`: the task of a session: the exec or the attach, the new pod of a node shell or
//!   the debug container, the bytes of the stream and the end of the remote program.

mod events;
mod frame;
mod guard;
mod input;
mod palette;
mod remote;
mod screen;
mod select;
mod session;

#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use tokio::sync::Notify;

pub use frame::{Cell, CellFlags, Cursor, CursorShape, Frame, Modes, MouseEncoding, MouseMode};
pub use input::{INPUT_LIMIT, PIECE, Refused};
pub use palette::Palette;
pub use select::{Scroll, SelectKind, Spot};
pub use session::{Program, SessionState, TerminalRequest, TerminalView};

pub(crate) use remote::{Launch, Parts as SessionParts, RemoteSession};

use events::Reporter;
use input::Io;
use screen::Screen;

/// The size of a terminal in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub columns: u16,
    pub rows: u16,
}

impl GridSize {
    /// A smaller grid makes the model and some remote programs fail.
    pub const MIN: Self = Self { columns: 2, rows: 1 };

    /// A size with at least the columns and rows of [`Self::MIN`].
    pub fn new(columns: u16, rows: u16) -> Self {
        Self { columns: columns.max(Self::MIN.columns), rows: rows.max(Self::MIN.rows) }
    }
}

impl Default for GridSize {
    fn default() -> Self {
        Self { columns: 80, rows: 24 }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.columns)
    }
}

/// The settings of a new terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub size: GridSize,
    /// The lines of history above the screen. 10,000 lines of 200 columns use 53 MiB.
    pub scrollback: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { size: GridSize::default(), scrollback: 10_000 }
    }
}

struct Shared {
    screen: FairMutex<Screen>,
    /// The input, the size, the state and the facts of the parser. The parser writes the
    /// answers here while it holds the screen, so no code holds this lock and waits for the
    /// screen.
    io: Arc<Mutex<Io>>,
    /// Wakes the session for new input or a new size.
    wanted: Notify,
}

/// A handle to one terminal. A clone is one more handle to the same terminal.
#[derive(Clone)]
pub struct Terminal {
    shared: Arc<Shared>,
}

impl Terminal {
    pub fn new(options: Options) -> Self {
        let io = Arc::new(Mutex::new(Io::new(options.size)));
        let screen = Screen::new(options, Reporter::new(Arc::clone(&io)));
        Self { shared: Arc::new(Shared { screen: FairMutex::new(screen), io, wanted: Notify::new() }) }
    }

    /// Parses output of the remote program, in chunks: the screen lock stays short, and a
    /// frame that waits gets the lock after one chunk. The caller asks for a repaint after.
    pub fn feed(&self, bytes: &[u8]) {
        for chunk in bytes.chunks(screen::CHUNK) {
            self.shared.screen.lock().feed(chunk);
            if self.io().has_input() {
                // An answer of the parser for the remote program, such as a cursor position.
                self.shared.wanted.notify_one();
            }
        }
    }

    /// Waits until input or a new size waits for the session. A call after the last take
    /// returns at once if something came in between.
    pub async fn wanted(&self) {
        self.shared.wanted.notified().await;
    }

    /// The time at which the synchronized update that the remote program started
    /// (`CSI ? 2026 h`) must end. The session calls [`Terminal::end_sync`] then: the parser
    /// only stores the time, and with no call the screen never shows the output after it.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.shared.screen.lock().sync_deadline()
    }

    /// Ends a synchronized update that the remote program did not end.
    pub fn end_sync(&self) {
        self.shared.screen.lock().end_sync();
    }

    /// Tells the state of the session.
    pub fn set_state(&self, state: SessionState) {
        self.io().state = state;
    }

    /// The state of the session.
    pub fn state(&self) -> SessionState {
        self.io().state.clone()
    }

    /// The size of the grid now. The session sends it first, before any input.
    pub fn size(&self) -> GridSize {
        let screen = self.shared.screen.lock();
        let (columns, rows) = (screen.term.columns(), screen.term.screen_lines());
        GridSize::new(u16::try_from(columns).unwrap_or(u16::MAX), u16::try_from(rows).unwrap_or(u16::MAX))
    }

    /// The window title that the remote program set, cut to its limit.
    pub fn title(&self) -> Option<String> {
        self.io().title.clone()
    }

    /// The count of the bells of the remote program: a view flashes for each new one.
    pub fn bells(&self) -> u64 {
        self.io().bells
    }

    fn io(&self) -> MutexGuard<'_, Io> {
        self.shared.io.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for Terminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Terminal").field("state", &self.state()).finish_non_exhaustive()
    }
}

impl PartialEq for Terminal {
    /// Two handles are equal when they are handles of one terminal.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}

impl Eq for Terminal {}
