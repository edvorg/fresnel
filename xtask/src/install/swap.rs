//! The install of a bundle, with the earlier bundle as a fallback.
//!
//! The folder of the app stays the same folder: the install moves its `Contents` into a
//! new fallback folder and the staged `Contents` into it. The Dock and each other link of
//! macOS find a folder by its identity, not by its name. After an install that renamed the
//! folder of the app to the fallback, the Dock started the earlier version.
//!
//! Each move is a rename, one step of the file system: the folder holds the old contents
//! or the new ones, never a mix. A copy over the old bundle could stop in the middle. The
//! earlier bundle stays with the extension `.prev`, which macOS does not show as an app.

use std::io;
use std::path::{Path, PathBuf};

use super::bundle::APP;

/// The extension of the fallback: `Fresnel.app.prev`.
const FALLBACK: &str = "prev";
/// The one folder of a bundle of macOS.
const CONTENTS: &str = "Contents";

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

/// Moves the contents of a staged bundle into the destination folder. The staged bundle
/// must be on the same volume, so that each step is a rename. An older fallback goes away
/// after the new contents are in their place. After an error, the folder is as before.
pub fn install(staged: &Path, destination: &Path) -> io::Result<Installed> {
    if !staged.join(CONTENTS).is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("no bundle at {}", staged.display())));
    }
    let (app, fallback) = (destination.join(APP), fallback_path(destination));
    if !exists(&app)? {
        std::fs::rename(staged, &app)?;
        return Ok(Installed { app, fallback: None });
    }
    if !exists(&app.join(CONTENTS))? {
        // An app folder with no contents has nothing for a fallback: the older one stays.
        std::fs::rename(staged.join(CONTENTS), app.join(CONTENTS))?;
        std::fs::remove_dir(staged)?;
        return Ok(Installed { fallback: exists(&fallback)?.then_some(fallback), app });
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
    std::fs::remove_dir(staged)?;
    Ok(Installed { app, fallback: Some(fallback) })
}

/// Moves the installed contents into a new fallback folder and the staged contents into
/// the folder of the app. After an error, each contents is back in its folder.
fn replace(staged: &Path, app: &Path, fallback: &Path) -> io::Result<()> {
    std::fs::create_dir(fallback)?;
    if let Err(error) = std::fs::rename(app.join(CONTENTS), fallback.join(CONTENTS)) {
        std::fs::remove_dir(fallback)?;
        return Err(error);
    }
    if let Err(error) = std::fs::rename(staged.join(CONTENTS), app.join(CONTENTS)) {
        std::fs::rename(fallback.join(CONTENTS), app.join(CONTENTS))?;
        std::fs::remove_dir(fallback)?;
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
