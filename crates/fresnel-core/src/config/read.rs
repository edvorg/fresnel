//! The reader of the text of `config.toml`. The TOML parser gives the place of each key and
//! value in the text, so each problem has its line and column.

use std::ops::Range;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::{Config, Loaded, Problem, VERSION};
use crate::commands::{self, CommandId, Keys, Shortcut};

/// Reads the settings of a text. A key that does not fit is a problem, and the other keys
/// stay in use.
pub fn parse(text: &str) -> Loaded {
    let table = match DeTable::parse(text) {
        Ok(table) => table,
        Err(error) => {
            let problem = at(text, error.span().unwrap_or_default(), error.message().to_owned());
            return Loaded { config: Config::default(), problems: vec![problem], used: false };
        }
    };
    let mut reader = Reader { text, problems: Vec::new() };
    let (mut version, mut choices) = (None, Vec::new());
    for (key, value) in table.get_ref().iter() {
        match key.get_ref().as_ref() {
            "version" => version = Some(reader.version(value)),
            "shortcuts" => choices = reader.shortcuts(value),
            other => reader.problem(key.span(), format!("unknown key `{other}`; the app ignores it")),
        }
    }
    // A version that is not a number has its own problem, and the app reads the file as
    // the version of this app.
    match version {
        Some(Some(VERSION) | None) => {}
        Some(Some(found)) => {
            let text = format!("the file has the version {found}, and this app reads the version {VERSION} only");
            reader.problem(0..0, text);
            return Loaded { config: Config::default(), problems: reader.problems, used: false };
        }
        None => reader.problem(0..0, format!("the file has no `version`; the app reads it as version {VERSION}")),
    }
    // A conflict of two keys is a problem at the second one, in the sequence of the file.
    choices.sort_by_key(|(_, _, span): &(CommandId, Option<Shortcut>, Range<usize>)| span.start);
    let (keys, conflicts) = Keys::with_choices(&choices);
    for (span, text) in conflicts {
        reader.problem(span, text);
    }
    reader.problems.sort_by_key(|problem| (problem.line, problem.column));
    Loaded { config: Config { keys }, problems: reader.problems, used: true }
}

struct Reader<'a> {
    text: &'a str,
    problems: Vec<Problem>,
}

impl Reader<'_> {
    fn problem(&mut self, span: Range<usize>, text: String) {
        self.problems.push(at(self.text, span, text));
    }

    /// The version. `None` for a value that is no whole number.
    fn version(&mut self, value: &Spanned<DeValue<'_>>) -> Option<i64> {
        let number = value.get_ref().as_integer().and_then(|number| {
            let digits: String = number.as_str().chars().filter(|char| *char != '_').collect();
            i64::from_str_radix(&digits, number.radix()).ok()
        });
        if number.is_none() {
            self.problem(value.span(), format!("`version` is a whole number, for example {VERSION}"));
        }
        number
    }

    /// The table `[shortcuts]`: the identifier of a command, and its key as text. An empty
    /// text takes the key from the command.
    fn shortcuts(&mut self, value: &Spanned<DeValue<'_>>) -> Vec<(CommandId, Option<Shortcut>, Range<usize>)> {
        let Some(table) = value.get_ref().as_table() else {
            self.problem(value.span(), "`shortcuts` is a table: `[shortcuts]`".to_owned());
            return Vec::new();
        };
        let mut choices = Vec::new();
        for (key, value) in table.iter() {
            let name = key.get_ref().as_ref();
            let Some(entry) = commands::named(name) else {
                self.problem(key.span(), format!("unknown command `{name}` in `shortcuts`; the app ignores it"));
                continue;
            };
            let Some(text) = value.get_ref().as_str() else {
                self.problem(value.span(), format!("the key of `{name}` is a text, for example \"Cmd+K\""));
                continue;
            };
            match text.trim() {
                "" => choices.push((entry.id, None, key.span())),
                text => match Shortcut::parse(text) {
                    Ok(shortcut) => choices.push((entry.id, Some(shortcut), key.span())),
                    Err(error) => self.problem(value.span(), format!("the key of `{name}`: {error}")),
                },
            }
        }
        choices
    }
}

/// The problem at the start of `span`, with its line and its column from 1.
fn at(text: &str, span: Range<usize>, problem: String) -> Problem {
    let before = text.get(..span.start).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |start| start.chars().count()) + 1;
    Problem { line, column, text: problem }
}
