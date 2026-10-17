//! The harness of the soak (plan section 13, line 8): a k3s cluster in Docker behind a fault
//! proxy, objects that change all the time, random faults, and `fresnel-cli soak` as a child
//! process that runs the app core on the cluster and records each minute.
//!
//! `cargo xtask soak --hours <N>` builds and runs it. The child gets the kubeconfig through the
//! proxy for the core, the direct one for the lists that it compares the stores with, and a
//! file in which the harness writes each fault. The exit status is the one of the child.
//!
//! - `args`: the options. `pace`: the pace of the changes, lines and faults. `scene`: the
//!   first objects. `churn`: their changes. `faults`: the faults. `keeper`: the outages of
//!   the environment.

mod args;
mod churn;
mod faults;
mod keeper;
mod pace;
mod scene;

use std::convert::Infallible;
use std::path::Path;
use std::process::{ExitCode, Stdio};
use std::time::Duration;

use fresnel_testkit::{FaultProxy, K3s, K3sOptions};
use tokio::runtime::Builder;

use args::Options;
use pace::Pace;

/// The wait for the tasks of the runtime after the end. The cluster is gone by then.
const EXIT_LIMIT: Duration = Duration::from_secs(1);

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let options = match args::parse(&arguments) {
        Ok(options) => options,
        Err(problem) => {
            eprintln!("{problem}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let runtime = match Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("fresnel-soak: cannot start the runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run(&options));
    runtime.shutdown_timeout(EXIT_LIMIT);
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(problem) => {
            eprintln!("fresnel-soak: {problem}");
            ExitCode::FAILURE
        }
    }
}

/// The run. The fields of the cluster drop at its end, also after an error or Ctrl+C: the
/// proxy first, then the containers.
async fn run(options: &Options) -> Result<bool, String> {
    let folder = &options.report;
    std::fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    println!("fresnel-soak: k3s with metrics-server starts; seed {}", options.seed);
    let k3s = K3s::start_with(&options.label, K3sOptions::default().metrics_server()).await.map_err(text)?;
    let proxy = FaultProxy::start(k3s.api_addr()).await.map_err(text)?;
    let through = k3s.kubeconfig_through(proxy.addr()).map_err(text)?;
    let pace = if options.fast { Pace::fast(options.minutes) } else { Pace::NORMAL };
    scene::create(&k3s, pace.line_pause).await?;
    let faults_file = folder.join("faults.log");
    std::fs::write(&faults_file, "").map_err(|error| format!("cannot write {}: {error}", faults_file.display()))?;
    let mut child = soak_command(options, &through, &k3s.kubeconfig(), &faults_file)
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", options.cli.display()))?;
    println!("fresnel-soak: {} runs for {} minutes in {}", k3s.name(), options.minutes, folder.display());

    let mut rng = fastrand::Rng::with_seed(options.seed);
    let mut churn = churn::Churn::new(&k3s, rng.fork(), pace.step);
    let mut faults = faults::Faults::new(&k3s, &proxy, &faults_file, rng.fork(), &pace);
    let changes = async {
        tokio::select! {
            never = churn.run() => match never {},
            faults = faults.run() => faults,
        }
    };
    let status = tokio::select! {
        status = child.wait() => status.map_err(|error| format!("the soak process: {error}"))?,
        ended = changes => match ended {
            Ok(never) => match never {},
            Err(error) => return Err(format!("the environment of the soak failed: {error}")),
        },
        _ = tokio::signal::ctrl_c() => return Err("stopped by Ctrl+C".to_owned()),
    };
    println!("fresnel-soak: {}; {}", churn.summary(), faults.summary());
    Ok(status.success())
}

/// The child: `fresnel-cli soak` with the kubeconfig through the proxy for the core.
fn soak_command(options: &Options, through: &Path, direct: &Path, faults: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(&options.cli);
    command
        .arg("soak")
        .args(["--context", "default", "--kubeconfig"])
        .arg(through)
        .arg("--compare")
        .arg(direct)
        .arg("--report")
        .arg(&options.report)
        .args(["--minutes", &options.minutes.to_string(), "--log", scene::LOGGER, "--forward", scene::FORWARD])
        .arg("--faults")
        .arg(faults)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// A loop of the harness that has no end.
type Never = Infallible;
