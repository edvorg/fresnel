//! The changes of the settings view in the text of `config.toml`, with `toml_edit`.
//!
//! A change sets or removes one key and keeps each other byte of the file: its comments,
//! its blank lines and the sequence of its keys. A value keeps the comment after it. The
//! app never writes over a file that it cannot read: a parse error, a file of an other
//! version or a change that the reader would refuse stops the write with the reason.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value as TomlValue};

use super::read::parse_all;
use super::{Loaded, VERSION};

/// The new value of one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditValue {
    Number(i64),
    Flag(bool),
    Text(String),
    List(Vec<String>),
    /// The key goes: the setting gets its default.
    Remove,
}

/// One change: the path of a key and its new value. `["network", "keepalive_secs"]`,
/// `["clusters", "ams-prod", "alias"]`, `["shortcuts", "back"]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub path: Vec<String>,
    pub value: EditValue,
}

impl Edit {
    pub fn new(path: &[&str], value: EditValue) -> Self {
        Self { path: path.iter().map(|part| (*part).to_owned()).collect(), value }
    }
}

/// The text with the changes. Fails with the reason when the text has a problem that keeps
/// the app from its settings, or when the new text has a problem that the old one has not:
/// the reader would refuse, clamp or move a value of the change, so the file would not say
/// what the app uses.
pub fn apply(text: &str, edits: &[Edit]) -> Result<String, String> {
    let before = parse_all(text);
    refusal(&before, "the file")?;
    let mut document: DocumentMut = text.parse().map_err(|error| format!("the file does not parse: {error}"))?;
    if !document.contains_key("version") {
        document.insert("version", Item::Value(TomlValue::from(VERSION)));
    }
    for edit in edits {
        change(document.as_table_mut(), edit)?;
    }
    let changed = document.to_string();
    let after = parse_all(&changed);
    refusal(&after, "the change")?;
    match after.problems.iter().find(|new| before.problems.iter().all(|old| old.text != new.text)) {
        Some(problem) => {
            Err(format!("the change has a problem at {problem}; the app writes no value that it does not use"))
        }
        None => Ok(changed),
    }
}

/// Why a write must not go on, for a text that the app does not use.
fn refusal(loaded: &Loaded, what: &str) -> Result<(), String> {
    match (loaded.used, loaded.problems.first()) {
        (false, Some(problem)) => {
            Err(format!("{what} has a problem at {problem}; the app writes no file that it cannot read"))
        }
        _ => Ok(()),
    }
}

fn change(root: &mut Table, edit: &Edit) -> Result<(), String> {
    let path = edit.path.join(".");
    let Some((key, tables)) = edit.path.split_last() else {
        return Err("a change with no key".to_owned());
    };
    let mut table: &mut dyn TableLike = root;
    for (depth, name) in tables.iter().enumerate() {
        if table.get(name).is_none() {
            if edit.value == EditValue::Remove {
                return Ok(());
            }
            let mut new = Table::new();
            // A table that only holds tables has no header of its own: `[clusters."a"]`
            // with no empty `[clusters]` before it.
            new.set_implicit(depth + 2 < edit.path.len());
            table.insert(name, Item::Table(new));
        }
        table = table
            .get_mut(name)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| format!("`{path}`: `{name}` is no table in the file"))?;
    }
    let value = match &edit.value {
        EditValue::Remove => {
            table.remove(key);
            return Ok(());
        }
        EditValue::Number(number) => TomlValue::from(*number),
        EditValue::Flag(flag) => TomlValue::from(*flag),
        EditValue::Text(text) => TomlValue::from(text.as_str()),
        EditValue::List(items) => TomlValue::Array(items.iter().map(String::as_str).collect::<Array>()),
    };
    match table.get_mut(key) {
        // The old value gives its spaces and its comment to the new one.
        Some(Item::Value(old)) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        Some(_) => return Err(format!("`{path}` is a table in the file, not a value")),
        None => {
            table.insert(key, Item::Value(value));
        }
    }
    Ok(())
}

/// Writes the changes to the file. A new text goes to a second file first, which then takes
/// the name of the file: a stop in the middle of the write leaves the old file whole.
pub fn write(path: &Path, edits: &[Edit]) -> Result<(), String> {
    let path = &target(path)?;
    let text = match super::read_text(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read the file: {error}")),
    };
    let changed = apply(&text, edits)?;
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(|error| format!("cannot make the folder: {error}"))?;
    }
    let next = path.with_extension("toml.new");
    fs::write(&next, changed).map_err(|error| format!("cannot write {}: {error}", next.display()))?;
    fs::rename(&next, path).map_err(|error| format!("cannot replace the file: {error}"))
}

/// The file that a save replaces: the file of a symbolic link, so that a link into a folder
/// of dotfiles stays a link. A file that is absent is the path itself.
fn target(path: &Path) -> Result<PathBuf, String> {
    match fs::canonicalize(path) {
        Ok(target) => Ok(target),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path.to_owned()),
        Err(error) => Err(format!("cannot find the file of {}: {error}", path.display())),
    }
}

#[cfg(test)]
mod tests;
