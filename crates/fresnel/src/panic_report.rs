//! The panic report: one file for each panic of the process (plan section 12.6).
//!
//! The start function of the tasks catches a panic of a task, and the task runs again.
//! A panic of the UI thread ends the app. In each case the hook of this module writes
//! what the panic was and where, so that the cause is not lost with the process.
//!
//! The hook writes its own file and takes no lock of the app: the thread that panics can
//! hold the lock of the journal.

use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const PREFIX: &str = "panic-";
const EXTENSION: &str = "txt";
/// The most report files of one run. A task that panics at each start would fill the disk.
const RUN_LIMIT: u32 = 20;
/// The most report files that stay from earlier runs.
const KEEP: usize = 10;

/// Writes the reports of this process into one folder.
#[derive(Debug)]
pub struct Reports {
    folder: PathBuf,
    version: &'static str,
    written: AtomicU32,
}

impl Reports {
    /// The reports go to `folder`. `version` is the version of the app.
    pub fn new(folder: &Path, version: &'static str) -> Self {
        Self { folder: folder.to_owned(), version, written: AtomicU32::new(0) }
    }

    /// Makes this the panic hook of the process. The hook before it still runs, so the
    /// panic also shows on the terminal.
    pub fn install(self) {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            self.write(info);
            before(info);
        }));
    }

    fn write(&self, info: &PanicHookInfo<'_>) {
        let number = self.written.fetch_add(1, Ordering::Relaxed);
        if number >= RUN_LIMIT {
            return;
        }
        let thread = std::thread::current();
        let facts = Facts {
            version: self.version,
            thread: thread.name().unwrap_or("with no name"),
            place: info.location().map_or_else(|| "a place that is not known".to_owned(), ToString::to_string),
            message: message(info),
        };
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
        let file = self.folder.join(format!("{PREFIX}{since_epoch}-{number}.{EXTENSION}"));
        if let Err(error) = std::fs::write(&file, facts.report(&Backtrace::force_capture().to_string())) {
            eprintln!("cannot write the panic report {}: {error}", file.display());
        }
    }
}

/// The text of the panic. Code of the app writes it, and that code puts no secret into it.
fn message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let text = payload.downcast_ref::<&str>().map(|text| (*text).to_owned());
    text.or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic with no text".to_owned())
}

/// What one report tells.
struct Facts<'a> {
    version: &'a str,
    thread: &'a str,
    place: String,
    message: String,
}

impl Facts<'_> {
    fn report(&self, backtrace: &str) -> String {
        format!(
            "fresnel {}\nthread: {}\nplace: {}\nmessage: {}\n\nbacktrace:\n{backtrace}\n",
            self.version, self.thread, self.place, self.message
        )
    }
}

/// The report files of a folder, the oldest first. The name of a file starts with its time.
pub fn list(folder: &Path) -> Vec<PathBuf> {
    let is_report = |path: &PathBuf| {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        name.starts_with(PREFIX) && path.extension().is_some_and(|extension| extension == EXTENSION)
    };
    let entries = std::fs::read_dir(folder).into_iter().flatten().flatten();
    let mut reports: Vec<PathBuf> = entries.map(|entry| entry.path()).filter(is_report).collect();
    reports.sort();
    reports
}

/// Removes the oldest reports of earlier runs, so that the newest `KEEP` stay. Returns the
/// count of reports that stay.
pub fn prune(folder: &Path) -> usize {
    let reports = list(folder);
    let excess = reports.len().saturating_sub(KEEP);
    for old in reports.iter().take(excess) {
        if let Err(error) = std::fs::remove_file(old) {
            eprintln!("cannot remove the old panic report {}: {error}", old.display());
        }
    }
    reports.len() - excess
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Scratch;

    #[test]
    fn a_report_has_the_version_the_thread_the_place_and_the_message() {
        let facts = Facts {
            version: "0.1.0",
            thread: "main",
            place: "crates/fresnel-ui/src/shell/mod.rs:61:9".to_owned(),
            message: "index out of bounds: the len is 9 but the index is 9".to_owned(),
        };
        let expected = "\
fresnel 0.1.0
thread: main
place: crates/fresnel-ui/src/shell/mod.rs:61:9
message: index out of bounds: the len is 9 but the index is 9

backtrace:
0: fresnel::main
";
        assert_eq!(facts.report("0: fresnel::main"), expected);
    }

    #[test]
    fn only_the_newest_reports_of_earlier_runs_stay() {
        let folder = Scratch::new("panic-prune");
        for number in 0..13 {
            std::fs::write(folder.path().join(format!("panic-17000000000{number:02}-0.txt")), "x").expect("a report");
        }
        std::fs::write(folder.path().join("fresnel.log"), "x").expect("some other file");
        assert_eq!(list(folder.path()).len(), 13);
        assert_eq!(prune(folder.path()), 10);
        let files = folder.files();
        assert_eq!(files.len(), 11, "{files:?}");
        assert_eq!((files[0].as_str(), files[1].as_str()), ("fresnel.log", "panic-1700000000003-0.txt"));
        assert_eq!(prune(&folder.path().join("absent")), 0);
    }
}
