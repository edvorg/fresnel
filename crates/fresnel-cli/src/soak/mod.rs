//! The `soak` command (plan section 13, line 8): the app core on one cluster for a time,
//! with a check of each store against the API and a record of each minute.
//!
//! - `args`: the options. `app`: the core in the folders of the report. `look`: the copy of
//!   the core that the tasks read. `account`: the memory account of the core.
//! - `api` and `cover`: the lists of the comparison, and the test of a store against one.
//!   `check`: the task of each kind. `logs`: the numbers of the log view. `forward`: the
//!   forward and its check.
//! - `minute` and `report`: the records and the files. `verdict`: the rules, with `heap`, the
//!   memory rule, and `environment`, the outages of k3s and the sleeps of the computer.

mod account;
mod api;
mod app;
mod args;
mod check;
mod cover;
mod environment;
mod forward;
mod heap;
mod logs;
mod look;
mod minute;
mod report;
mod verdict;

use std::fs::File;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use fresnel_core::app::Command;
use fresnel_core::kinds::Kind;
use fresnel_core::net::task::{TaskReport, Tasks};
use fresnel_core::store::StoreState;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::time::{Instant, sleep_until, timeout};

use api::Api;
use args::Options;
use look::Look;
use minute::{Minute, Recorder};
use report::Report;

/// The longest wait for the first list of each store.
const START_LIMIT: Duration = Duration::from_secs(300);

/// Runs the command. The exit status is 0 when the soak passed.
pub(crate) fn run(arguments: &[String]) -> ExitCode {
    match args::parse(arguments) {
        Ok(options) => crate::runtime::run(|runtime| async move {
            match soak(&options, runtime).await {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(problem) => {
                    eprintln!("soak: {problem}");
                    ExitCode::FAILURE
                }
            }
        }),
        Err(problem) => crate::usage_error(&problem, args::USAGE),
    }
}

async fn soak(options: &Options, runtime: Handle) -> Result<bool, String> {
    let folder = &options.report;
    std::fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    let journal = File::create(folder.join("journal.log")).map_err(|error| format!("the journal file: {error}"))?;
    // A task of the soak that ends makes its record incomplete: the report names it.
    let ended = Arc::new(Mutex::new(Vec::new()));
    let tasks = Tasks::new(runtime.clone(), {
        let ended = Arc::clone(&ended);
        move |report: TaskReport| {
            eprintln!("soak: {report}");
            ended.lock().unwrap_or_else(PoisonError::into_inner).push(report.to_string());
        }
    });
    let app = app::start(options, runtime.clone())?;
    let compare = if options.compare.is_empty() { &options.kubeconfig } else { &options.compare };
    let api = Api::connect(&tasks, runtime, compare, &options.context)?;
    let heap = Arc::new(look::HeapRange::default());
    let (look_task, mut looks) = look::start(&tasks, &app, &heap);
    let started = Instant::now();
    let mut report = Report::new(options, minute::now());
    let kinds = until_stores(&mut looks).await?;
    let log = options.log.clone().map(|target| {
        let follow = logs::Follow { core: Arc::clone(&app.core), cluster: app.id.clone(), target };
        logs::start(&tasks, follow, &looks)
    });
    let (log_task, log) = log.unzip();
    let tally = check::Shared::default();
    let checks = check::start(&tasks, &kinds, &api.gateway(), &looks, &tally);
    let forward = options.forward.as_ref().map(|target| {
        let (rule, checks) = (target.rule(&app.id), forward::Checks::default());
        app.core.send(Command::StartForward(rule.clone()));
        (forward::start(&tasks, rule.clone(), &looks, &checks), (rule, checks))
    });
    let (forward_task, forward) = forward.unzip();
    let mut recorder = Recorder::new(journal, options.faults.clone(), (forward, log), heap);
    for number in 1..=options.minutes {
        sleep_until(started + Duration::from_secs(60 * number)).await;
        let tally = std::mem::take(&mut *tally.lock().unwrap_or_else(PoisonError::into_inner));
        let look = Arc::clone(&looks.borrow());
        let minute = recorder.record(number, &look, account::take(&app.core), tally);
        println!("{}", progress(&minute));
        report.push(minute);
        report.problems.extend(std::mem::take(&mut *ended.lock().unwrap_or_else(PoisonError::into_inner)));
        report.judge();
        report.write(folder)?;
    }
    // The checks stop before the connections of the comparison and the core.
    drop((checks, forward_task, log_task, look_task));
    app.core.stop();
    print!("\n{}", report.text());
    Ok(report.verdict.passed)
}

/// Waits until each store that is always on has its first list, and gives their kinds. A
/// store that the server refuses has no check: its list fails on the path of the comparison too.
async fn until_stores(looks: &mut watch::Receiver<Arc<Look>>) -> Result<Vec<Kind>, String> {
    let ready = |look: &Look| {
        let statuses = look.statuses();
        let done = |state: &StoreState| {
            state.is_live() || matches!(state, StoreState::Forbidden { .. } | StoreState::NotSupported { .. })
        };
        let all = !statuses.is_empty() && statuses.iter().all(|status| done(&status.state));
        all.then(|| statuses.iter().filter(|status| status.state.is_live()).map(|status| status.kind).collect())
    };
    let wait = async {
        loop {
            let look = Arc::clone(&looks.borrow_and_update());
            if let Some(kinds) = ready(&look) {
                return Ok(kinds);
            }
            if looks.changed().await.is_err() {
                return Err("the copy of the core stopped".to_owned());
            }
        }
    };
    let waited = timeout(START_LIMIT, wait).await;
    waited.map_err(|_| format!("the stores of the cluster were not live after {} s", START_LIMIT.as_secs()))?
}

/// One line of progress for the terminal.
fn progress(minute: &Minute) -> String {
    let silent = minute.gaps.iter().filter(|gap| gap.without_state).count();
    format!(
        "minute {}: heap {:.1} MiB (low {:.1}), {} connections, {} stores live, {} not live, {} checks, lag {}/{} ms, {} gaps ({silent} with no visible state)",
        minute.minute,
        minute.heap_bytes as f64 / (1024.0 * 1024.0),
        minute.heap_low as f64 / (1024.0 * 1024.0),
        minute.open_connections,
        minute.stores_live,
        minute.stores_not_live.len(),
        minute.checks,
        minute.lag_median_ms,
        minute.lag_max_ms,
        minute.gaps.len(),
    )
}
