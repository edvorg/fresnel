//! The `doctor` command: a measurement of the connection to each selected cluster.
//!
//! The command runs the code of the app: the environment of the login shell, the kubeconfig
//! loader, the connects, the channels and the gateway. It changes nothing in a cluster. Its
//! only request is `GET /version`, so it never lists or reads an object.
//!
//! - `args`: the options. `report`: the result, which is also the JSON file. `text`: the
//!   text for the terminal.
//! - `connects`: new connections with and with no hedge. `session`: the connected cluster.

mod args;
mod connects;
mod report;
mod session;
mod text;

use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fresnel_core::cluster::{ClusterId, Registry};
use fresnel_core::kubeconfig::{self, Context, Kubeconfig};
use fresnel_core::net::NetSettings;
use fresnel_core::net::credential::PluginSpec;
use fresnel_core::net::env::{self, EnvSettings, EnvSource, Environment, ProcessEnv};
use fresnel_core::net::task::{TaskReport, Tasks};
use tokio::runtime::Handle;

use args::Options;
use report::{Measured, Report, Step, ms};

use crate::contexts;

/// The margin above the time limit of the plugin, for the wait for the first token.
const CREDENTIAL_MARGIN: Duration = Duration::from_secs(2);

/// Runs the command. The exit status is 0 when each selected cluster gave its version.
pub(crate) fn run(arguments: &[String]) -> ExitCode {
    match args::parse(arguments) {
        Ok(options) => crate::runtime::run(|runtime| async move { examine(&options, &runtime).await }),
        Err(problem) => crate::usage_error(&problem, args::USAGE),
    }
}

async fn examine(options: &Options, runtime: &Handle) -> ExitCode {
    let time = jiff::Timestamp::now().round(jiff::Unit::Second).map_or_else(|_| String::new(), |time| time.to_string());
    let base = ProcessEnv::from_process();
    let environment = env::resolve(&base, &EnvSettings::default()).await;
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
    let (clusters, task_reports) = measure(&selected, &loaded, &base, &environment.vars, options, runtime).await;
    let report = Report {
        tool: concat!("fresnel-cli ", env!("CARGO_PKG_VERSION")).to_owned(),
        time,
        environment: environment_report(&environment, &base),
        kubeconfig: kubeconfig_report(&loaded),
        clusters,
        task_reports,
    };
    print!("{}", text::render(&report));
    finish(&report, options)
}

/// Measures each selected cluster, one after the other. Gives the report of each cluster,
/// and one line for each task of the app that ended in the run.
async fn measure(
    selected: &[&Context],
    loaded: &Kubeconfig,
    base: &ProcessEnv,
    resolved: &ProcessEnv,
    options: &Options,
    runtime: &Handle,
) -> (Vec<report::Cluster>, Vec<String>) {
    let ended = Arc::new(Mutex::new(Vec::new()));
    let reports = Arc::clone(&ended);
    let tasks = Tasks::new(runtime.clone(), move |report: TaskReport| {
        reports.lock().unwrap_or_else(PoisonError::into_inner).push(report.to_string());
    });
    let settings = settings(options);
    let limits = session::Limits {
        credential: settings.credential.plugin_limit + CREDENTIAL_MARGIN,
        idle_close: options.idle_close,
    };
    let registry = Registry::new(tasks, runtime.clone(), resolved.clone(), settings);
    registry.set_contexts(loaded.contexts.clone());
    let mut clusters = Vec::new();
    for context in selected {
        // The report comes at the end, and a run can take minutes.
        let wait = options.idle_close.map_or_else(String::new, |limit| format!(", with a wait of {limit:?} at most"));
        eprintln!("doctor: the cluster {}{wait}", context.name);
        clusters.push(cluster(&registry, context, base, resolved, options, &limits).await);
    }
    drop(registry);
    let task_reports = ended.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (clusters, task_reports)
}

fn kubeconfig_report(loaded: &Kubeconfig) -> report::Kubeconfig {
    let texts = |paths: &[std::path::PathBuf]| paths.iter().map(|path| path.display().to_string()).collect();
    let problem = |problem: &kubeconfig::Problem| format!("{}: {}", problem.path.display(), problem.text);
    report::Kubeconfig {
        sources: texts(&loaded.sources),
        absent: texts(&loaded.absent),
        contexts: loaded.contexts.len(),
        problems: loaded.problems.iter().map(problem).collect(),
    }
}

