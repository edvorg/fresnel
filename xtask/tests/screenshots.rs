//! The write of the pictures of the README into their folder.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::fs;
use std::path::{Path, PathBuf};

use xtask::screenshots::{Picture, publish};

/// A new empty folder of this test in the test folder of the build.
fn folder(label: &str) -> PathBuf {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("screenshots-{label}-{}", std::process::id()));
    if folder.exists() {
        fs::remove_dir_all(&folder).expect("the old folder goes away");
    }
    fs::create_dir_all(&folder).expect("a new folder");
    folder
}

fn picture(name: &str, bytes: usize) -> Picture {
    Picture { name: name.to_owned(), file: vec![7; bytes] }
}

/// The names and the contents of the files of a folder.
fn files(folder: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(folder)
        .expect("the folder")
        .map(|entry| {
            let path = entry.expect("an entry").path();
            let name = path.file_name().expect("a name").to_string_lossy().into_owned();
            (name, fs::read(&path).expect("the file"))
        })
        .collect();
    files.sort();
    files
}

#[test]
fn a_picture_above_its_limit_changes_no_file() {
    let folder = folder("limit");
    fs::write(folder.join("pods.png"), b"old pods").expect("a file");
    fs::write(folder.join("logs.png"), b"old logs").expect("a file");
    let before = files(&folder);
    let error = publish(&[picture("pods.png", 10), picture("logs.png", 450_001)], &folder).expect_err("a refusal");
    assert!(error.to_string().contains("logs.png has 450001 bytes"), "{error}");
    assert_eq!(files(&folder), before);
    fs::remove_dir_all(&folder).expect("the folder goes away");
}

#[test]
fn the_folder_keeps_only_the_new_pictures_and_its_other_files() {
    let folder = folder("old");
    fs::write(folder.join("renamed.png"), b"a scene of before").expect("a file");
    fs::write(folder.join("notes.txt"), b"no picture").expect("a file");
    publish(&[picture("pods.png", 3)], &folder).expect("a write");
    assert_eq!(files(&folder), [("notes.txt".to_owned(), b"no picture".to_vec()), ("pods.png".to_owned(), vec![7; 3])]);
    fs::remove_dir_all(&folder).expect("the folder goes away");
}

#[test]
fn no_picture_is_an_error_and_keeps_the_old_ones() {
    let folder = folder("none");
    fs::write(folder.join("pods.png"), b"old pods").expect("a file");
    let error = publish(&[], &folder).expect_err("a refusal");
    assert!(error.to_string().contains("made no picture"), "{error}");
    assert_eq!(files(&folder), [("pods.png".to_owned(), b"old pods".to_vec())]);
    fs::remove_dir_all(&folder).expect("the folder goes away");
}
