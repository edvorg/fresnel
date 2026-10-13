//! The command line program. It is for tests and for fault diagnosis.

mod doctor;

use std::process::ExitCode;

const USAGE: &str = "\
usage: fresnel-cli <command>

commands:
  doctor  measure the connection to the clusters of the kubeconfig; it reads no object";

/// The exit status for a command line that the program cannot use.
const USAGE_ERROR: u8 = 2;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.split_first() {
        Some((command, options)) if command == "doctor" => doctor::run(options),
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
