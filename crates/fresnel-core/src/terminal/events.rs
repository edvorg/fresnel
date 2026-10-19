//! What the parser reports: answers for the remote program, the title and the bell.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use alacritty_terminal::event::{Event, EventListener};

use super::input::Io;

/// The most characters of a title that the app keeps. A title is one line of a tab; the
/// guard already cut its sequence to 4 KiB.
pub(super) const TITLE_LIMIT: usize = 200;

/// The listener of the model. The model calls it during a parse, under the lock of the
/// screen, so it takes only the small lock of the input.
pub(super) struct Reporter {
    io: Arc<Mutex<Io>>,
}

impl Reporter {
    pub(super) fn new(io: Arc<Mutex<Io>>) -> Self {
        Self { io }
    }

    fn io(&self) -> MutexGuard<'_, Io> {
        self.io.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl EventListener for Reporter {
    fn send_event(&self, event: Event) {
        let mut io = self.io();
        match event {
            // A cursor position, the device attributes and the like.
            Event::PtyWrite(text) => io.answer(text.as_bytes()),
            Event::TextAreaSizeRequest(format) => {
                let text = format(io.window);
                io.answer(text.as_bytes());
            }
            Event::ColorRequest(index, format) => {
                let text = format(io.palette.by_index(index));
                io.answer(text.as_bytes());
            }
            Event::Title(title) => io.title = Some(clean(&title)),
            Event::ResetTitle => io.title = None,
            // The app has no sound for a bell: a view flashes for each new one.
            Event::Bell => io.bells = io.bells.wrapping_add(1),
            // OSC 52 is off, so no clipboard event comes. The others belong to a local PTY and a
            // window of alacritty.
            Event::ClipboardStore(..)
            | Event::ClipboardLoad(..)
            | Event::CursorBlinkingChange
            | Event::MouseCursorDirty
            | Event::Wakeup
            | Event::Exit
            | Event::ChildExit(_) => {}
        }
    }
}

/// A title with no control character, cut at its limit.
fn clean(title: &str) -> String {
    title.chars().filter(|c| !c.is_control()).take(TITLE_LIMIT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_loses_its_control_characters_and_its_end_after_the_limit() {
        assert_eq!(clean("top\u{7}\u{1b}[31m - web-1"), "top[31m - web-1");
        assert_eq!(clean(&"x".repeat(1000)).chars().count(), TITLE_LIMIT);
    }
}
