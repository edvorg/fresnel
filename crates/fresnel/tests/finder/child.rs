//! The child process: the program with no window, in the environment that the parent gave.

use std::time::{Duration, Instant};

use fresnel::boot::Boot;
use fresnel_core::app::{CoreView, Health, Startup};
use fresnel_core::net::env::ProcessEnv;
use serde_json::json;

use crate::RESULT;

/// The longest wait for a signal that must arrive.
pub(crate) const LIMIT: Duration = Duration::from_secs(30);

/// Starts the program as `main` does, waits until the cluster is live, quits, and prints
/// what it saw as one line of JSON.
pub(crate) fn run() {
    let env = ProcessEnv::from_process();
    let boot = Boot::start(env.clone()).expect("the program starts");
    let second_start = Boot::start(env.clone()).expect_err("one process of the app at a time").to_string();

    // The window gives the core a wake that asks for a repaint. Here it ends the wait.
    let waiter = std::thread::current();
    boot.core().start(move || waiter.unpark());
    let live = |view: &CoreView| {
        view.startup == Startup::Ready && view.clusters.iter().any(|row| row.health() == Health::Live)
    };
    let deadline = Instant::now() + LIMIT;
    let view = loop {
        let view = boot.core().view();
        if live(&view) {
            break view;
        }
        let left = deadline.checked_duration_since(Instant::now());
        std::thread::park_timeout(left.unwrap_or_else(|| panic!("the cluster is live in time: {view:#?}")));
    };

    // A thread that panics: the hook of the program writes a report, and the app runs on.
    let panicked = std::thread::scope(|scope| scope.spawn(|| panic!("the panic of the test")).join());
    assert!(panicked.is_err());

    let clusters: Vec<_> =
        view.clusters.iter().map(|row| json!([row.id.as_str(), row.selected, format!("{:?}", row.health())])).collect();
    let credential = view.clusters.first().and_then(|row| row.status()).and_then(|status| status.credential.clone());
    boot.quit();
    boot.quit();
    let result = json!({
        "process_path": std::env::var("PATH").unwrap_or_default(),
        "process_has_kubeconfig": std::env::var_os("KUBECONFIG").is_some(),
        "clusters": clusters,
        "shell_problem": view.shell_problem.map(|note| note.problem),
        "plugin_runs": credential.map(|credential| credential.plugin_runs),
        "data_folder": boot.folders().data.display().to_string(),
        "second_start": second_start,
        "journal_entries": boot.core().view().journal.total,
    });
    println!("{RESULT}{result}");
}
