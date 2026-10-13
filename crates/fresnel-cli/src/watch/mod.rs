//! The `watch` command: the state and the object count of each store of the selected
//! clusters, as the app sees them.
//!
//! The command runs the stores of the app: it reads a list of each kind and then follows
//! its watch. It changes nothing in a cluster. For a tier B kind, a Secret for example, it
//! asks for the metadata only.
//!
//! - `args`: the options. `text`: the report for the terminal.

mod args;
mod text;

use std::process::ExitCode;

use fresnel_core::cluster::{ClusterId, Link, Registry};
use fresnel_core::kubeconfig::{self, Context};
use fresnel_core::net::env::{self, EnvSettings, ProcessEnv};
use fresnel_core::net::task::{TaskReport, Tasks};
use fresnel_core::net::{NetSettings, WallClock};
use fresnel_core::published::Disposal;
use fresnel_core::store::{ClusterStores, StoreParts, StoreSettings};
use tokio::runtime::Handle;
use tokio::time::{Instant, sleep_until};

use args::Options;
use text::ClusterReport;

use crate::contexts;

/// Runs the command. With a run time, the exit status is 0 when each store of each
/// selected cluster is live at the end.
pub(crate) fn run(arguments: &[String]) -> ExitCode {
    match args::parse(arguments) {
        Ok(options) => crate::runtime::run(|runtime| follow(options, runtime)),
        Err(problem) => crate::usage_error(&problem, args::USAGE),
    }
}

/// One selected cluster with its stores. The stores are the first field: they stop
/// before the connections close.
struct Watched {
    stores: ClusterStores,
    link: Link,
}

async fn follow(options: Options, runtime: Handle) -> ExitCode {
    let environment = env::resolve(&ProcessEnv::from_process(), &EnvSettings::default()).await;
    let sources = match options.kubeconfig.as_slice() {
        [] => kubeconfig::default_sources(&environment.vars),
        given => given.to_vec(),
    };
    let loaded = kubeconfig::load(&sources);
    let selected = match contexts::selected(&loaded, &options.selection) {
        Ok(Some(selected)) => selected,
        Ok(None) => {
            print!("{}", contexts::list(&loaded));
            return ExitCode::SUCCESS;
        }
        Err(name) => return crate::no_context(name, args::USAGE),
    };
    // A task of the app that ends is a fault. Its report goes to the terminal at once.
    let tasks = Tasks::new(runtime.clone(), |report: TaskReport| eprintln!("watch: {report}"));
    let registry = Registry::new(tasks.clone(), runtime, environment.vars, NetSettings::default());
    registry.set_contexts(loaded.contexts.clone());
    let disposal = Disposal::start(&tasks);
    let parts = StoreParts { settings: StoreSettings::default(), bin: disposal.bin(), clock: WallClock::system() };
    let watched: Vec<Watched> =
        selected.iter().filter_map(|context| start(&registry, &tasks, context, &parts)).collect();
    let all_started = watched.len() == selected.len();
    let all_live = report_until(&watched, &options).await;
    // The stores stop before the thread that frees their snapshots.
    drop(watched);
    drop(disposal);
    if all_started && all_live { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Connects one cluster and starts its stores. A cluster that does not start gives its
/// reason and no entry.
fn start(registry: &Registry, tasks: &Tasks, context: &Context, parts: &StoreParts) -> Option<Watched> {
    let id = ClusterId::new(context.name.as_str());
    let started = registry.connect(&id).and_then(|link| {
        let stores = ClusterStores::start(tasks, &link, parts.clone())?;
        Ok(Watched { stores, link })
    });
    match started {
        Ok(watched) => Some(watched),
        Err(error) => {
            eprintln!("watch: the cluster {id} did not start: {error}");
            None
        }
    }
}

/// Prints one report each interval, to the end of the run time or to Ctrl+C. Gives true
/// when each store is live in the last report.
///
/// Ctrl+C ends the command in the clean way: the caller then stops each task, and a task
/// ends the process of a credential plugin that runs.
async fn report_until(watched: &[Watched], options: &Options) -> bool {
    let started = Instant::now();
    let end = options.time.map(|time| started + time);
    let mut next = started;
    loop {
        next += options.interval;
        let at_end = end.is_some_and(|end| end <= next);
        let last = tokio::select! {
            () = sleep_until(end.filter(|_| at_end).unwrap_or(next)) => at_end,
            signal = tokio::signal::ctrl_c() => {
                // With no signal of the system, the command has no clean end later. It ends now.
                if let Err(error) = signal {
                    eprintln!("watch: the system gives no signal for Ctrl+C: {error}");
                }
                true
            }
        };
        let reports: Vec<ClusterReport> = watched.iter().map(report).collect();
        let time =
            jiff::Timestamp::now().round(jiff::Unit::Second).map_or_else(|_| String::new(), |time| time.to_string());
        print!("{}", text::render(&time, &reports, WallClock::system().now()));
        if last {
            return reports.iter().all(|cluster| cluster.stores.iter().all(|store| store.state.is_live()));
        }
        println!();
    }
}

fn report(watched: &Watched) -> ClusterReport {
    ClusterReport {
        name: watched.stores.cluster().to_string(),
        connections: watched.link.net.status(),
        stores: watched.stores.statuses(),
    }
}
