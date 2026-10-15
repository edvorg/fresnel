//! The download of plan section 10: the manifest of an object as the server gives it now,
//! with no `managedFields`, as a new YAML file in the folder of downloads. The manifest of a
//! Secret has its values: the view asks the user before it sends this order.
//!
//! The file gets its whole text in one write on a thread for blocking calls, so a slow disk
//! holds no task of the runtime. A write that fails removes the file: a part of a manifest
//! would look like a whole one.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::SystemTime;

use super::{JobState, Parts, Progress, Steps, read_failure};
use crate::downloads;
use crate::error::CoreError;
use crate::manifest::{self, ManagedFields};
use crate::ops::Target;

pub(super) async fn save<P: Progress>(target: &Target, parts: &Parts, job: &mut Steps<'_, P>) -> Result<(), JobState> {
    job.begin("read the manifest");
    let read = manifest::fetch(&parts.gateway, target.kind, &target.key, &target.uid, ManagedFields::Removed).await;
    let manifest = read.map_err(read_failure)?;
    let version = manifest.version.as_ref().map_or_else(String::new, |version| format!(", version {version}"));
    job.done(format!("read{version}"));
    if parts.cancel.is_set() {
        return Err(JobState::Cancelled);
    }
    job.begin("write the file");
    let stem = downloads::stem(&target.cluster, target.kind, &target.key);
    let (folder, at) = (parts.downloads.clone(), SystemTime::now());
    let write = move || write(&folder, &stem, at, manifest.text.as_bytes());
    let written = tokio::task::spawn_blocking(write).await;
    let path =
        written.map_err(|error| JobState::Failed(CoreError::invalid(format!("the write did not end: {error}"))))?;
    job.done(format!("saved as {}", path.map_err(JobState::Failed)?.display()));
    Ok(())
}

/// Writes a new file with the name of the object and the time, for example
/// `ams-prod_shop_deployment-web_20270115-080000.yaml`.
fn write(folder: &std::path::Path, stem: &str, at: SystemTime, text: &[u8]) -> Result<PathBuf, CoreError> {
    let fault = |error: std::io::Error| CoreError::invalid(format!("the file cannot be written: {error}"));
    let name = |attempt| format!("{stem}_{}{}.yaml", downloads::time(at), downloads::number(attempt));
    let (mut file, path) = downloads::create(folder, name).map_err(fault)?;
    match file.write_all(text).and_then(|()| file.sync_all()) {
        Ok(()) => Ok(path),
        Err(error) => {
            let removed = std::fs::remove_file(&path)
                .map_or_else(|stays| format!("; the part stays: {stays}"), |()| String::new());
            Err(CoreError::invalid(format!("the file cannot be written: {error}{removed}")))
        }
    }
}
