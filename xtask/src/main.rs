//! Repository tasks. Run one with `cargo xtask <command>`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use xtask::{borders, install, screenshots, sizes};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  check-sizes            check the size limits of the documents and the source files
  check-borders          check that fresnel-net has no kube dependency and fresnel-core
                         no egui dependency
  install [--to <folder>]
                         build Fresnel.app and install it into /Applications or into
                         <folder>; the earlier bundle stays as Fresnel.app.prev
  screenshots            draw the pictures of the README into docs/screenshots";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["check-sizes"] => check_sizes(),
        ["check-borders"] => check_borders(),
        ["install"] => install(Path::new(install::APPLICATIONS)),
        ["install", "--to", folder] => install(Path::new(folder)),
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
