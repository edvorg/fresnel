//! The file `state.json`: what the app keeps from one run to the next (plan section 12.5).
//!
//! Its first value is the selection of clusters. Later milestones add the layout of the
//! window. The file has a version number, and a key that this version does not know is an
//! error. The app does not write over a file that it cannot read: such a file can be from
//! a newer version of the app.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read as _, Write as _};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::cluster::ClusterId;
use crate::error::CoreError;

/// The name of the file in the settings folder of the app.
pub const FILE_NAME: &str = "state.json";
/// The version of the format that this code reads and writes.
const VERSION: u32 = 1;
/// A larger file is not a state file of the app. The limit keeps it out of the memory.
const MAX_BYTES: u64 = 1024 * 1024;

/// What one run of the app gives to the next run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedState {
    /// The clusters that the user selected.
    pub selected: BTreeSet<ClusterId>,
}

/// The only key that each version of the format has.
#[derive(Deserialize)]
struct Versioned {
    version: u32,
}

/// The format of version 1.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    selected_clusters: Vec<String>,
}

/// Reads the file. `None` when it is absent: the app did not run before.
///
/// The error of a text that does not fit gives the position only, never a part of the text.
pub fn load(path: &Path) -> Result<Option<SavedState>, CoreError> {
    let text = match read(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(CoreError::invalid(format!("cannot read the file: {error}"))),
    };
    let found = serde_json::from_str::<Versioned>(&text).map_err(|error| position(&error))?.version;
    if found != VERSION {
        return Err(CoreError::invalid(format!("the format has the version {found}, and this app reads {VERSION}")));
    }
    let document: Document = serde_json::from_str(&text).map_err(|error| position(&error))?;
    Ok(Some(SavedState { selected: document.selected_clusters.into_iter().map(ClusterId::new).collect() }))
}

/// Writes the file. The text goes to a new file that then gets the name of the state file,
/// so the state file is complete at each moment, also after a crash in the middle of a write.
pub fn save(path: &Path, state: &SavedState) -> Result<(), CoreError> {
    let selected_clusters = state.selected.iter().map(|cluster| cluster.as_str().to_owned()).collect();
    let document = Document { version: VERSION, selected_clusters };
    let mut text = serde_json::to_string_pretty(&document)
        .map_err(|error| CoreError::invalid(format!("cannot write the state as JSON: {error}")))?;
    text.push('\n');
    let new = path.with_extension("json.new");
    let written = File::create(&new).and_then(|mut file| {
        file.write_all(text.as_bytes())?;
        file.sync_all()
    });
    written.and_then(|()| fs::rename(&new, path)).map_err(|error| CoreError::invalid(error.to_string()))
}

fn read(path: &Path) -> io::Result<String> {
    let mut text = String::new();
    let read = File::open(path)?.take(MAX_BYTES + 1).read_to_string(&mut text)?;
    if u64::try_from(read).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err(io::Error::other(format!("it has more than {MAX_BYTES} bytes")));
    }
    Ok(text)
}

fn position(error: &serde_json::Error) -> CoreError {
    let (line, column) = (error.line(), error.column());
    CoreError::invalid(format!("line {line}, column {column} does not fit the format of version {VERSION}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A folder for the files of one test. Its drop removes it.
    struct Folder(PathBuf);

    impl Folder {
        fn new(label: &str) -> Self {
            let folder = std::env::temp_dir().join(format!("fresnel-core-test-{label}-{:08x}", fastrand::u32(..)));
            fs::create_dir(&folder).expect("a new folder");
            Self(folder)
        }

        /// The state file of the folder, with this text.
        fn with_text(&self, text: &str) -> PathBuf {
            let path = self.0.join(FILE_NAME);
            fs::write(&path, text).expect("the file of the test");
            path
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                eprintln!("cannot remove {}: {error}", self.0.display());
            }
        }
    }

    fn selection(names: &[&str]) -> SavedState {
        SavedState { selected: names.iter().map(|name| ClusterId::new(*name)).collect() }
    }

    fn error_of(folder: &Folder, text: &str) -> String {
        load(&folder.with_text(text)).expect_err("a file that the app cannot use").to_string()
    }

    #[test]
    fn a_saved_state_loads_again_and_an_absent_file_is_no_state() {
        let folder = Folder::new("round-trip");
        let path = folder.0.join(FILE_NAME);
        assert_eq!(load(&path), Ok(None), "the first start of the app");

        save(&path, &selection(&["fra-stage", "ams-prod"])).expect("a write");
        let text = fs::read_to_string(&path).expect("the file");
        assert_eq!(
            text,
            "{\n  \"version\": 1,\n  \"selected_clusters\": [\n    \"ams-prod\",\n    \"fra-stage\"\n  ]\n}\n"
        );
        assert_eq!(load(&path), Ok(Some(selection(&["ams-prod", "fra-stage"]))));

        // An empty selection is a state too: the user selected no cluster.
        save(&path, &SavedState::default()).expect("a second write replaces the file");
        assert_eq!(load(&path), Ok(Some(SavedState::default())));
        let names: Vec<_> =
            fs::read_dir(&folder.0).expect("the folder").map(|entry| entry.expect("an entry").file_name()).collect();
        assert_eq!(names, [FILE_NAME], "the new file got the name of the state file");
    }

    #[test]
    fn a_file_that_does_not_fit_gives_the_position_and_no_part_of_its_text() {
        let folder = Folder::new("format");
        let broken = error_of(&folder, "{\n  \"version\": 1,\n  \"selected_clusters\": [\"secret-name\",\n}\n");
        assert_eq!(broken, "line 4, column 1 does not fit the format of version 1");
        let unknown_key = error_of(&folder, r#"{"version": 1, "selected_clusters": [], "layout": "secret-name"}"#);
        assert_eq!(unknown_key, "line 1, column 48 does not fit the format of version 1");
        let no_version = error_of(&folder, r#"{"selected_clusters": ["secret-name"]}"#);
        assert_eq!(no_version, "line 1, column 38 does not fit the format of version 1");
        assert_eq!(error_of(&folder, ""), "line 1, column 0 does not fit the format of version 1");
    }

    #[test]
    fn a_file_of_some_other_version_or_of_a_large_size_is_an_error() {
        let folder = Folder::new("version");
        let newer = error_of(&folder, r#"{"version": 2, "selection": {"clusters": ["ams-prod"]}}"#);
        assert_eq!(newer, "the format has the version 2, and this app reads 1");
        let large = error_of(&folder, &" ".repeat(1024 * 1024 + 1));
        assert_eq!(large, "cannot read the file: it has more than 1048576 bytes");
        let folder_as_file = load(&folder.0).expect_err("a folder is not a state file").to_string();
        assert!(folder_as_file.starts_with("cannot read the file: "), "{folder_as_file}");
    }

    #[test]
    fn a_write_that_fails_is_an_error_and_keeps_the_old_file() {
        use std::os::unix::fs::PermissionsExt as _;

        let folder = Folder::new("no-write");
        let path = folder.with_text("old text");
        let permit = |mode| fs::set_permissions(&folder.0, fs::Permissions::from_mode(mode)).expect("the mode");
        // A folder that takes no new file: the write fails before the rename.
        permit(0o500);
        let failed = save(&path, &selection(&["ams-prod"]));
        permit(0o700);
        assert!(failed.is_err(), "the folder took a new file");
        assert_eq!(fs::read_to_string(&path).expect("the file"), "old text");
    }
}
