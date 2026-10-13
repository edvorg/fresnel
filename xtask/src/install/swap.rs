//! The install of a bundle by rename, with the earlier bundle as a fallback.
//!
//! A rename is one step of the file system: the folder holds the old bundle or the new
//! one, never a mix of the two. A copy over the old bundle could stop in the middle. The
//! earlier bundle stays with the extension `.prev`, which macOS does not show as an app.

use std::io;
use std::path::{Path, PathBuf};

use super::bundle::APP;

/// The extension of the fallback: `Fresnel.app.prev`.
const FALLBACK: &str = "prev";

/// What an install left in the destination folder.
#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    /// The new bundle.
    pub app: PathBuf,
    /// The bundle from before the install, if there was one.
    pub fallback: Option<PathBuf>,
}

/// The path of the fallback bundle in a destination folder.
pub fn fallback_path(destination: &Path) -> PathBuf {
    destination.join(format!("{APP}.{FALLBACK}"))
}

/// Moves a staged bundle into the destination folder. The staged bundle must be on the
/// same volume, so that each step is a rename. An older fallback goes away after the new
/// bundle is in its place. After an error, the folder is as before.
pub fn install(staged: &Path, destination: &Path) -> io::Result<Installed> {
    if !staged.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("no bundle at {}", staged.display())));
    }
    let (app, fallback) = (destination.join(APP), fallback_path(destination));
    if !exists(&app)? {
        std::fs::rename(staged, &app)?;
        return Ok(Installed { app, fallback: None });
    }
    // The older fallback waits under this name, so that a failed install can put it back.
    let retired = destination.join(format!("{APP}.{FALLBACK}.old"));
    remove(&retired)?;
    let had_fallback = exists(&fallback)?;
    if had_fallback {
        std::fs::rename(&fallback, &retired)?;
    }
    if let Err(error) = replace(staged, &app, &fallback) {
        if had_fallback {
            std::fs::rename(&retired, &fallback)?;
        }
        return Err(error);
    }
    remove(&retired)?;
    Ok(Installed { app, fallback: Some(fallback) })
}

/// Makes the installed bundle the fallback and puts the staged bundle into its place.
fn replace(staged: &Path, app: &Path, fallback: &Path) -> io::Result<()> {
    std::fs::rename(app, fallback)?;
    if let Err(error) = std::fs::rename(staged, app) {
        std::fs::rename(fallback, app)?;
        return Err(error);
    }
    Ok(())
}

/// Removes a bundle. A bundle that is absent is no error.
fn remove(bundle: &Path) -> io::Result<()> {
    if exists(bundle)? { std::fs::remove_dir_all(bundle) } else { Ok(()) }
}

/// True for a path that exists. A link that points to nothing exists too.
fn exists(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}