/// Writes the JSON file and gives the exit status.
fn finish(report: &Report, options: &Options) -> ExitCode {
    let written = serde_json::to_string_pretty(report)
        .map_err(|error| error.to_string())
        .and_then(|json| std::fs::write(&options.json, json + "\n").map_err(|error| error.to_string()));
    if let Err(error) = written {
        eprintln!("cannot write {}: {error}", options.json.display());
        return ExitCode::FAILURE;
    }
    println!("\nJSON report: {}", options.json.display());
    if report.clusters.iter().all(answered) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// True when the cluster gave its version: the full way to the server works.
fn answered(cluster: &report::Cluster) -> bool {
    let Step::Done(Measured { session: Step::Done(session), .. }) = &cluster.result else {
        return false;
    };
    matches!(session.version, Step::Done(_))
}

/// The settings of the app. For the idle close measurement, the channels for log streams
/// send no light request: such a channel is idle after its first answer.
fn settings(options: &Options) -> NetSettings {
    let mut settings = NetSettings::default();
    if options.idle_close.is_some() {
        settings.logs.light_period = Duration::MAX;
    }
    settings
}

fn environment_report(environment: &Environment, base: &ProcessEnv) -> report::Environment {
    let (source, problem) = match &environment.source {
        EnvSource::LoginShell(shell) => (format!("login shell {}", shell.display()), None),
        EnvSource::Fallback(problem) => ("process".to_owned(), Some(problem.to_string())),
    };
    report::Environment {
        source,
        problem,
        resolve_ms: ms(environment.elapsed),
        path_entries: environment.vars.path_entries().len(),
        process_path_entries: base.path_entries().len(),
    }
}

async fn cluster(
    registry: &Registry,
    context: &Context,
    base: &ProcessEnv,
    resolved: &ProcessEnv,
    options: &Options,
    limits: &session::Limits,
) -> report::Cluster {
    let name = context.name.clone();
    let access = match &context.access {
        Ok(access) => access,
        Err(error) => {
            return report::Cluster {
                context: name,
                server: None,
                auth: None,
                plugin: None,
                result: Step::failed(error),
            };
        }
    };
    let id = ClusterId::new(name.as_str());
    let result = match registry.probe(&id) {
        Ok(probe) => {
            let facts = connects::measure(&probe, options.samples, options.connect_limit).await;
            // The channels need an HTTP/2 connection. With none, the session would only
            // wait for its time limits.
            let session = match (&facts.dns, &facts.alpn.http2) {
                (Step::Done(_), Step::Done(_)) => session::measure(registry, &id, limits).await,
                (Step::Failed { .. }, _) => Step::failed("not connected: the host has no address"),
                (Step::Done(_), Step::Failed { .. }) => {
                    Step::failed("not connected: no HTTP/2 connection to the server")
                }
            };
            Step::Done(Measured { dns: facts.dns, alpn: facts.alpn, plain: facts.plain, hedged: facts.hedged, session })
        }
        Err(error) => Step::failed(error),
    };
    report::Cluster {
        context: name,
        server: Some(access.server.to_string()),
        auth: Some(contexts::auth(&access.auth())),
        plugin: access.plugin().map(|plugin| plugin_report(plugin, base, resolved)),
        result,
    }
}

/// Where the plugin is, as the app would find it at a start from Finder.
fn plugin_report(plugin: &PluginSpec, base: &ProcessEnv, resolved: &ProcessEnv) -> report::Plugin {
    let folder = plugin.folder.as_deref();
    let started = Instant::now();
    let found = resolved.find_program(&plugin.command, folder);
    let search = started.elapsed();
    report::Plugin {
        command: plugin.command.clone(),
        folder: found.as_deref().and_then(std::path::Path::parent).map(|folder| folder.display().to_string()),
        in_process_path: base.find_program(&plugin.command, folder).is_some(),
        search_ms: ms(search),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_idle_measurement_gives_the_channels_for_log_streams_no_light_request() {
        let options = |idle_close| Options { idle_close, ..args::parse(&[]).expect("the options with no argument") };
        assert_eq!(settings(&options(None)), NetSettings::default());
        let idle = settings(&options(Some(Duration::from_secs(120))));
        assert_eq!(idle.logs.light_period, Duration::MAX);
        let logs = NetSettings::default().logs;
        assert_eq!(NetSettings { logs, ..idle }, NetSettings::default(), "no other setting changes");
    }
}
