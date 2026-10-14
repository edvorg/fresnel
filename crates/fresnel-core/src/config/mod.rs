//! The settings file `config.toml` (plan section 12.5).
//!
//! The file has a version number. The app uses each key that it knows and reports each
//! other key, each value that does not fit and a parse error with its line and column.
//! The app never writes the file: the settings view of a later milestone edits it, and the
//! app must not write over a file that it cannot read.
//!
//! This version reads the key of each command (`[shortcuts]`). A later milestone adds the
//! other groups of plan section 12.5.
//!
//! - `read`: the reader of the text, with the place of each problem.

mod read;

use std::fmt::{self, Display};
use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;

pub use read::parse;

use crate::commands::Keys;
use crate::error::CoreError;

/// The name of the file in the settings folder of the app.
pub const FILE_NAME: &str = "config.toml";
/// The version of the format that this code reads.
pub const VERSION: i64 = 1;
/// A larger file is not a settings file of the app. The limit keeps it out of the memory.
const MAX_BYTES: u64 = 1024 * 1024;

/// The settings of the user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// The key of each command.
    pub keys: Keys,
}

/// One problem of the file, at its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The line, from 1.
    pub line: usize,
    /// The character in the line, from 1.
    pub column: usize,
    pub text: String,
}

impl Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}: {}", self.line, self.column, self.text)
    }
}

/// The settings of a file and its problems.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    pub problems: Vec<Problem>,
    /// False when the app uses no setting of the file: a parse error, or a version that
    /// this app does not read. The settings then have their defaults.
    pub used: bool,
}

/// Reads the file. `None` when it is absent: each setting has its default.
pub fn load(path: &Path) -> Result<Option<Loaded>, CoreError> {
    match read_text(path) {
        Ok(text) => Ok(Some(parse(&text))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CoreError::invalid(format!("cannot read the file: {error}"))),
    }
}

fn read_text(path: &Path) -> io::Result<String> {
    let mut text = String::new();
    let read = File::open(path)?.take(MAX_BYTES + 1).read_to_string(&mut text)?;
    if u64::try_from(read).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err(io::Error::other(format!("it has more than {MAX_BYTES} bytes")));
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
