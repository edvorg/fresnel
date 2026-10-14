//! A key with its modifiers, and its text in the settings: `Cmd+Shift+K`, `Cmd+[`, `F5`.
//!
//! The core has no GUI toolkit, so a key is a name of the list `NAMES`. Each name is a name
//! that egui reads with `Key::from_name`, and a test of `fresnel-ui` holds that rule.

use std::fmt::{self, Display};

/// The keys that a shortcut can use, by the name in the text. A letter can be small in the text.
const NAMES: &[&str] = &[
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "Up",
    "Down",
    "Left",
    "Right",
    "Enter",
    "Escape",
    "Tab",
    "Space",
    "Backspace",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Comma",
    "Period",
    "Slash",
    "Semicolon",
    "Quote",
    "OpenBracket",
    "CloseBracket",
    "Backslash",
    "Minus",
    "Equals",
    "Backtick",
];
/// The keys with a symbol, which the text can use in place of the name.
const SYMBOLS: &[(&str, &str)] = &[
    ("Comma", ","),
    ("Period", "."),
    ("Slash", "/"),
    ("Semicolon", ";"),
    ("Quote", "'"),
    ("OpenBracket", "["),
    ("CloseBracket", "]"),
    ("Backslash", "\\"),
    ("Minus", "-"),
    ("Equals", "="),
    ("Backtick", "`"),
];

/// One key of the list `NAMES`, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(&'static str);

impl Key {
    /// The key with this name or symbol. A letter can be small. `None` for a key that is
    /// not in the list.
    pub fn named(text: &str) -> Option<Self> {
        let by_symbol = SYMBOLS.iter().find(|(_, symbol)| *symbol == text).map(|(name, _)| *name);
        by_symbol.or_else(|| NAMES.iter().copied().find(|name| name.eq_ignore_ascii_case(text))).map(Self)
    }

    /// Each key that a shortcut can use.
    pub fn all() -> impl Iterator<Item = Self> {
        NAMES.iter().copied().map(Self)
    }

    /// The name, which egui reads with `Key::from_name`.
    pub fn name(self) -> &'static str {
        self.0
    }

    /// The text of the key in a shortcut: its symbol, or else its name.
    fn text(self) -> &'static str {
        SYMBOLS.iter().find(|(name, _)| *name == self.0).map_or(self.0, |(_, symbol)| symbol)
    }

    /// True for F1 to F12: such a key can be a shortcut with no modifier, because no text
    /// field takes it.
    fn is_function(self) -> bool {
        self.0
            .strip_prefix('F')
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
    }
}

/// A key with its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Shortcut {
    /// Command on macOS and Control on the other systems.
    pub command: bool,
    /// The Control key of macOS. On the other systems, it is the same key as `command`.
    pub ctrl: bool,
    /// Option on macOS, Alt on the other systems.
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

impl Shortcut {
    /// Command and a key: the default keys of the command table.
    pub(super) const fn command(key: &'static str) -> Self {
        Self { command: true, ctrl: false, alt: false, shift: false, key: Key(key) }
    }

    /// The same shortcut with Shift.
    pub(super) const fn shift(self) -> Self {
        Self { shift: true, ..self }
    }

    /// Reads the text of a shortcut, for example `Cmd+Shift+K`. The modifiers come first,
    /// in any sequence, and each part can be in small letters.
    ///
    /// A shortcut needs Command, Control or Option, except a function key: the text of a
    /// field arrives as text, so a plain key would act while the user types into a field.
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let Some((last, modifiers)) = parts.split_last() else {
            return Err("no key".to_owned());
        };
        let key = Key::named(last).ok_or_else(|| format!("`{last}` is not a key that a shortcut can use"))?;
        let mut shortcut = Self { command: false, ctrl: false, alt: false, shift: false, key };
        for modifier in modifiers {
            let flag = match modifier.to_ascii_lowercase().as_str() {
                "cmd" | "command" => &mut shortcut.command,
                "ctrl" | "control" => &mut shortcut.ctrl,
                "alt" | "option" | "opt" => &mut shortcut.alt,
                "shift" => &mut shortcut.shift,
                _ => return Err(format!("`{modifier}` is not a modifier: use Cmd, Ctrl, Alt or Shift")),
            };
            if *flag {
                return Err(format!("`{modifier}` comes two times"));
            }
            *flag = true;
        }
        if !(shortcut.command || shortcut.ctrl || shortcut.alt || key.is_function()) {
            return Err("a shortcut needs Cmd, Ctrl or Alt, so that it does not act on the text of a field".to_owned());
        }
        Ok(shortcut)
    }
}

impl Display for Shortcut {
    /// The text that [`Shortcut::parse`] reads.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let modifiers = [(self.command, "Cmd+"), (self.ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+")];
        for (_, text) in modifiers.iter().filter(|(on, _)| *on) {
            f.write_str(text)?;
        }
        f.write_str(self.key.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shortcut_reads_its_text_and_writes_it_again() {
        for text in ["Cmd+K", "Cmd+Shift+K", "Cmd+[", "Ctrl+Alt+Delete", "F5", "Alt+Up", "Cmd+Shift+Backspace"] {
            let shortcut = Shortcut::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"));
            assert_eq!(shortcut.to_string(), text);
        }
        let spoken = Shortcut::parse(" command + shift + k ").expect("small letters and spaces");
        assert_eq!(spoken, Shortcut::command("K").shift());
        assert_eq!(Shortcut::parse("Cmd+OpenBracket").map(|shortcut| shortcut.to_string()), Ok("Cmd+[".to_owned()));
        assert_eq!(Shortcut::parse("Option+Cmd+P").map(|shortcut| shortcut.to_string()), Ok("Cmd+Alt+P".to_owned()));
    }

    #[test]
    fn a_shortcut_with_no_key_a_bad_part_or_no_modifier_is_an_error_with_its_reason() {
        let error = |text: &str| Shortcut::parse(text).expect_err(text);
        assert_eq!(error("Cmd+"), "`` is not a key that a shortcut can use");
        assert_eq!(error("Cmd+Hyper+K"), "`Hyper` is not a modifier: use Cmd, Ctrl, Alt or Shift");
        assert_eq!(error("Cmd+cmd+K"), "`cmd` comes two times");
        assert_eq!(error("Cmd+F13"), "`F13` is not a key that a shortcut can use");
        assert!(error("K").starts_with("a shortcut needs Cmd"));
        assert!(error("Shift+/").starts_with("a shortcut needs Cmd"));
        assert_eq!(Shortcut::parse("F12").map(|shortcut| shortcut.key.name()), Ok("F12"));
    }

    #[test]
    fn each_key_has_one_name_and_a_symbol_names_its_key() {
        let names: Vec<&str> = Key::all().map(Key::name).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
        assert_eq!(Key::named("]").map(Key::name), Some("CloseBracket"));
        assert_eq!(Key::named("k").map(Key::name), Some("K"));
        assert_eq!(Key::named(""), None, "an empty symbol names no key");
        assert!(Key::named("F1").is_some_and(Key::is_function) && !Key::named("F").is_some_and(Key::is_function));
    }
}
