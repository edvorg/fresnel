//! The task of one save: it makes a new file, writes the lines or the streams into it in
//! pieces on a thread for blocking calls, and tells its state after each piece.

use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::watch;

use super::{LogSave, SaveState, SaveView, text};
use crate::error::CoreError;
use crate::logs::request::Origin;
use crate::logs::ring::Lines;
use crate::logs::source::{Chunks, Opener};
use crate::logs::view::SourceView;
use crate::published::Publisher;

/// The bytes that the task collects before one write: few writes, and a small buffer.
const PIECE: usize = 1024 * 1024;
/// The most names that a save tries when files of its name exist.
const NAMES: u32 = 100;

/// What the file of a save gets.
#[derive(Clone)]
pub(super) enum Content {
    /// The lines of the view, with its sources for the prefix of each line.
    Buffer { lines: Lines, sources: Arc<[SourceView]> },
    /// The full log of each source of the view, of the instance before the last restart
    /// with `previous`.
    Full { sources: Arc<[SourceView]>, previous: bool },
}

/// One save.
#[derive(Clone)]
pub(super) struct Job<O> {
    pub(super) save: LogSave,
    pub(super) content: Content,
    pub(super) opener: O,
    pub(super) folder: PathBuf,
}

/// How the writes of a save ended, when no write failed.
enum Ended {
    /// Each part is in the file. The sources that the server did not give, with why.
    Complete(Vec<String>),
    Cancelled,
}

/// Runs one save. `at` is the time in the name of its file.
pub(super) async fn run<O: Opener>(
    job: Job<O>,
    at: SystemTime,
    publisher: Publisher<SaveView>,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), CoreError> {
    let first = SaveView::clone(&publisher.read());
    let mut report = Report { publisher, view: first, ended: false };
    let (folder, save) = (job.folder.clone(), job.save.clone());
    let (file, path) = match blocking(move || create(&folder, &save, at)).await.and_then(|made| made.map_err(fault)) {
        Ok(made) => made,
        Err(error) => {
            report.end(SaveState::Failed(error));
            return Ok(());
        }
    };
    report.file(path.clone());
    let mut sink = Sink { file: Some(file), pending: Vec::new(), bytes: 0 };
    let times = job.save.times;
    let ended = match &job.content {
        Content::Buffer { lines, sources } => buffer(&mut sink, lines, sources, times, &cancel, &mut report).await,
        Content::Full { sources, previous } => {
            let stream = Stream { opener: &job.opener, previous: *previous, times };
            full(&mut sink, &stream, sources, &mut cancel, &mut report).await
        }
    };
    let ended = match ended {
        Ok(Ended::Complete(skipped)) => sink.flush().await.map(|()| Ended::Complete(skipped)),
        other => other,
    };
    match ended {
        Ok(Ended::Complete(skipped)) => report.end(SaveState::Done { bytes: sink.bytes, skipped }),
        Ok(Ended::Cancelled) => match remove(path).await {
            Ok(()) => report.end(SaveState::Cancelled),
            Err(stays) => report.end(SaveState::Failed(stays)),
        },
        Err(error) => {
            let error = match remove(path).await {
                Ok(()) => error,
                Err(stays) => CoreError::invalid(format!("{error}; {stays}")),
            };
            report.end(SaveState::Failed(error));
        }
    }
    Ok(())
}

/// Writes the lines of the view.
async fn buffer(
    sink: &mut Sink,
    lines: &Lines,
    sources: &[SourceView],
    times: bool,
    cancel: &watch::Receiver<bool>,
    report: &mut Report,
) -> Result<Ended, CoreError> {
    let many = sources.len() > 1;
    let mut text = String::new();
    for (_, line) in lines.range(lines.first(), lines.end()) {
        let source = usize::try_from(line.source).ok().and_then(|index| sources.get(index)).filter(|_| many);
        text::line(&mut text, line, source, times);
        if text.len() >= PIECE {
            if *cancel.borrow() {
                return Ok(Ended::Cancelled);
            }
            sink.add(text.as_bytes()).await?;
            text.clear();
            report.bytes(sink.bytes);
        }
    }
    sink.add(text.as_bytes()).await?;
    Ok(Ended::Complete(Vec::new()))
}

/// Where the full logs come from.
struct Stream<'a, O> {
    opener: &'a O,
    previous: bool,
    times: bool,
}

