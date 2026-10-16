//! One table of key presses and the bytes that xterm sends for them: the 67 cases of the
//! spike.

use super::*;

const NONE: Modifiers = Modifiers::NONE;
const SHIFT: Modifiers = Modifiers::SHIFT;
const CTRL: Modifiers = Modifiers::CTRL;
const ALT: Modifiers = Modifiers::ALT;
const META: KeyOptions = KeyOptions { option_as_meta: true };
const NO_META: KeyOptions = KeyOptions { option_as_meta: false };

type Case = (&'static str, Key, Modifiers, Option<&'static [u8]>);

/// The keys that act the same in each mode.
const CASES: &[Case] = &[
    ("Enter", Key::Enter, NONE, Some(b"\r")),
    ("Tab", Key::Tab, NONE, Some(b"\t")),
    ("Shift+Tab", Key::Tab, SHIFT, Some(b"\x1b[Z")),
    ("Backspace", Key::Backspace, NONE, Some(b"\x7f")),
    ("Ctrl+Backspace", Key::Backspace, CTRL, Some(b"\x08")),
    ("Alt+Backspace", Key::Backspace, ALT, Some(b"\x1b\x7f")),
    ("Escape", Key::Escape, NONE, Some(b"\x1b")),
    ("Insert", Key::Insert, NONE, Some(b"\x1b[2~")),
    ("Delete", Key::Delete, NONE, Some(b"\x1b[3~")),
    ("PageUp", Key::PageUp, NONE, Some(b"\x1b[5~")),
    ("PageDown", Key::PageDown, NONE, Some(b"\x1b[6~")),
    ("Ctrl+Delete", Key::Delete, CTRL, Some(b"\x1b[3;5~")),
    // Arrows with a modifier use the CSI form in both cursor modes.
    ("Shift+Up", Key::ArrowUp, SHIFT, Some(b"\x1b[1;2A")),
    ("Alt+Left", Key::ArrowLeft, ALT, Some(b"\x1b[1;3D")),
    ("Ctrl+Right", Key::ArrowRight, CTRL, Some(b"\x1b[1;5C")),
    ("Ctrl+Shift+End", Key::End, Modifiers { shift: true, ctrl: true, ..NONE }, Some(b"\x1b[1;6F")),
    // Ctrl keys give C0 control bytes.
    ("Ctrl+A", Key::A, CTRL, Some(b"\x01")),
    ("Ctrl+C", Key::C, CTRL, Some(b"\x03")),
    ("Ctrl+D", Key::D, CTRL, Some(b"\x04")),
    ("Ctrl+L", Key::L, CTRL, Some(b"\x0c")),
    ("Ctrl+Z", Key::Z, CTRL, Some(b"\x1a")),
    ("Ctrl+Space", Key::Space, CTRL, Some(b"\x00")),
    ("Ctrl+[", Key::OpenBracket, CTRL, Some(b"\x1b")),
    ("Ctrl+\\", Key::Backslash, CTRL, Some(b"\x1c")),
    ("Ctrl+]", Key::CloseBracket, CTRL, Some(b"\x1d")),
    ("Ctrl+-", Key::Minus, CTRL, Some(b"\x1f")),
    ("Ctrl+Alt+C", Key::C, Modifiers { ctrl: true, alt: true, ..NONE }, Some(b"\x1b\x03")),
    ("Ctrl+F13 has no bytes", Key::F13, CTRL, None),
    // Function keys.
    ("F1", Key::F1, NONE, Some(b"\x1bOP")),
    ("F2", Key::F2, NONE, Some(b"\x1bOQ")),
    ("F3", Key::F3, NONE, Some(b"\x1bOR")),
    ("F4", Key::F4, NONE, Some(b"\x1bOS")),
    ("F5", Key::F5, NONE, Some(b"\x1b[15~")),
    ("F6", Key::F6, NONE, Some(b"\x1b[17~")),
    ("F7", Key::F7, NONE, Some(b"\x1b[18~")),
    ("F8", Key::F8, NONE, Some(b"\x1b[19~")),
    ("F9", Key::F9, NONE, Some(b"\x1b[20~")),
    ("F10", Key::F10, NONE, Some(b"\x1b[21~")),
    ("F11", Key::F11, NONE, Some(b"\x1b[23~")),
    ("F12", Key::F12, NONE, Some(b"\x1b[24~")),
    ("Shift+F1", Key::F1, SHIFT, Some(b"\x1b[1;2P")),
    ("Shift+F5", Key::F5, SHIFT, Some(b"\x1b[15;2~")),
    ("Ctrl+F12", Key::F12, CTRL, Some(b"\x1b[24;5~")),
    // A key that gives text: the text event carries it.
    ("A", Key::A, NONE, None),
    ("Shift+A", Key::A, SHIFT, None),
    ("Space", Key::Space, NONE, None),
    // A key with Cmd belongs to the app.
    ("Cmd+C", Key::C, Modifiers::MAC_CMD, None),
    ("Cmd+Left", Key::ArrowLeft, Modifiers::MAC_CMD, None),
];

