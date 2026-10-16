//! The events of egui as acts of the editor, with the keys of macOS (Option for a word,
//! Cmd for a line) and of Windows (Ctrl for a word, Home and End for a line).

use egui::{Event, ImeEvent, Key, Modifiers};

use super::doc::{Act, Unit};
use super::moves::Motion;

/// What an event asks of the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Input {
    Act(Act),
    Copy,
    Cut,
}

/// The input of one event, if the editor has one for it. `mac` selects the keys of macOS.
pub(crate) fn of(event: &Event, mac: bool) -> Option<Input> {
    let act = match event {
        Event::Text(text) if !text.chars().any(char::is_control) => Act::Type(text.clone()),
        Event::Ime(ImeEvent::Commit(text)) if !text.is_empty() => Act::Type(text.clone()),
        Event::Paste(text) => Act::Insert(text.clone()),
        Event::Copy => return Some(Input::Copy),
        Event::Cut => return Some(Input::Cut),
        Event::Key { key, pressed: true, modifiers, .. } => key_act(*key, *modifiers, mac)?,
        _ => return None,
    };
    Some(Input::Act(act))
}

fn key_act(key: Key, modifiers: Modifiers, mac: bool) -> Option<Act> {
    let word = if mac { modifiers.alt } else { modifiers.ctrl };
    let line = mac && modifiers.command;
    let extend = modifiers.shift;
    let go = |motion| Some(Act::Move { motion, extend });
    let unit = if line {
        Unit::Line
    } else if word {
        Unit::Word
    } else {
        Unit::Character
    };
    match key {
        Key::ArrowLeft if line => go(Motion::LineStart),
        Key::ArrowLeft => go(if word { Motion::WordLeft } else { Motion::Left }),
        Key::ArrowRight if line => go(Motion::LineEnd),
        Key::ArrowRight => go(if word { Motion::WordRight } else { Motion::Right }),
        Key::ArrowUp => go(if line { Motion::Start } else { Motion::Up }),
        Key::ArrowDown => go(if line { Motion::End } else { Motion::Down }),
        Key::Home => go(if modifiers.command { Motion::Start } else { Motion::LineStart }),
        Key::End => go(if modifiers.command { Motion::End } else { Motion::LineEnd }),
        Key::PageUp => go(Motion::PageUp),
        Key::PageDown => go(Motion::PageDown),
        Key::Enter if !modifiers.command => Some(Act::Newline),
        Key::Tab if !modifiers.command && !modifiers.alt => Some(if extend { Act::Unindent } else { Act::Indent }),
        Key::Backspace => Some(Act::Delete { forward: false, unit }),
        Key::Delete => Some(Act::Delete { forward: true, unit }),
        Key::A if modifiers.command => Some(Act::SelectAll),
        Key::Z if modifiers.command => Some(if extend { Act::Redo } else { Act::Undo }),
        Key::Y if modifiers.command => Some(Act::Redo),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, modifiers: Modifiers, mac: bool) -> Option<Input> {
        of(&Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }, mac)
    }

    #[test]
    fn the_keys_of_each_system_give_their_moves_and_edits() {
        let act = |act| Some(Input::Act(act));
        let word_left = act(Act::Move { motion: Motion::WordLeft, extend: true });
        assert_eq!(key(Key::ArrowLeft, Modifiers::ALT | Modifiers::SHIFT, true), word_left);
        assert_eq!(key(Key::ArrowLeft, Modifiers::CTRL | Modifiers::SHIFT, false), word_left);
        let line_start = act(Act::Move { motion: Motion::LineStart, extend: false });
        assert_eq!(key(Key::ArrowLeft, Modifiers::MAC_CMD | Modifiers::COMMAND, true), line_start);
        assert_eq!(key(Key::Home, Modifiers::NONE, false), line_start);
        let line = act(Act::Delete { forward: false, unit: Unit::Line });
        assert_eq!(key(Key::Backspace, Modifiers::MAC_CMD | Modifiers::COMMAND, true), line);
        assert_eq!(key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT, true), act(Act::Redo));
        assert_eq!(key(Key::Tab, Modifiers::SHIFT, false), act(Act::Unindent));
        assert_eq!(key(Key::K, Modifiers::COMMAND, true), None, "a command of the app");
        assert_eq!(of(&Event::Text("\u{7f}".to_owned()), true), None, "no control character");
        assert_eq!(of(&Event::Cut, false), Some(Input::Cut));
    }
}
