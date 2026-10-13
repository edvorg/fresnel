//! Repository tasks. Run one with `cargo xtask <command>`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use xtask::{install, sizes};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  check-sizes            check the size limits of the documents and the source files
  install [--to <folder>]
                         build Fresnel.app and install it into /Applications or into
                         <folder>; the earlier bundle stays as Fresnel.app.prev";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["check-sizes"] => check_sizes(),
        ["install"] => install(Path::new(install::APPLICATIONS)),
        ["install", "--to", folder] => install(Path::new(folder)),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The root of the workspace: the folder above the folder of this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn check_sizes() -> ExitCode {
    let sizes = sizes::check(&root());
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

fn install(destination: &Path) -> ExitCode {
    match install::run(&root(), destination) {
        Ok(installed) => {
            println!("install: {} is the new version", installed.app.display());
            if let Some(fallback) = installed.fallback {
                println!("install: the earlier version is {}", fallback.display());
            }
            println!("install: quit a running Fresnel before the first start of the new version");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("install: {error}");
            ExitCode::FAILURE
        }
    }
}
