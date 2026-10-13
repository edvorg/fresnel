//! The program with no window, started as from Finder.
//!
//! The parent test starts this test program again as a child process, with `env -i`: only
//! `HOME`, the `PATH` of Finder and one variable that marks the child. The child then runs
//! [`fresnel::boot::Boot`] as `main` does, with the environment of its process. The home
//! folder is a folder of the test, so no test reads or changes a file of the user.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod child;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use fresnel::folders::{Build, Folders};
use fresnel_testkit::{FINDER_PATH, FakeApiServer, Home};
use serde_json::Value;

/// The variable that tells the test program that it is the child process.
const CHILD: &str = "FRESNEL_TEST_CHILD";
/// The test that both processes run.
const TEST: &str = "the_app_core_starts_with_the_environment_of_finder";
/// The start of the line with the result of the child.
const RESULT: &str = "CHILD-RESULT ";
/// The command of the plugin in the kubeconfig: a bare name, which only the profile finds.
const PLUGIN: &str = "fresnel-test-plugin";
/// The start of each token that the plugin gives.
const TOKEN: &str = "secret-token-of-the-test";

/// The home folder of the child, with the plugin of the tests of `fresnel-net` and a
/// kubeconfig with one cluster on `server`.
fn home(server: &FakeApiServer) -> Home {
    let home = Home::new("program", &["kubeconfig.yaml"]).expect("a home folder");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fresnel-net/tests/fixtures/fake-plugin");
    home.add_program(PLUGIN, &format!("#!/bin/sh\nexec '{}'\n", fixture.display())).expect("the plugin");
    home.write("work/kubeconfig.yaml", &kubeconfig(&home, server.addr(), server)).expect("the kubeconfig");
    home
}

fn kubeconfig(home: &Home, address: SocketAddr, server: &FakeApiServer) -> String {
    let certificate = base64(server.certificate_der());
    let authority =
        base64(format!("-----BEGIN CERTIFICATE-----\n{certificate}\n-----END CERTIFICATE-----\n").as_bytes());
    let state = home.path("state");
    format!(
        "current-context: fake
clusters:
- {{name: fake, cluster: {{server: 'https://{address}', certificate-authority-data: {authority}}}}}
users:
- name: fake
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: {PLUGIN}
      env: [{{name: FAKE_PLUGIN_STATE, value: '{}'}}, {{name: FAKE_PLUGIN_TOKEN, value: {TOKEN}}}]
contexts:
- {{name: fake, context: {{cluster: fake, user: fake}}}}
",
        state.display()
    )
}

/// Base64 with padding. The test needs it for one certificate, so it has no crate for it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().fold(0_u32, |group, byte| group << 8 | u32::from(*byte)) << (8 * (3 - chunk.len()));
        for position in 0..4 {
            let symbol = ALPHABET[(group >> (18 - 6 * position) & 63) as usize];
            text.push(if position <= chunk.len() { char::from(symbol) } else { '=' });
        }
    }
    text
}

/// Runs this test program again as the child, with the environment of Finder.
fn run_child(home: &Home) -> Output {
    let program = std::env::current_exe().expect("the path of this test program");
    let mut command = Command::new("/usr/bin/env");
    command
        .args(["-i", &format!("HOME={}", home.folder().display()), &format!("PATH={FINDER_PATH}")])
        .arg(format!("{CHILD}=1"))
        .arg(program)
        .args(["--exact", TEST, "--nocapture"])
        .current_dir(home.folder())
        .stdin(Stdio::null());
    command.output().expect("the child process starts")
}

fn files(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(folder).expect("the folder").map(|entry| entry.expect("an entry").path()).collect();
    files.sort();
    files
}

/// The acceptance of milestone M0 for the program: it starts from Finder, finds the
/// kubeconfig and the plugin through the login shell, and connects.
#[test]
fn the_app_core_starts_with_the_environment_of_finder() {
    if std::env::var_os(CHILD).is_some() {
        return child::run();
    }
    let runtime =
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("a runtime");
    let server = runtime.block_on(FakeApiServer::start()).expect("a server");
    let home = home(&server);

    let output = run_child(&home);
    let (stdout, stderr) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "the child failed.\nstdout:\n{stdout}\nstderr:\n{stderr}");
    let result = stdout.lines().find_map(|line| line.strip_prefix(RESULT)).expect("the result line of the child");
    let result: Value = serde_json::from_str(result).expect("the result of the child is JSON");

    // The child had the environment of Finder, and the core got the rest from the login shell.
    assert_eq!(result["process_path"], FINDER_PATH);
    assert_eq!(result["process_has_kubeconfig"], false);
    assert_eq!(result["clusters"], serde_json::json!([["fake", true, "Live"]]));
    assert_eq!((&result["shell_problem"], &result["plugin_runs"]), (&Value::Null, &1.into()));
    assert_eq!(home.read("state/runs"), "run\n", "the plugin process ran one time");

    // A debug build has its own folders, in the home folder of the test.
    let folders = Folders::of(home.folder(), Build::current());
    assert_eq!(result["data_folder"], folders.data.display().to_string());
    assert!(folders.data.ends_with("fresnel-debug") || !cfg!(debug_assertions), "{folders:?}");
    assert_eq!(files(&folders.data), [folders.data.join("fresnel.lock")]);
    let lock = std::fs::read_to_string(folders.data.join("fresnel.lock")).expect("the lock file");
    assert_eq!(result["second_start"], format!("fresnel runs already (process {})", lock.trim()));

    // The log file has each entry of the journal, from the start to the quit.
    let log = std::fs::read_to_string(folders.logs.join("fresnel.log")).expect("the log file");
    let steps = [
        " INFO  [app] fresnel 0.1.0 (debug build) started; settings in ",
        " INFO  [app] the login shell /",
        " INFO  [app] read 1 context from 1 kubeconfig file",
        " INFO  [fake] the channels connect",
        " INFO  [fake] live: a request got an answer on each channel",
        " INFO  [app] quit: the tasks stop and the connections close",
        " INFO  [app] quit complete",
    ];
    let mut rest = log.as_str();
    for step in steps.iter().filter(|step| cfg!(debug_assertions) || !step.contains("debug build")) {
        let at = rest.find(step).unwrap_or_else(|| panic!("the log has `{step}` after the steps before it:\n{log}"));
        rest = &rest[at + step.len()..];
    }
    assert_eq!(rest, "\n", "the quit is the last line:\n{log}");
    assert!(!log.contains(TOKEN), "the log has no token:\n{log}");
    assert_eq!(result["journal_entries"], log.lines().count(), "the log file has each entry of the journal");

    // The panic of a thread of the child gave one report file.
    let reports = fresnel::panic_report::list(&folders.logs);
    assert_eq!(reports.len(), 1, "{:?}", files(&folders.logs));
    let report = std::fs::read_to_string(&reports[0]).expect("the panic report");
    assert!(report.starts_with("fresnel 0.1.0\nthread: "), "{report}");
    assert!(report.contains("\nmessage: the panic of the test\n\nbacktrace:\n"), "{report}");
    assert!(report.contains("/tests/finder/child.rs:"), "{report}");

    // The quit closed each connection of the child.
    let closed = server.wait_for(|counters| counters.connections >= 3 && counters.open_connections == 0);
    let counters = runtime.block_on(async { tokio::time::timeout(child::LIMIT, closed).await });
    assert!(counters.is_ok(), "the connections closed: {:?}", server.counters());
}