/// Writes the full log of each source, one after the other. A source that the server does
/// not give gets a line in the file and a note, and the next source follows.
async fn full<O: Opener>(
    sink: &mut Sink,
    stream: &Stream<'_, O>,
    sources: &[SourceView],
    cancel: &mut watch::Receiver<bool>,
    report: &mut Report,
) -> Result<Ended, CoreError> {
    let mut skipped = Vec::new();
    for source in sources {
        if sources.len() > 1 {
            sink.add(text::head(source).as_bytes()).await?;
        }
        let origin = Origin::Whole { times: stream.times };
        let opened = tokio::select! {
            () = cancelled(cancel) => return Ok(Ended::Cancelled),
            opened = stream.opener.open(&source.pod, &source.container, stream.previous, origin) => opened,
        };
        let mut chunks = match opened {
            Ok(chunks) => chunks,
            Err(error) => {
                sink.add(format!("--- the server gave no log: {error}\n").as_bytes()).await?;
                skipped.push(format!("{}/{}: {error}", source.pod.name, source.container));
                continue;
            }
        };
        loop {
            let chunk = tokio::select! {
                () = cancelled(cancel) => return Ok(Ended::Cancelled),
                chunk = chunks.next() => chunk,
            };
            match chunk {
                Some(Ok(data)) => {
                    let before = sink.bytes;
                    sink.add(&data).await?;
                    if sink.bytes != before {
                        report.bytes(sink.bytes);
                    }
                }
                Some(Err(error)) => {
                    sink.add(format!("\n--- the stream broke: {error}\n").as_bytes()).await?;
                    skipped.push(format!("{}/{}: {error}", source.pod.name, source.container));
                    break;
                }
                None => break,
            }
        }
    }
    Ok(Ended::Complete(skipped))
}

/// Ends when the user stops the save, or when no one can stop it any more.
async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

/// The file of a save with the bytes that wait for their write.
struct Sink {
    /// `None` only during a write.
    file: Option<File>,
    pending: Vec<u8>,
    /// The bytes in the file.
    bytes: u64,
}

impl Sink {
    async fn add(&mut self, data: &[u8]) -> Result<(), CoreError> {
        self.pending.extend_from_slice(data);
        if self.pending.len() >= PIECE { self.flush().await } else { Ok(()) }
    }

    /// Writes the bytes that wait. The file and the buffer go to the thread and come back.
    async fn flush(&mut self) -> Result<(), CoreError> {
        let mut file = self.file.take().ok_or_else(|| CoreError::invalid("the file of the save is gone"))?;
        let data = std::mem::take(&mut self.pending);
        let (file, mut data, written) = blocking(move || {
            let written = file.write_all(&data);
            (file, data, written)
        })
        .await?;
        written.map_err(fault)?;
        self.bytes += data.len() as u64;
        data.clear();
        (self.file, self.pending) = (Some(file), data);
        Ok(())
    }
}

/// The state of a save for the view. A run that ends with no final state removes its file
/// and tells it at its drop: after a panic, or at a stop of its cluster or of the app, which
/// ends the task between two steps. A part of a log would look like a full one.
struct Report {
    publisher: Publisher<SaveView>,
    view: SaveView,
    ended: bool,
}

impl Report {
    fn file(&mut self, path: PathBuf) {
        self.view.file = Some(path);
        self.publisher.publish(self.view.clone());
    }

    fn bytes(&mut self, bytes: u64) {
        self.view.state = SaveState::Writing { bytes };
        self.publisher.publish(self.view.clone());
    }

    fn end(&mut self, state: SaveState) {
        (self.view.state, self.ended) = (state, true);
        self.publisher.publish(self.view.clone());
    }
}

impl Drop for Report {
    /// The remove runs on the thread of the drop, which cannot wait for a thread for blocking
    /// calls: it is one call, and only after a fault.
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        let removed = self.view.file.as_ref().map_or(Ok(()), std::fs::remove_file);
        let file = match removed {
            Err(error) if error.kind() != io::ErrorKind::NotFound => format!("the incomplete file stays: {error}"),
            Ok(()) | Err(_) => "its file is gone".to_owned(),
        };
        let error = CoreError::invalid(format!("the save stopped before its end; {file}"));
        self.view.state = SaveState::Failed(error);
        self.publisher.publish(self.view.clone());
    }
}

/// Makes the folder and a new file in it with a name that no file has.
fn create(folder: &Path, save: &LogSave, at: SystemTime) -> io::Result<(File, PathBuf)> {
    std::fs::create_dir_all(folder)?;
    let mut attempt = 0;
    loop {
        let path = folder.join(text::file_name(save, at, attempt));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempt + 1 < NAMES => attempt += 1,
            Err(error) => return Err(error),
        }
    }
}

/// Removes the file of a save that did not complete: a part of a log looks like a full one.
async fn remove(path: PathBuf) -> Result<(), CoreError> {
    let removed = blocking(move || std::fs::remove_file(path)).await?;
    removed.map_err(|error| CoreError::invalid(format!("the incomplete file stays: {error}")))
}

/// Runs a call of the file system on a thread for blocking calls.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, CoreError> {
    let done = tokio::task::spawn_blocking(work).await;
    done.map_err(|error| CoreError::invalid(format!("a write of a file did not end: {error}")))
}

fn fault(error: io::Error) -> CoreError {
    CoreError::invalid(format!("the file cannot be written: {error}"))
}
