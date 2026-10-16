//! The check of an edited text (plan section 12.9): the error of the YAML parser with its
//! line and column, and hints.
//!
//! Only the YAML parser and the server can refuse a save. A hint tells how the text reads
//! where that can surprise, and never refuses: valid YAML gets no error. The parser reads a
//! number with a leading zero as octal and `yes` as a boolean, as kubectl does (research 05
//! section 6.3). A check of 8,000 lines takes some milliseconds, so a view runs it on a
//! worker thread.
//!
//! - `hints`: the hints and the marks of a bad `!!binary` value, from the events of the parser.

mod hints;

use serde_json::Value;
use serde_saphyr::{MessageFormatter as _, UserMessageFormatter};

use crate::kinds::Kind;
use crate::store::ObjectKey;

/// The most marks of one check. A text can have a hint on each line.
const MOST: usize = 500;

/// The object that a text must stay: a save replaces it, and the server refuses a text of an
/// other kind, name or namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub kind: Kind,
    pub key: ObjectKey,
}

/// What a mark tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// The parser refuses the text.
    Error,
    /// The text is valid; it reads in a way that can surprise.
    Hint,
}

/// One mark at a place of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    /// The line, from 1.
    pub line: usize,
    /// The column in characters, from 1.
    pub column: usize,
    pub level: Level,
    pub text: String,
}

/// The marks of one text, in the sequence of their places.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    pub marks: Vec<Mark>,
    /// The count of marks past the limit of one check.
    pub more: usize,
}

impl Check {
    /// True when the parser refuses the text.
    pub fn has_error(&self) -> bool {
        self.marks.iter().any(|mark| mark.level == Level::Error)
    }
}

/// Checks the text of the editor of an object.
pub fn check(text: &str, identity: &Identity) -> Check {
    let mut marks = Vec::new();
    if let Err(error) = parse(text) {
        let (line, column) = error.location().map_or((1, 1), |place| (place.line(), place.column()));
        let (line, column) = (usize::try_from(line).unwrap_or(1), usize::try_from(column).unwrap_or(1));
        let text = sentence(&UserMessageFormatter.format_message(&error));
        marks.push(Mark { line, column, level: Level::Error, text });
    }
    hints::add(text, identity, &mut marks);
    marks.sort_by_key(|mark| (mark.line, mark.column, mark.level));
    let more = marks.len().saturating_sub(MOST);
    marks.truncate(MOST);
    Check { marks, more }
}

/// Reads a text as the save will read it: with the scalar rules of kubectl, and a value with
/// the tag `!!binary` as its base64, which the save of a Secret sends as it is.
fn parse(text: &str) -> Result<Value, serde_saphyr::Error> {
    let options = serde_saphyr::options! { legacy_octal_numbers: true, ignore_binary_tag_for_string: true };
    serde_saphyr::from_str_with_options(text, options)
}

/// A message of the parser as a sentence: "Duplicate mapping key: a not allowed here."
fn sentence(message: &str) -> String {
    let mut characters = message.trim().chars();
    let first = characters.next().map(|first| first.to_uppercase().collect::<String>()).unwrap_or_default();
    let rest = characters.as_str();
    if rest.ends_with('.') { format!("{first}{rest}") } else { format!("{first}{rest}.") }
}

#[cfg(test)]
mod tests;
