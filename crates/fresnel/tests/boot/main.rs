//! The start of the program before its window, and its quit, on each system: the folders of
//! the app in the home folder of the test, the lock of the process with its id, and the log
//! file. The test `finder` starts the program as from Finder, with the login shell.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use fresnel::boot::Boot;
use fresnel::folders::{Build, Folders};
use fresnel_core::net::env::ProcessEnv;
use fresnel_testkit::Home;

#[test]
fn the_start_takes_the_folders_and_the_lock_and_the_quit_writes_the_log() {
    let home = Home::new("boot", &[]).expect("a home folder");
    let env = home.finder_env();
    let boot = Boot::start(env.clone()).expect("a start");
    let folders = Folders::of(&env, Build::current()).expect("the folders");
    assert_eq!(boot.folders(), &folders);
    assert!(folders.data.starts_with(home.folder()) && folders.logs.starts_with(home.folder()), "{folders:?}");
    let lock = std::fs::read_to_string(folders.data.join("fresnel.lock")).expect("the lock file");
    assert_eq!(lock.trim(), std::process::id().to_string(), "the lock file has the id of the process");

    let second = Boot::start(env).expect_err("one process of the app at a time");
    assert_eq!(second.to_string(), format!("fresnel runs already (process {})", std::process::id()));
    let no_folders = Boot::start(ProcessEnv::default()).expect_err("no variable names the folders");
    assert!(no_folders.to_string().starts_with("the environment has no "), "{no_folders}");

    boot.quit();
    let log = std::fs::read_to_string(folders.logs.join("fresnel.log")).expect("the log file");
    assert!(log.contains(" INFO  [app] fresnel 0.1.0") && log.contains(" started; settings in "), "{log}");
    assert!(log.ends_with(" INFO  [app] quit complete\n"), "{log}");
}
