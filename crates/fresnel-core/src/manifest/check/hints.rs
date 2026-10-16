//! The hints of a text and the marks of a bad `!!binary` value, from the events of the YAML
//! parser of serde-saphyr, each with its line and column.
//!
//! - A plain value that the parser reads as a boolean or a number where a user can mean a
//!   text: `yes`, `0644`, `1_000` (research 05 section 6.3).
//! - A plain key of that form: kubectl reads it as the boolean or the number and sends that
//!   text, the app sends the key as it is.
//! - A kind, a name or a namespace that is not the one of the object of the editor, and a
//!   namespace of an object of a kind with none.
//! - A text that is empty or not a map.
//!
//! A value with the tag `!!binary` that is not base64 is an error: the YAML type `binary`
//! is base64, and the save of a Secret sends it as it is.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle, Span, Tag};

use super::{Identity, Level, Mark};

/// The namespace of the tags of the YAML types, as `!!` names them.
const YAML_TYPES: &str = "tag:yaml.org,2002:";

/// Adds the marks of the events of a text, at most `most` of each level, and gives the count
/// of the others: a text can have a hint for each value. An error of the parser ends the walk:
/// the check has its mark from the parser of serde-saphyr.
pub(super) fn add(text: &str, identity: &Identity, marks: &mut Vec<Mark>, most: usize) -> usize {
    let mut walk = Walk { identity, stack: Vec::new(), root: Root::Before, marks, room: (most, most), dropped: 0 };
    for event in Parser::new_from_str(text) {
        let Ok((event, span)) = event else {
            return walk.dropped;
        };
        walk.event(event, span);
    }
    if walk.root == Root::Before {
        walk.mark(1, 1, Level::Hint, "The text is empty. The text of an object is a map with its kind and metadata.");
    }
    walk.dropped
}

/// Where the walk is in the first document.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Root {
    /// Before its first node.
    Before,
    /// In its root node.
    In,
}

/// An open map or list of the walk.
enum Frame {
    /// A map: the key of the value that comes now, or `None` while a key comes.
    Map(Option<String>),
    List,
}

struct Walk<'a> {
    identity: &'a Identity,
    stack: Vec<Frame>,
    root: Root,
    marks: &'a mut Vec<Mark>,
    /// The count of errors and of hints that `marks` can still take.
    room: (usize, usize),
    /// The count of marks past that room.
    dropped: usize,
}

impl Walk<'_> {
    fn event(&mut self, event: Event<'_>, span: Span) {
        match event {
            Event::Scalar(value, style, _, tag) => {
                self.root(&span, false);
                self.scalar(&value, style, tag.as_deref(), &span);
            }
            Event::MappingStart(..) => {
                self.root(&span, true);
                self.stack.push(Frame::Map(None));
            }
            Event::SequenceStart(..) => {
                self.root(&span, false);
                self.stack.push(Frame::List);
            }
            Event::MappingEnd | Event::SequenceEnd => {
                self.stack.pop();
                self.node_ended(None);
            }
            Event::Alias(_) => {
                self.root(&span, false);
                self.node_ended(None);
            }
            _ => {}
        }
    }

    /// Marks a first node that is not a map: the text of an object is one.
    fn root(&mut self, span: &Span, map: bool) {
        if self.root == Root::Before {
            self.root = Root::In;
            if !map {
                let (line, column) = place(span);
                self.mark(line, column, Level::Hint, "The text of an object is a map with its kind and metadata.");
            }
        }
    }

    fn scalar(&mut self, value: &str, style: ScalarStyle, tag: Option<&Tag>, span: &Span) {
        let (line, column) = place(span);
        let is_key = matches!(self.stack.last(), Some(Frame::Map(None)));
        if tag.is_some_and(is_binary) && STANDARD.decode(value.split_whitespace().collect::<String>()).is_err() {
            self.mark(line, column, Level::Error, "The value with the tag !!binary is not base64.");
        }
        if style == ScalarStyle::Plain
            && tag.is_none()
            && let Some(read) = Read::of(value)
        {
            let text = if is_key {
                format!("kubectl reads the key {value} as {}. Quote it, so that each tool reads {value}.", read.text())
            } else {
                format!("{value} is {} here, not a text. Quote it for a text.", read.words())
            };
            self.mark(line, column, Level::Hint, &text);
        }
        if !is_key {
            self.identity(value, line, column);
        }
        self.node_ended(is_key.then(|| value.to_owned()));
    }

    /// Marks a kind, a name or a namespace that differs from the object of the editor.
    fn identity(&mut self, value: &str, line: usize, column: usize) {
        let Identity { kind, key } = self.identity;
        let wanted = if self.at(&["kind"]) {
            Some(("kind", kind.info().name))
        } else if self.at(&["metadata", "name"]) {
            Some(("name", &*key.name))
        } else if self.at(&["metadata", "namespace"]) {
            Some(("namespace", &*key.namespace))
        } else {
            None
        };
        if let Some((field, wanted)) = wanted.filter(|(_, wanted)| *wanted != value) {
            let info = kind.info();
            // The server drops the namespace of an object of a kind with none.
            let text = if field == "namespace" && !info.namespaced {
                format!("A {} has no namespace: the server drops this one.", info.name)
            } else {
                format!("The editor edits the object with the {field} {wanted}. The server refuses an other {field}.")
            };
            self.mark(line, column, Level::Hint, &text);
        }
    }

    /// True when the walk is at the value of this path of keys from the root map.
    fn at(&self, path: &[&str]) -> bool {
        self.stack.len() == path.len()
            && self.stack.iter().zip(path).all(|(frame, key)| matches!(frame, Frame::Map(Some(at)) if at == key))
    }

    /// A node ended: a key, given as its text, or a value.
    fn node_ended(&mut self, key: Option<String>) {
        if let Some(Frame::Map(current)) = self.stack.last_mut() {
            *current = match current {
                // A key that is a map or a list has no text.
                None => Some(key.unwrap_or_default()),
                Some(_) => None,
            };
        }
    }

    fn mark(&mut self, line: usize, column: usize, level: Level, text: &str) {
        let room = match level {
            Level::Error => &mut self.room.0,
            Level::Hint => &mut self.room.1,
        };
        if *room == 0 {
            self.dropped += 1;
            return;
        }
        *room -= 1;
        self.marks.push(Mark { line, column, level, text: text.to_owned() });
    }
}

