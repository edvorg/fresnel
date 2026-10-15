//! The reader of the text of `config.toml`. The TOML parser gives the place of each key and
//! value in the text, so each problem has its line and column.
//!
//! - `fixed`: the groups of the settings table, with the bounds of each number.
//! - `clusters`: the options of each cluster and the cluster groups.

pub(super) mod clusters;
pub(super) mod fixed;

use std::collections::BTreeMap;
use std::ops::Range;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::table::Group;
use super::{Config, Loaded, Problem, VERSION};
use crate::commands::{self, CommandId, Keys, Shortcut};

/// The most problems of a file that go to the journal, and one line for the rest. A file of
/// unknown keys would else push each other entry out of the journal.
const MAX_PROBLEMS: usize = 20;

/// Reads the settings of a text. A key that does not fit is a problem, and the other keys
/// stay in use.
pub fn parse(text: &str) -> Loaded {
    let mut loaded = parse_all(text);
    loaded.problems = limited(loaded.problems);
    loaded
}

/// The same with each problem: a save compares the problems of the old and the new text,
/// and the line of the count of the others would change with each one.
pub(super) fn parse_all(text: &str) -> Loaded {
    let table = match DeTable::parse(text) {
        Ok(table) => table,
        Err(error) => {
            let problem = at(text, error.span().unwrap_or_default(), error.message().to_owned());
            return Loaded { config: Config::default(), problems: vec![problem], used: false };
        }
    };
    let mut reader = Reader { text, problems: Vec::new() };
    let (mut version, mut choices, mut config) = (None, Vec::new(), Config::default());
    let mut values = BTreeMap::new();
    for (key, value) in table.get_ref().iter() {
        match key.get_ref().as_ref() {
            "version" => version = Some(reader.version(value)),
            "shortcuts" => choices = reader.shortcuts(value),
            "clusters" => config.clusters = reader.clusters(value),
            "groups" => config.groups = reader.groups(value),
            other => match Group::named(other) {
                Some(group) => reader.fixed(group, value, &mut values),
                None => reader.problem(key.span(), format!("unknown key `{other}`; the app ignores it")),
            },
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
    config.keys = keys;
    config.values = values;
    Loaded { config, problems: reader.problems, used: true }
}

/// The first [`MAX_PROBLEMS`] problems, and the count of the others at the place of the
/// first one of them.
fn limited(mut problems: Vec<Problem>) -> Vec<Problem> {
    if let Some(first) = problems.get(MAX_PROBLEMS) {
        let (line, column) = (first.line, first.column);
        let more = problems.len() - MAX_PROBLEMS;
        problems.truncate(MAX_PROBLEMS);
        let text = format!("{more} more problems from here; the journal shows the first {MAX_PROBLEMS} only");
        problems.push(Problem { line, column, text });
    }
    problems
}

struct Reader<'a> {
    text: &'a str,
    problems: Vec<Problem>,
}

impl Reader<'_> {
    fn problem(&mut self, span: Range<usize>, text: String) {
        self.problems.push(at(self.text, span, text));
    }

    /// The table of a key, or a problem for a value that is no table.
    fn table_of<'v, 'i>(&mut self, path: &str, value: &'v Spanned<DeValue<'i>>) -> Option<&'v DeTable<'i>> {
        let table = value.get_ref().as_table();
        if table.is_none() {
            self.problem(value.span(), format!("`{path}` is a table: [{path}]"));
        }
        table
    }

    /// A text of a key, or a problem.
    fn text<'v>(&mut self, path: &str, value: &'v Spanned<DeValue<'_>>) -> Option<&'v str> {
        let text = value.get_ref().as_str();
        if text.is_none() {
            self.problem(value.span(), format!("`{path}` is a text in quotes"));
        }
        text
    }

    /// A list of texts, or a problem for a value that is no such list.
    fn texts(&mut self, path: &str, value: &Spanned<DeValue<'_>>, limit: usize) -> Option<Vec<String>> {
        let items = value.get_ref().as_array().and_then(|items| {
            items.iter().map(|item| item.get_ref().as_str().map(str::to_owned)).collect::<Option<Vec<String>>>()
        });
        match items {
            Some(items) if items.len() <= limit => Some(items),
            Some(items) => {
                self.problem(
                    value.span(),
                    format!("`{path}` has {} entries; the app uses the first {limit}", items.len()),
                );
                Some(items.into_iter().take(limit).collect())
            }
            None => {
                self.problem(value.span(), format!("`{path}` is a list of texts, for example [\"a\", \"b\"]"));
                None
            }
        }
    }

    /// A flag of a key, or a problem.
    fn flag(&mut self, path: &str, value: &Spanned<DeValue<'_>>) -> Option<bool> {
        let flag = value.get_ref().as_bool();
        if flag.is_none() {
            self.problem(value.span(), format!("`{path}` is true or false"));
        }
        flag
    }

    /// The version. `None` for a value that is no whole number.
    fn version(&mut self, value: &Spanned<DeValue<'_>>) -> Option<i64> {
        let number = integer(value).and_then(|number| i64::try_from(number).ok());
        if number.is_none() {
            self.problem(value.span(), format!("`version` is a whole number, for example {VERSION}"));
        }
        number
    }

    /// The table `[shortcuts]`: the identifier of a command, and its key as text. An empty
    /// text takes the key from the command.
    fn shortcuts(&mut self, value: &Spanned<DeValue<'_>>) -> Vec<(CommandId, Option<Shortcut>, Range<usize>)> {
        let Some(table) = self.table_of("shortcuts", value) else {
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

/// A whole number of the file. `None` for a value that is no integer.
fn integer(value: &Spanned<DeValue<'_>>) -> Option<i128> {
    value.get_ref().as_integer().and_then(|number| {
        let digits: String = number.as_str().chars().filter(|char| *char != '_').collect();
        i128::from_str_radix(&digits, number.radix()).ok()
    })
}

/// The problem at the start of `span`, with its line and its column from 1.
fn at(text: &str, span: Range<usize>, problem: String) -> Problem {
    let before = text.get(..span.start).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |start| start.chars().count()) + 1;
    Problem { line, column, text: problem }
}
