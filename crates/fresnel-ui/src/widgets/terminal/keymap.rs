//! Key presses to bytes, as xterm sends them (research 05, section 4.6).
//!
//! egui-winit on macOS: a key with no Ctrl and no Cmd gives `Event::Key` and then
//! `Event::Text`; a key with Ctrl gives only `Event::Key`. So this map gives the bytes of the
//! keys that give no text, and the text event gives the rest. Cmd belongs to the app.

use egui::{Key, Modifiers};
use fresnel_core::terminal::Modes;

const ESC: u8 = 0x1b;

/// How the keys act, from the settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyOptions {
    /// Option (Alt) and a key send ESC and the key (Meta), not the character that macOS
    /// composes.
    pub(crate) option_as_meta: bool,
}

/// The bytes of one key press. `None`: the key gives text (its text event carries it), or the
/// key belongs to the app.
pub(super) fn encode(key: Key, modifiers: Modifiers, modes: Modes, options: KeyOptions) -> Option<Vec<u8>> {
    if modifiers.mac_cmd {
        return None;
    }
    if let Some(bytes) = special_key(key, modifiers, modes) {
        return Some(bytes);
    }
    if modifiers.ctrl {
        return control_byte(key).map(|byte| meta_prefix(modifiers, &[byte]));
    }
    meta_char(key, modifiers, options).map(|c| vec![ESC, c])
}

/// The character that Option and `key` send after ESC with the setting. The caller drops the
/// text that macOS composes for the same press.
pub(super) fn meta_char(key: Key, modifiers: Modifiers, options: KeyOptions) -> Option<u8> {
    if !(options.option_as_meta && modifiers.alt) || modifiers.ctrl || modifiers.mac_cmd {
        return None;
    }
    let c = match key {
        Key::Space => b' ',
        Key::Minus => b'-',
        Key::Equals => b'=',
        Key::Period => b'.',
        Key::Comma => b',',
        Key::Slash => b'/',
        Key::Backslash => b'\\',
        Key::Semicolon => b';',
        Key::Quote => b'\'',
        Key::Backtick => b'`',
        Key::OpenBracket => b'[',
        Key::CloseBracket => b']',
        _ => match key.name().as_bytes() {
            [c @ (b'A'..=b'Z' | b'0'..=b'9')] => *c,
            _ => return None,
        },
    };
    Some(if modifiers.shift { c } else { c.to_ascii_lowercase() })
}

fn special_key(key: Key, modifiers: Modifiers, modes: Modes) -> Option<Vec<u8>> {
    let app = modes.app_cursor;
    let bytes = match key {
        Key::Enter => meta_prefix(modifiers, b"\r"),
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![b'\t'],
        Key::Backspace if modifiers.ctrl => meta_prefix(modifiers, &[0x08]),
        Key::Backspace => meta_prefix(modifiers, &[0x7f]),
        Key::Escape => meta_prefix(modifiers, &[ESC]),
        Key::ArrowUp => letter_key(b'A', modifiers, app),
        Key::ArrowDown => letter_key(b'B', modifiers, app),
        Key::ArrowRight => letter_key(b'C', modifiers, app),
        Key::ArrowLeft => letter_key(b'D', modifiers, app),
        Key::Home => letter_key(b'H', modifiers, app),
        Key::End => letter_key(b'F', modifiers, app),
        // F1 to F4 use SS3 with no modifier, in each mode.
        Key::F1 => letter_key(b'P', modifiers, true),
        Key::F2 => letter_key(b'Q', modifiers, true),
        Key::F3 => letter_key(b'R', modifiers, true),
        Key::F4 => letter_key(b'S', modifiers, true),
        Key::Insert => tilde_key(2, modifiers),
        Key::Delete => tilde_key(3, modifiers),
        Key::PageUp => tilde_key(5, modifiers),
        Key::PageDown => tilde_key(6, modifiers),
        Key::F5 => tilde_key(15, modifiers),
        Key::F6 => tilde_key(17, modifiers),
        Key::F7 => tilde_key(18, modifiers),
        Key::F8 => tilde_key(19, modifiers),
        Key::F9 => tilde_key(20, modifiers),
        Key::F10 => tilde_key(21, modifiers),
        Key::F11 => tilde_key(23, modifiers),
        Key::F12 => tilde_key(24, modifiers),
        _ => return None,
    };
    Some(bytes)
}

/// Ctrl and a key give a C0 control byte.
fn control_byte(key: Key) -> Option<u8> {
    let byte = match key {
        Key::Space | Key::Num2 => 0x00,
        Key::OpenBracket | Key::Num3 => ESC,
        Key::Backslash | Key::Num4 => 0x1c,
        Key::CloseBracket | Key::Num5 => 0x1d,
        Key::Num6 => 0x1e,
        Key::Minus | Key::Slash | Key::Num7 => 0x1f,
        Key::Num8 => 0x7f,
        _ => match key.name().as_bytes() {
            [letter @ b'A'..=b'Z'] => letter - b'A' + 1,
            _ => return None,
        },
    };
    Some(byte)
}

/// The modifier parameter of xterm: 1 + Shift + 2 Alt + 4 Ctrl. 1 is no modifier.
fn modifier_parameter(modifiers: Modifiers) -> u8 {
    1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.ctrl)
}

/// `CSI letter`, `SS3 letter` in application mode, or `CSI 1 ; modifier letter`.
fn letter_key(letter: u8, modifiers: Modifiers, application: bool) -> Vec<u8> {
    match modifier_parameter(modifiers) {
        1 if application => vec![ESC, b'O', letter],
        1 => vec![ESC, b'[', letter],
        parameter => format!("\x1b[1;{parameter}{}", char::from(letter)).into_bytes(),
    }
}

/// `CSI code ~` or `CSI code ; modifier ~`.
fn tilde_key(code: u8, modifiers: Modifiers) -> Vec<u8> {
    match modifier_parameter(modifiers) {
        1 => format!("\x1b[{code}~").into_bytes(),
        parameter => format!("\x1b[{code};{parameter}~").into_bytes(),
    }
}

/// Alt sends ESC before the bytes of the key.
fn meta_prefix(modifiers: Modifiers, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    if modifiers.alt {
        out.push(ESC);
    }
    out.extend_from_slice(bytes);
    out
}

#[cfg(test)]
mod tests;
