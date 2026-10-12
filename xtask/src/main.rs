//! Repository tasks. Run one with `cargo xtask <command>`.

mod sizes;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  check-sizes  check the size limits of the documents and the source files
  install      build Fresnel.app and install it (not available yet)";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [command] if command == "check-sizes" => check_sizes(),
        [command] if command == "install" => install(),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn check_sizes() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let sizes = sizes::check(&root);
    for problem in &sizes.problems {
        eprintln!("{problem}");
    }
    if sizes.problems.is_empty() {
        println!("check-sizes: {} files are inside their limits", sizes.files);
        ExitCode::SUCCESS
    } else {
        eprintln!("check-sizes: {} problems in {} files", sizes.problems.len(), sizes.files);
        ExitCode::FAILURE
    }
}

/// The placeholder of the install command. It fails, so that no script reads it as an install.
fn install() -> ExitCode {
    eprintln!("install: not available yet. The bundle step of milestone M0 adds this command.");
    ExitCode::FAILURE
}
