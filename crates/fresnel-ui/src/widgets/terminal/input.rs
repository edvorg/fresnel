//! The events of the keyboard in one frame to the actions of the terminal.
//!
//! The copy keys differ between the systems, and egui-winit turns them into events with no
//! key: Cmd+C on macOS and Ctrl+C on Windows give `Event::Copy`, the same for Cut and
//! Paste. On macOS Ctrl+C stays a key (0x03). On Windows a copy with no selection is the
//! byte of Ctrl+C, and a cut the byte of Ctrl+X, as in Windows Terminal.

use egui::{Event, ImeEvent, Key, Modifiers};
use fresnel_core::terminal::{Modes, Scroll};

use super::keymap::{self, KeyOptions};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Action {
    /// Bytes for stdin.
    Send(Vec<u8>),
    Paste(String),
    /// Copy the selection.
    Copy,
    /// Copy the selection; with none, send these bytes.
    CopyOr(Vec<u8>),
    Scroll(Scroll),
}

/// The actions of the keyboard events of one frame. `held` are the modifiers of the frame:
/// a copy event has none of its own.
pub(super) fn translate(events: &[Event], modes: Modes, options: KeyOptions, held: Modifiers) -> Vec<Action> {
    let mut actions = Vec::new();
    // Option as Meta: macOS sends the character that it composed as text too.
    let mut drop_text = false;
    // Ctrl and Alt are AltGr on Windows, which gives a character as text: the text wins.
    let mut alt_graph: Option<Vec<u8>> = None;
    for event in events {
        let text = match event {
            Event::Text(text) | Event::Ime(ImeEvent::Commit(text)) => Some(text),
            _ => None,
        };
        if let Some(bytes) = alt_graph.take()
            && text.is_none()
        {
            actions.push(Action::Send(bytes));
        }
        match event {
            Event::Key { key, pressed: true, modifiers, .. } => {
                drop_text = keymap::meta_char(*key, *modifiers, options).is_some();
                if let Some(scroll) = history_key(*key, *modifiers, modes) {
                    actions.push(Action::Scroll(scroll));
                } else if let Some(bytes) = keymap::encode(*key, *modifiers, modes, options) {
                    if modifiers.ctrl && modifiers.alt {
                        alt_graph = Some(bytes);
                    } else {
                        actions.push(Action::Send(bytes));
                    }
                }
            }
            Event::Text(_) | Event::Ime(ImeEvent::Commit(_)) => {
                if !std::mem::take(&mut drop_text)
                    && let Some(text) = text
                {
                    actions.push(Action::Send(text.as_bytes().to_vec()));
                }
            }
            Event::Paste(text) => actions.push(Action::Paste(text.clone())),
            Event::Copy if held.mac_cmd => actions.push(Action::Copy),
            Event::Copy => actions.push(Action::CopyOr(vec![0x03])),
            Event::Cut if !held.mac_cmd => actions.push(Action::Send(vec![0x18])),
            _ => {}
        }
    }
    if let Some(bytes) = alt_graph {
        actions.push(Action::Send(bytes));
    }
    actions
}

/// Shift with Page Up, Page Down, Home and End moves the view through the history, as in
/// xterm. A program on the alternate screen has no history: it gets the keys.
fn history_key(key: Key, modifiers: Modifiers, modes: Modes) -> Option<Scroll> {
    if !modifiers.shift || modifiers.ctrl || modifiers.alt || modes.alt_screen {
        return None;
    }
    match key {
        Key::PageUp => Some(Scroll::PageUp),
        Key::PageDown => Some(Scroll::PageDown),
        Key::End => Some(Scroll::Bottom),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    const META: KeyOptions = KeyOptions { option_as_meta: true };
    const NONE: Modifiers = Modifiers::NONE;

    fn send(bytes: &[u8]) -> Action {
        Action::Send(bytes.to_vec())
    }

    #[test]
    fn option_as_meta_drops_the_composed_character_and_a_plain_key_is_sent_one_time() {
        let events = [key(Key::B, Modifiers::ALT), Event::Text("\u{222b}".into()), Event::Text("a".into())];
        assert_eq!(translate(&events, Modes::default(), META, NONE), [send(b"\x1bb"), send(b"a")]);
        let composed = [key(Key::B, Modifiers::ALT), Event::Text("\u{222b}".into())];
        assert_eq!(translate(&composed, Modes::default(), KeyOptions::default(), NONE), [send("∫".as_bytes())]);
        let plain = [key(Key::A, NONE), Event::Text("a".into()), Event::Ime(ImeEvent::Commit("é".into()))];
        assert_eq!(translate(&plain, Modes::default(), META, NONE), [send(b"a"), send("é".as_bytes())]);
    }

    #[test]
    fn the_copy_keys_of_macos_and_of_windows() {
        let events = [key(Key::C, Modifiers::CTRL), Event::Copy, Event::Cut];
        let mac = translate(&events, Modes::default(), META, Modifiers::MAC_CMD);
        assert_eq!(mac, [send(b"\x03"), Action::Copy]);
        let windows = translate(&[Event::Copy, Event::Cut], Modes::default(), META, Modifiers::CTRL);
        assert_eq!(windows, [Action::CopyOr(vec![0x03]), send(b"\x18")]);
    }

    #[test]
    fn alt_graph_gives_its_character_and_ctrl_alt_with_no_text_its_bytes() {
        let both = Modifiers { ctrl: true, alt: true, ..NONE };
        let windows = [key(Key::Q, both), Event::Text("@".into())];
        assert_eq!(translate(&windows, Modes::default(), META, NONE), [send(b"@")]);
        let mac = [key(Key::C, both), key(Key::D, both)];
        assert_eq!(translate(&mac, Modes::default(), META, NONE), [send(b"\x1b\x03"), send(b"\x1b\x04")]);
    }

    #[test]
    fn shift_page_up_moves_the_history_and_a_program_on_the_alternate_screen_gets_it() {
        let events = [key(Key::PageUp, Modifiers::SHIFT)];
        assert_eq!(translate(&events, Modes::default(), META, NONE), [Action::Scroll(Scroll::PageUp)]);
        let full = Modes { alt_screen: true, ..Modes::default() };
        assert_eq!(translate(&events, full, META, NONE), [send(b"\x1b[5;2~")]);
        let paste = [Event::Paste("ls\n".into())];
        assert_eq!(translate(&paste, full, META, NONE), [Action::Paste("ls\n".into())]);
    }
}