/// The line and the column of the start of a span, from 1.
fn place(span: &Span) -> (usize, usize) {
    (span.start.line().max(1), span.start.col() + 1)
}

/// True for the tag of the YAML type `binary`: `!!binary`, or `!binary` as serde-saphyr reads it.
fn is_binary(tag: &Tag) -> bool {
    tag.suffix_in_namespace(YAML_TYPES).is_some_and(|name| name == "binary") || tag.parts() == ("!", "binary")
}

/// What the parser reads from a plain text that a user can mean as a text.
#[derive(Debug, PartialEq, Eq)]
enum Read {
    Boolean(bool),
    /// A number with a leading zero: octal.
    Octal(i64),
    /// A number with `_` between its digits.
    Spaced(i64),
}

impl Read {
    /// The reading of a plain text, as the parser and kubectl read it: the words of YAML 1.1
    /// for a boolean, a leading zero for octal, and `_` in a number. `None` for each text that
    /// reads as written.
    fn of(text: &str) -> Option<Self> {
        let word = ["y", "yes", "on", "n", "no", "off"].iter().position(|word| text.eq_ignore_ascii_case(word));
        if let Some(index) = word {
            return Some(Self::Boolean(index < 3));
        }
        let (sign, digits) = match text.strip_prefix('-') {
            Some(rest) => (-1, rest),
            None => (1, text.strip_prefix('+').unwrap_or(text)),
        };
        let first = digits.as_bytes().first().copied()?;
        if !first.is_ascii_digit() || !digits.bytes().all(|byte| byte.is_ascii_digit() || byte == b'_') {
            return None;
        }
        if digits.len() > 1 && first == b'0' && digits.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
            return i64::from_str_radix(digits, 8).ok().map(|value| Self::Octal(sign * value));
        }
        let spaced = digits.contains('_') && !digits.ends_with('_');
        let value = spaced.then(|| digits.replace('_', "").parse::<i64>().ok()).flatten();
        value.map(|value| Self::Spaced(sign * value))
    }

    /// "the boolean true", "the octal number 420", "the number 1000".
    fn words(&self) -> String {
        match self {
            Self::Boolean(value) => format!("the boolean {value}"),
            Self::Octal(value) => format!("the octal number {value}"),
            Self::Spaced(value) => format!("the number {value}"),
        }
    }

    /// The text that kubectl makes of such a key: "true", "420", "1000".
    fn text(&self) -> String {
        match self {
            Self::Boolean(value) => value.to_string(),
            Self::Octal(value) | Self::Spaced(value) => value.to_string(),
        }
    }
}
