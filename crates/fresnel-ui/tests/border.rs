//! The borders around `egui_table` and `egui_plot` (plan section 7.2): only the wrapper
//! module names its crate, so an upgrade of it changes one file. A rule that only a
//! document states does not hold, thus this test reads the source files.

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

/// The files of the crate that name `crate_name`, as paths below `src`.
fn users_of(crate_name: &str) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&root, &mut files);
    assert!(files.len() > 20, "the test read the source files: {files:?}");
    let mut users: Vec<String> = files
        .iter()
        .filter(|file| std::fs::read_to_string(file).expect("a source file").contains(&format!("{crate_name}::")))
        .map(|file| file.strip_prefix(&root).expect("a file of the crate").display().to_string())
        .collect();
    users.sort();
    users
}

#[test]
fn only_the_table_wrapper_names_the_crate_egui_table() {
    assert_eq!(users_of("egui_table"), ["widgets/table.rs"]);
}

#[test]
fn only_the_chart_wrapper_names_the_crate_egui_plot() {
    assert_eq!(users_of("egui_plot"), ["widgets/chart.rs"]);
}
