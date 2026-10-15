//! The login shell resolve with a real shell process. The shell is a script of the tests,
//! so that the test does not run the profile of the user.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use fresnel_net::env::{EnvSettings, EnvSource, ProcessEnv, ShellFailure, resolve};

use crate::common::in_time;
use crate::plugin::{Scratch, fixture, process_exists};

fn base(mode: &str, state: &Scratch) -> ProcessEnv {
    [
        ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin".to_owned()),
        ("HOME", "/Users/tester".to_owned()),
        ("TERM", "xterm-256color".to_owned()),
        ("FAKE_SHELL_MODE", mode.to_owned()),
        ("FAKE_SHELL_STATE", state.path().display().to_string()),
    ]
    .into_iter()
    .collect()
}

fn settings(limit: Duration) -> EnvSettings {
    EnvSettings {
        shell_limit: limit,
        extra_folders: vec![PathBuf::from("/setting/bin")],
        shell: Some(fixture("fake-shell")),
        ..EnvSettings::default()
    }
}

fn path(vars: &ProcessEnv) -> Vec<String> {
    vars.path_entries().iter().map(|entry| entry.display().to_string()).collect()
}

#[tokio::test]
async fn the_environment_of_the_login_shell_has_the_values_of_the_profile() {
    let state = Scratch::new("shell-ok");
    let environment = resolve(&base("ok", &state), &settings(Duration::from_secs(5))).await;
    assert_eq!(environment.source, EnvSource::LoginShell(fixture("fake-shell")));
    let expected = [
        "/setting/bin",
        "/profile/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/Users/tester/.local/bin",
    ];
    assert_eq!(path(&environment.vars), expected, "the setting, the profile, the process, then the fixed folders");
    assert_eq!(environment.vars.get("PROFILE_VALUE"), Some(OsStr::new("line one\nline two")));
    assert_eq!(environment.vars.get("TERM"), Some(OsStr::new("xterm-256color")), "the value of the process");
    assert_eq!(environment.vars.get("DISABLE_AUTO_UPDATE"), None);
    assert!(environment.elapsed < Duration::from_secs(2), "{:?}", environment.elapsed);
}

#[tokio::test]
async fn a_shell_that_hangs_gives_the_fixed_folders_at_the_time_limit() {
    let state = Scratch::new("shell-hang");
    let started = Instant::now();
    let environment = resolve(&base("hang", &state), &settings(Duration::from_secs(1))).await;
    let needed = started.elapsed();
    assert!(Duration::from_secs(1) <= needed && needed < Duration::from_secs(3), "{needed:?}");
    let EnvSource::Fallback(problem) = &environment.source else { panic!("{:?}", environment.source) };
    assert_eq!(problem.failure(), &ShellFailure::Timeout(Duration::from_secs(1)));
    // The text of the profile is for the UI. The text for the log does not have it.
    assert!(problem.detail().contains("the profile waits for a key"), "{}", problem.detail());
    assert!(
        !problem.to_string().contains("secret-text") && !format!("{problem:?}").contains("secret-text"),
        "{problem:?}"
    );
    // The app still finds a plugin of Homebrew.
    let expected = [
        "/setting/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/Users/tester/.local/bin",
    ];
    assert_eq!(path(&environment.vars), expected);
}

#[tokio::test]
async fn a_shell_that_writes_no_dump_gives_the_fixed_folders_and_its_exit_status() {
    let state = Scratch::new("shell-no-dump");
    let environment = resolve(&base("no-dump", &state), &settings(Duration::from_secs(5))).await;
    let EnvSource::Fallback(problem) = &environment.source else { panic!("{:?}", environment.source) };
    assert_eq!(problem.failure(), &ShellFailure::NoDump("exit status: 3".to_owned()));
    assert!(path(&environment.vars).contains(&"/opt/homebrew/bin".to_owned()));

    let absent = EnvSettings { shell: Some(PathBuf::from("/no/such/shell")), ..EnvSettings::default() };
    let environment = resolve(&base("ok", &state), &absent).await;
    let EnvSource::Fallback(problem) = &environment.source else { panic!("{:?}", environment.source) };
    assert!(matches!(problem.failure(), ShellFailure::Start(_)), "{problem}");
}

/// A profile can run a slow step at its end or leave a process. The resolve returns when
/// the dump is complete, and the process of the profile does not stay.
#[tokio::test]
async fn the_resolve_does_not_wait_for_the_end_of_the_shell_and_leaves_no_process() {
    let state = Scratch::new("shell-slow-end");
    let started = Instant::now();
    let environment = resolve(&base("slow-end", &state), &settings(Duration::from_secs(5))).await;
    assert!(started.elapsed() < Duration::from_secs(2), "the resolve waited {:?}", started.elapsed());
    assert_eq!(environment.source, EnvSource::LoginShell(fixture("fake-shell")));
    assert_eq!(path(&environment.vars)[1], "/profile/bin");

    let child = state.read("child");
    let gone = async {
        while process_exists(&child) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    in_time("the process of the profile is gone", gone).await;
}
