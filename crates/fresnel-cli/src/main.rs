//! The command line program. It is for tests and for fault diagnosis.

mod contexts;
mod doctor;
mod runtime;
mod watch;
mod words;

use std::process::ExitCode;

const USAGE: &str = "\
usage: fresnel-cli <command>

commands:
  doctor  measure the connection to the clusters of the kubeconfig; it reads no object
  watch   show the state and the object count of each store of the selected clusters";

/// The exit status for a command line that the program cannot use.
const USAGE_ERROR: u8 = 2;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.split_first() {
        Some((command, options)) if command == "doctor" => doctor::run(options),
        Some((command, options)) if command == "watch" => watch::run(options),
        Some((command, _)) if command == "--version" => {
            println!("fresnel-cli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(USAGE_ERROR)
        }
    }
}

/// Tells the problem of a command line with the usage of its command.
fn usage_error(problem: &str, usage: &str) -> ExitCode {
    eprintln!("{problem}\n\n{usage}");
    ExitCode::from(USAGE_ERROR)
}

/// The error for a context that the user selected and that no kubeconfig file has.
fn no_context(name: &str, usage: &str) -> ExitCode {
    usage_error(&format!("the kubeconfig files have no context `{name}`"), usage)
}
