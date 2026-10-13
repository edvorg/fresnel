//! The border around `egui_table` (plan section 7.2): only the wrapper module names the
//! crate, so an upgrade of it changes one file. A rule that only a document states does
//! not hold, thus this test reads the source files.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::path::{Path, PathBuf};

/// Each Rust file below `folder`.
fn source_files(folder: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(folder).expect("a source folder") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            source_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn only_the_table_wrapper_names_the_crate_egui_table() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&root, &mut files);
    assert!(files.len() > 20, "the test read the source files: {files:?}");
    let mut users: Vec<String> = files
        .iter()
        .filter(|file| std::fs::read_to_string(file).expect("a source file").contains("egui_table::"))
        .map(|file| file.strip_prefix(&root).expect("a file of the crate").display().to_string())
        .collect();
    users.sort();
    assert_eq!(users, ["widgets/table.rs"]);
}
