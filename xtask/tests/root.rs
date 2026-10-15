//! The workspace that a command of xtask works on.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::path::Path;
use std::process::Command;

/// A second checkout that builds into the same target folder leaves a binary of xtask with
/// the path of that checkout. `cargo xtask check-sizes` then checked the other checkout and
/// passed. Thus the command works on the workspace that cargo names at its start.
#[test]
fn a_command_checks_the_workspace_that_cargo_names_and_not_the_one_of_its_build() {
    // A workspace with no file: each document and each source folder is absent.
    let workspace = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("root-{}", std::process::id()));
    let crate_folder = workspace.join("xtask");
    std::fs::create_dir_all(&crate_folder).expect("a new folder");

    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    let output = command.arg("check-sizes").env("CARGO_MANIFEST_DIR", &crate_folder).output().expect("xtask starts");
    std::fs::remove_dir_all(&workspace).expect("the folder goes away");

    let problems = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "the check passed on the workspace of the build");
    assert!(problems.contains("README.md: cannot read the file"), "{problems}");
    assert!(problems.contains("check-sizes: 5 problems in 4 files"), "{problems}");
}

#[test]
fn with_no_cargo_the_workspace_is_the_one_of_the_build() {
    let root = xtask::workspace_root(None);
    assert!(root.join("xtask/src/lib.rs").is_file(), "{root:?}");
    assert_eq!(xtask::workspace_root(Some("/work/fresnel/xtask".into())), Path::new("/work/fresnel/xtask/.."));
}

/// `cargo test` gives this test the variables of the package, as `cargo run` gives them
/// to `cargo xtask`. A cargo that a task starts gets none of them, and keeps the others.
#[test]
fn a_cargo_of_a_task_gets_no_variable_of_the_package_of_xtask() {
    let command = xtask::cargo(Path::new("."));
    let removed: Vec<String> = command
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    for name in ["CARGO_MANIFEST_DIR", "CARGO_PKG_NAME", "CARGO_PKG_VERSION"] {
        assert!(removed.iter().any(|removed| removed == name), "{name} is in {removed:?}");
    }
    assert!(!removed.iter().any(|removed| removed == "CARGO_TARGET_DIR" || removed == "PATH"), "{removed:?}");
}
