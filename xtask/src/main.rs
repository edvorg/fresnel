//! Repository tasks. Run one with `cargo xtask <command>`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use xtask::version::Version;
use xtask::{borders, install, screenshots, sizes, soak, windows};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  check-sizes            check the size limits of the documents and the source files
  check-borders          check that fresnel-net has no kube dependency and fresnel-core
                         no egui dependency
  install [--to <folder>]
                         build Fresnel.app and install it into /Applications or into
                         <folder>; the earlier bundle stays as Fresnel.app.prev
  windows --to <folder>  build fresnel.exe with its icon and its version (on Windows)
  soak --hours <number> [--label <text>] [--report <folder>] [--seed <number>] [--fast]
                         run the app core for some hours on k3s in Docker with changes
                         and random faults, and write a report of each minute; --fast
                         gives more changes, lines and faults, and a recreate of the cluster
  screenshots            draw the pictures of the README into docs/screenshots (on macOS)

The variable FRESNEL_BUILD of install and windows is the build number of a release: the
version is then <version of the workspace>+build.<number>.";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["check-sizes"] => check_sizes(),
        ["check-borders"] => check_borders(),
        ["install"] => install(Path::new(install::APPLICATIONS)),
        ["install", "--to", folder] => install(Path::new(folder)),
        ["windows", "--to", folder] => windows(Path::new(folder)),
        ["soak", options @ ..] => soak(options),
        ["screenshots"] => draw_screenshots(),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The root of the workspace in which `cargo xtask` runs.
fn root() -> PathBuf {
    xtask::workspace_root(std::env::var_os("CARGO_MANIFEST_DIR"))
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

fn check_borders() -> ExitCode {
    let borders = borders::check(&root());
    for problem in &borders.problems {
        eprintln!("{problem}");
    }
    if borders.problems.is_empty() {
        println!("check-borders: {} rules hold", borders.rules);
        ExitCode::SUCCESS
    } else {
        eprintln!("check-borders: {} problems", borders.problems.len());
        ExitCode::FAILURE
    }
}

/// The version of the build, from the variable `FRESNEL_BUILD`. A wrong value stops the command.
fn version(command: &str) -> Option<Version> {
    Version::of_environment().map_err(|problem| eprintln!("{command}: {problem}")).ok()
}

fn install(destination: &Path) -> ExitCode {
    let Some(version) = version("install") else { return ExitCode::FAILURE };
    match install::run(&root(), destination, &version) {
        Ok(installed) => {
            println!("install: {} is the new version, {version}", installed.app.display());
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

fn windows(destination: &Path) -> ExitCode {
    let Some(version) = version("windows") else { return ExitCode::FAILURE };
    match windows::run(&root(), destination, &version) {
        Ok(program) => {
            println!("windows: {} is fresnel {version}", program.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("windows: {error}");
            ExitCode::FAILURE
        }
    }
}

fn soak(options: &[&str]) -> ExitCode {
    let soak = match soak::parse(options) {
        Ok(soak) => soak,
        Err(problem) => {
            eprintln!("{problem}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match soak::run(&root(), &soak) {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("soak: the soak failed ({status}); soak.txt of the report tells why");
            ExitCode::FAILURE
        }
        Err(problem) => {
            eprintln!("soak: {problem}");
            ExitCode::FAILURE
        }
    }
}

fn draw_screenshots() -> ExitCode {
    match screenshots::run(&root()) {
        Ok(pictures) => {
            for picture in &pictures {
                println!("screenshots: {}/{}: {} bytes", screenshots::FOLDER, picture.name, picture.file.len());
            }
            let total: usize = pictures.iter().map(|picture| picture.file.len()).sum();
            println!("screenshots: {} pictures, {total} bytes", pictures.len());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("screenshots: {error}");
            ExitCode::FAILURE
        }
    }
}
