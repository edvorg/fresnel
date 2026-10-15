//! The settings file `config.toml` (plan section 12.5).
//!
//! The file has a version number. The app uses each key that it knows and reports each
//! other key, each value that does not fit and a parse error with its line and column. A
//! number outside its bounds takes the nearest bound, with a note. The settings view edits
//! the file with `edit`, which keeps its comments; the app never writes over a file that it
//! cannot read.
//!
//! - `table`: each setting of the fixed groups with its default and its bounds.
//! - `values`: the value of a setting, from the file or the table. `clusters`: the options
//!   of each cluster and the cluster groups.
//! - `read`: the reader of the text, with the place of each problem. `apply`: the values of
//!   the file in the settings of the core. `edit`: a change of the settings view in the text.

pub(crate) mod apply;
mod clusters;
pub mod edit;
mod read;
pub mod table;
mod values;

use std::collections::BTreeMap;
use std::fmt::{self, Display};
use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;
use std::path::PathBuf;

pub use clusters::{ALIAS_LIMIT, ClusterGroup, ClusterOptions, ConfirmRule, OPTION_KEYS, Rgb};
pub use edit::{Edit, EditValue};
pub use read::fixed::{amount, is_time_zone};
pub use read::parse;
pub use values::{Theme, Value};

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
    /// The value of each setting of a fixed group that the file sets, inside its bounds.
    /// The getters of `values` give the default of the table for the others.
    pub values: BTreeMap<table::Setting, Value>,
    /// The options of each cluster that the file names, by the name of its context.
    pub clusters: BTreeMap<String, ClusterOptions>,
    /// The cluster groups, in the sequence of the file.
    pub groups: Vec<ClusterGroup>,
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

/// What the core read of the settings file, and the last save of the settings view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileState {
    /// The file.
    pub path: PathBuf,
    pub read: FileRead,
    /// The last save of the settings view. `None` before the first one.
    pub save: Option<SaveState>,
}

/// The last read of the settings file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FileRead {
    /// The core did not read the file yet.
    #[default]
    NotYet,
    /// No file: each setting has its default. A save makes the file.
    Absent,
    /// The file loaded, with its problems. `used` is false for a parse error or an other
    /// version: each setting then has its default, and the app does not write the file.
    Read { problems: Vec<Problem>, used: bool },
    /// The file could not be read, for this reason.
    Failed(String),
}

impl FileRead {
    /// Why the settings view cannot save, if it cannot: the app never writes over a file
    /// that it cannot read.
    pub fn refusal(&self) -> Option<String> {
        match self {
            Self::NotYet => Some("The app did not read the file yet.".to_owned()),
            Self::Absent | Self::Read { used: true, .. } => None,
            Self::Read { problems, used: false } => Some(match problems.first() {
                Some(problem) => format!("The file has a problem at {problem}. Correct it in an editor first."),
                None => "The app does not use the file.".to_owned(),
            }),
            Self::Failed(error) => Some(format!("The app cannot read the file: {error}.")),
        }
    }
}

/// A save of the settings view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveState {
    /// The number of the save: each one has a new number.
    pub number: u64,
    /// `None` while the core writes the file, then the result.
    pub result: Option<Result<(), String>>,
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
