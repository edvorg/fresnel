//! The model of the screen: the grid of `alacritty_terminal`, its parser and the guard in
//! front of the parser.

use std::time::Instant;

use alacritty_terminal::term::{Config, Osc52, Term};
use alacritty_terminal::vte::ansi::Processor;

use super::events::Reporter;
use super::guard::Guard;
use super::{GridSize, Options};

/// The most bytes that the parser reads in one hold of the lock. The cost of a chunk grows
/// with its count of lines: 4 KiB of short lines took 0.6 to 1.6 ms in the spike, 32 KiB up
/// to 17 ms, with the same throughput.
pub(super) const CHUNK: usize = 4 * 1024;

pub(super) struct Screen {
    pub(super) term: Term<Reporter>,
    parser: Processor,
    guard: Guard,
    /// The bytes of one chunk after the guard. The buffer stays for the next chunk.
    guarded: Vec<u8>,
    /// Grows with each change that a frame can show, so a frame copies the rows only after
    /// a change.
    pub(super) generation: u64,
}

impl Screen {
    pub(super) fn new(options: Options, reporter: Reporter) -> Self {
        let term = Term::new(config(options.scrollback), &options.size, reporter);
        Self {
            term,
            parser: Processor::new(),
            guard: Guard::default(),
            guarded: Vec::with_capacity(CHUNK),
            generation: 0,
        }
    }

    /// Parses one chunk of output.
    pub(super) fn feed(&mut self, chunk: &[u8]) {
        self.guarded.clear();
        self.guard.filter(chunk, &mut self.guarded);
        self.parser.advance(&mut self.term, &self.guarded);
        self.generation += 1;
    }

    /// A new size: the grid flows wrapped lines again.
    pub(super) fn resize(&mut self, size: GridSize) {
        self.term.resize(size);
        self.generation += 1;
    }

    /// A new count of lines of history. Fewer lines drop the oldest ones.
    pub(super) fn set_scrollback(&mut self, lines: usize) {
        self.term.set_options(config(lines));
        self.generation += 1;
    }

    pub(super) fn sync_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    pub(super) fn end_sync(&mut self) {
        self.parser.stop_sync(&mut self.term);
        self.generation += 1;
    }

    /// Marks a change that is not output, such as a scroll or a selection.
    pub(super) fn changed(&mut self) {
        self.generation += 1;
    }
}

fn config(scrollback: usize) -> Config {
    // The default of OSC 52 lets a remote program write the clipboard of the user.
    Config { scrolling_history: scrollback, osc52: Osc52::Disabled, ..Config::default() }
}
