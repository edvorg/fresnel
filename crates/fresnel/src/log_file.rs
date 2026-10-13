//! The log file: each entry of the journal as one line, with rotation by size.
//!
//! The journal in memory has a limit. This file keeps all entries. Each text of the
//! journal is safe for a log, so the file holds no token and no output of a plugin.

use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use fresnel_core::journal::{Entry, Level, Sink};
use jiff::Timestamp;

const NAME: &str = "fresnel";
const EXTENSION: &str = "log";

/// The limits of the log files.
#[derive(Clone, Copy, Debug)]
pub struct Rotation {
    /// A file that would grow above this size starts a new file.
    pub max_bytes: u64,
    /// The count of older files that stay.
    pub keep: usize,
}

impl Default for Rotation {
    /// 5 files of 4 MiB: some days of a session with network faults.
    fn default() -> Self {
        Self { max_bytes: 4 * 1024 * 1024, keep: 4 }
    }
}

/// The open log file. It is the sink of the journal.
#[derive(Debug)]
pub struct LogFile {
    folder: PathBuf,
    rotation: Rotation,
    /// `None` after an error. The next entry opens the file again.
    file: Option<File>,
    size: u64,
    /// Set after an error, to the next line that the file takes.
    lost: Option<Lost>,
}

impl LogFile {
    /// Opens the log file of a folder. New lines go to the end of the file of the last run.
    pub fn open(folder: &Path, rotation: Rotation) -> io::Result<Self> {
        let (file, size) = open(&path(folder, 0))?;
        Ok(Self { folder: folder.to_owned(), rotation, file: Some(file), size, lost: None })
    }

    fn append(&mut self, line: &str) -> io::Result<()> {
        let bytes = line.len() as u64;
        if self.size > 0 && self.size + bytes > self.rotation.max_bytes {
            self.file = None;
            rotate(&self.folder, self.rotation.keep)?;
        }
        let file = match self.file.take() {
            Some(file) => file,
            None => {
                let (file, size) = open(&path(&self.folder, 0))?;
                self.size = size;
                file
            }
        };
        (&file).write_all(line.as_bytes())?;
        self.size += bytes;
        self.file = Some(file);
        Ok(())
    }
}

impl Sink for LogFile {
    fn write(&mut self, entry: &Entry) {
        let mut text = String::new();
        // The note for the entries that an earlier error lost comes before the new entry.
        if let Some(lost) = &self.lost {
            text.push_str(&line(entry.time, Level::Error, "log file", &lost.to_string()));
        }
        text.push_str(&line(entry.time, entry.level, &entry.source, &entry.text));
        match self.append(&text) {
            Ok(()) => self.lost = None,
            Err(error) => {
                let entries = self.lost.as_ref().map_or(0, |lost| lost.entries) + 1;
                self.lost = Some(Lost { entries, error: error.to_string() });
            }
        }
    }

    fn flush(&mut self) {
        if let Some(Err(error)) = self.file.as_ref().map(File::sync_all) {
            let entries = self.lost.as_ref().map_or(0, |lost| lost.entries);
            self.lost = Some(Lost { entries, error: error.to_string() });
        }
    }
}

/// What an error of the file cost. The next line that the file takes tells it (design
/// rule 12: a buffer that drops data shows a counter).
#[derive(Debug)]
struct Lost {
    entries: u64,
    error: String,
}

impl std::fmt::Display for Lost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "an error of the log file ({}); entries that are not in the file: {}", self.error, self.entries)
    }
}

/// The file with this number. 0 is the file that grows; 1 is the newest of the older files.
fn path(folder: &Path, number: usize) -> PathBuf {
    match number {
        0 => folder.join(format!("{NAME}.{EXTENSION}")),
        older => folder.join(format!("{NAME}.{older}.{EXTENSION}")),
    }
}

fn open(path: &Path) -> io::Result<(File, u64)> {
    let file = OpenOptions::new().append(true).create(true).open(path)?;
    let size = file.metadata()?.len();
    Ok((file, size))
}

/// Gives each file the next number. The oldest file goes away.
fn rotate(folder: &Path, keep: usize) -> io::Result<()> {
    for number in (0..keep).rev() {
        match std::fs::rename(path(folder, number), path(folder, number + 1)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            Ok(()) | Err(_) => {}
        }
    }
    if keep == 0 {
        std::fs::remove_file(path(folder, 0))?;
    }
    Ok(())
}

/// One line of the file. A line break in a text shows as `\n`, so one entry is one line.
fn line(time: SystemTime, level: Level, source: &str, text: &str) -> String {
    let time = Timestamp::try_from(time).map_or_else(|_| "-".to_owned(), |time| format!("{time:.3}"));
    let level = match level {
        Level::Info => "INFO ",
        Level::Warning => "WARN ",
        Level::Error => "ERROR",
    };
    let one_line = |text: &str| text.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r");
    format!("{time} {level} [{}] {}\n", one_line(source), one_line(text))
}

#[cfg(test)]
mod tests;