/// The cursor keys: the bytes in the normal mode and in the application cursor mode.
const CURSOR_KEYS: &[(&str, Key, &[u8], &[u8])] = &[
    ("Up", Key::ArrowUp, b"\x1b[A", b"\x1bOA"),
    ("Down", Key::ArrowDown, b"\x1b[B", b"\x1bOB"),
    ("Right", Key::ArrowRight, b"\x1b[C", b"\x1bOC"),
    ("Left", Key::ArrowLeft, b"\x1b[D", b"\x1bOD"),
    ("Home", Key::Home, b"\x1b[H", b"\x1bOH"),
    ("End", Key::End, b"\x1b[F", b"\x1bOF"),
];

/// Option as Meta: the bytes with the setting on.
const META_KEYS: &[(&str, Key, Modifiers, &[u8])] = &[
    ("Alt+B", Key::B, ALT, b"\x1bb"),
    ("Alt+F", Key::F, ALT, b"\x1bf"),
    ("Alt+Shift+F", Key::F, Modifiers { alt: true, shift: true, ..NONE }, b"\x1bF"),
    ("Alt+.", Key::Period, ALT, b"\x1b."),
    ("Alt+1", Key::Num1, ALT, b"\x1b1"),
    ("Alt+Enter", Key::Enter, ALT, b"\x1b\r"),
];

#[test]
fn each_key_gives_the_bytes_of_xterm() {
    let normal = Modes::default();
    let application = Modes { app_cursor: true, ..Modes::default() };
    let mut failed = Vec::new();
    let mut cases = 0;
    let mut check = |name: String, got: Option<Vec<u8>>, want: Option<&[u8]>| {
        cases += 1;
        if got.as_deref() != want {
            failed.push(format!("{name}: {got:?}, not {want:?}"));
        }
    };
    for (name, key, modifiers, want) in CASES {
        check((*name).to_owned(), encode(*key, *modifiers, normal, NO_META), *want);
    }
    for (name, key, normal_bytes, application_bytes) in CURSOR_KEYS {
        check((*name).to_owned(), encode(*key, NONE, normal, NO_META), Some(normal_bytes));
        check(format!("{name}, application"), encode(*key, NONE, application, NO_META), Some(application_bytes));
    }
    for (name, key, modifiers, want) in META_KEYS {
        check(format!("{name} as Meta"), encode(*key, *modifiers, normal, META), Some(want));
    }
    // With no setting, Option and a letter give the character that macOS composes.
    check("Alt+B with no Meta".to_owned(), encode(Key::B, ALT, normal, NO_META), None);
    assert_eq!(failed, Vec::<String>::new());
    assert_eq!(cases, 67);
}
