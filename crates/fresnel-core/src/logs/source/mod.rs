//! The task of one source: one container of one pod, one stream after the other (plan
//! section 12.7).
//!
//! A stream reads the log of the instance that the server gives. At its end, after a break
//! or after a refusal, the source has a state, and the pod in its store tells what comes
//! next. A later stream starts at the second of the latest line, drops the lines that come
//! two times, and puts a marker line into the view. [`Memory`] outlives a run of the task,
//! so a run after a panic starts from the latest line too.
//!
//! - `look`: the next step from the state of the pod. `open`: where the streams come from.

mod look;
mod open;

use std::future::pending;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use fresnel_net::{ErrorClass, Listener, Signal, WallClock};
use k8s_openapi::api::core::v1::Pod;
use tokio::sync::mpsc;
use tokio::time::sleep;

pub(super) use open::{Chunks, Opener};

use super::ingest::Ingest;
use super::line::{LineKind, LogLine};
use super::request::Origin;
use super::view::SourceState;
use crate::cluster::ClusterId;
use crate::error::CoreError;
use crate::published::Reader;
use crate::store::{ObjectKey, Snapshot};
use look::{Look, look};

/// One container of one pod in a view.
#[derive(Clone, Debug)]
pub(super) struct Stream {
    /// The index of the source in its view.
    pub(super) index: u32,
    pub(super) pod: ObjectKey,
    /// The UID of the pod. Empty for any pod of the name.
    pub(super) uid: Arc<str>,
    pub(super) container: Arc<str>,
}

/// What a source tells its view, in the sequence of its stream.
#[derive(Debug)]
pub(super) enum Report {
    /// New lines, with the count of lines that the streams sent two times.
    Lines { source: u32, lines: Vec<LogLine>, repeated: u64 },
    /// A new state.
    State { source: u32, state: SourceState },
}

/// What one source needs.
pub(super) struct SourceParts<O> {
    pub(super) stream: Stream,
    /// True for the log of the instance before the last restart.
    pub(super) previous: bool,
    /// Where the first stream starts: the options of the view.
    pub(super) first: Origin,
    pub(super) opener: O,
    /// The pods of the cluster.
    pub(super) pods: Reader<Snapshot<Pod>>,
    /// The signal of the cluster: a wake, a network change or "retry now" ends a pause.
    pub(super) signal: Signal,
    pub(super) reports: mpsc::Sender<Report>,
    /// The pauses before a new stream after failures in sequence.
    pub(super) retry: Backoff,
    pub(super) clock: WallClock,
}

/// What a source knows of its streams before, also after a panic of its task.
struct Memory {
    ingest: Ingest,
    /// The instance of the last stream that opened.
    instance: Option<Arc<str>>,
    /// The instance whose log a stream read to its end.
    complete: Option<Arc<str>>,
    /// Why the next stream follows an earlier one: the text of its marker.
    gap: Option<String>,
    /// True when the last line of the source in the view is a marker: a source that fails
    /// again and again writes one marker for the gap.
    marked: bool,
}

/// Starts the task of one source. It stops when the handle drops.
pub(super) fn start<O: Opener>(
    tasks: &Tasks,
    cluster: &ClusterId,
    restart: Backoff,
    line_bytes: usize,
    parts: SourceParts<O>,
) -> TaskHandle {
    let ingest = Ingest::new(parts.stream.index, line_bytes);
    let memory = Memory { ingest, instance: None, complete: None, gap: None, marked: false };
    let (parts, memory) = (Arc::new(parts), Arc::new(Mutex::new(memory)));
    let mut runs = 0_u64;
    tasks.start(TaskName::scoped("log stream", cluster.as_str()), Restart::Always(restart), move || {
        runs += 1;
        let source = Source::new(Arc::clone(&parts), Arc::clone(&memory));
        source.run(runs > 1)
    })
}

/// One run of the task of a source.
struct Source<O> {
    parts: Arc<SourceParts<O>>,
    memory: Arc<Mutex<Memory>>,
    pods: Reader<Snapshot<Pod>>,
    signal: Listener,
    /// The count of failures in sequence. A stream that gives data ends the sequence.
    failures: u32,
}

impl<O: Opener> Source<O> {
    fn new(parts: Arc<SourceParts<O>>, memory: Arc<Mutex<Memory>>) -> Self {
        let (pods, signal) = (parts.pods.clone(), parts.signal.listen());
        Self { parts, memory, pods, signal, failures: 0 }
    }

    async fn run(mut self, restarted: bool) -> Result<(), CoreError> {
        if restarted {
            let text = "the stream started again after an internal error of the app";
            self.memory().gap.get_or_insert_with(|| text.to_owned());
        }
        loop {
            let complete = self.memory().complete.clone();
            let parts = &self.parts;
            let next = look(&self.pods.read(), &parts.stream, parts.previous, complete.as_deref());
            let instance = match next {
                Look::Open(instance) => instance,
                Look::Wait(reason) => {
                    self.tell(SourceState::Waiting { reason: reason.into() }).await?;
                    self.pods.changed().await;
                    continue;
                }
                Look::End(state) => {
                    self.tell(state).await?;
                    return pending().await;
                }
            };
            self.tell(SourceState::Connecting).await?;
            let origin = self.origin();
            let Stream { pod, container, .. } = &self.parts.stream;
            match self.parts.opener.open(pod, container, self.parts.previous, origin).await {
                Ok(stream) => self.follow(stream, instance).await?,
                Err(error) => self.refused(error).await?,
            }
        }
    }

    /// Where the next stream starts: the options of the view for the first one, then the
    /// second of the latest line. After a first stream with no line, the full log.
    fn origin(&self) -> Origin {
        let memory = self.memory();
        match (memory.instance.is_some(), memory.ingest.start()) {
            (false, _) => self.parts.first,
            (true, Some(start)) => Origin::Time(start),
            (true, None) => Origin::Whole,
        }
    }

    /// Reads one stream to its end, and waits as its end asks.
    async fn follow(&mut self, mut stream: O::Stream, instance: Arc<str>) -> Result<(), CoreError> {
        let marker = self.begin(&instance);
        self.tell(SourceState::Streaming).await?;
        self.send(marker.into_iter().collect()).await?;
        let broke = loop {
            let mut lines = Vec::new();
            match stream.next().await {
                Some(Ok(data)) => {
                    self.failures = 0;
                    self.memory().ingest.feed(&data, &mut lines);
                }
                Some(Err(error)) => break Some(error),
                None => {
                    self.memory().ingest.finish(&mut lines);
                    self.send(lines).await?;
                    break None;
                }
            }
            self.send(lines).await?;
        };
        if let Some(error) = broke {
            return self.pause(error, false).await;
        }
        if self.parts.previous {
            let reason = "the view has the full log of the earlier instance".into();
            self.tell(SourceState::Ended { reason }).await?;
            return pending().await;
        }
        self.memory().complete = Some(Arc::clone(&instance));
        let parts = &self.parts;
        let next = look(&self.pods.read(), &parts.stream, parts.previous, Some(&instance));
        if next == Look::Open(instance) {
            // The store can tell the end of the container some time after the end of the
            // stream: a change of the pod ends this pause.
            let reason = CoreError::new(ErrorClass::Network, "the stream ended while the container runs");
            return self.pause(reason, true).await;
        }
        Ok(())
    }

    /// The start of a stream of `instance`: gives the marker of the gap before it.
    fn begin(&self, instance: &Arc<str>) -> Option<LogLine> {
        let mut memory = self.memory();
        memory.ingest.restart();
        if memory.instance.as_ref().is_some_and(|old| old != instance) {
            memory.gap = Some(format!("the container {} started again", self.parts.stream.container));
        }
        memory.instance = Some(Arc::clone(instance));
        memory.complete = None;
        let gap = memory.gap.take().filter(|_| !memory.marked)?;
        memory.marked = true;
        Some(LogLine::marker(memory.ingest.latest().unwrap_or_default(), self.parts.stream.index, gap))
    }

    /// A refusal of the server for a stream.
    async fn refused(&mut self, error: CoreError) -> Result<(), CoreError> {
        match (error.class(), error.status()) {
            // The kubelet refuses the log of a container that waits to start, with its reason.
            (_, Some(400)) => {
                self.failures += 1;
                let pause = self.parts.retry.pause(self.failures, fastrand::u32(..));
                self.tell(SourceState::Waiting { reason: error.to_string().into() }).await?;
                tokio::select! {
                    () = self.pods.changed() => {}
                    () = sleep(pause) => {}
                }
                Ok(())
            }
            // The store tells if the pod is gone, at the next look.
            (_, Some(404)) => self.pause(error, true).await,
            (ErrorClass::Forbidden | ErrorClass::Invalid, _) => {
                self.tell(SourceState::Failed { reason: error }).await?;
                pending().await
            }
            _ => self.pause(error, false).await,
        }
    }

    /// The pause before a new stream after a failure. A change of the pod ends it when
    /// `on_pod_change` is set; the signal of the cluster always ends it.
    async fn pause(&mut self, reason: CoreError, on_pod_change: bool) -> Result<(), CoreError> {
        self.failures += 1;
        let pause = self.parts.retry.pause(self.failures, fastrand::u32(..));
        {
            let mut memory = self.memory();
            if memory.instance.is_some() {
                memory.gap.get_or_insert_with(|| format!("the stream started again after: {reason}"));
            }
        }
        let at = self.parts.clock.now() + pause;
        self.tell(SourceState::Reconnecting { reason, at }).await?;
        let (pods, signal) = (&mut self.pods, &mut self.signal);
        let pod_change = async {
            if on_pod_change {
                pods.changed().await;
            } else {
                pending::<()>().await;
            }
        };
        tokio::select! {
            () = sleep(pause) => {}
            () = pod_change => {}
            _ = signal.raised() => {}
        }
        Ok(())
    }

    async fn tell(&self, state: SourceState) -> Result<(), CoreError> {
        self.report(Report::State { source: self.parts.stream.index, state }).await
    }

    async fn send(&self, lines: Vec<LogLine>) -> Result<(), CoreError> {
        if lines.is_empty() {
            return Ok(());
        }
        let repeated = {
            let mut memory = self.memory();
            memory.marked = lines.last().is_some_and(|line| line.kind == LineKind::Marker);
            memory.ingest.repeated()
        };
        self.report(Report::Lines { source: self.parts.stream.index, lines, repeated }).await
    }

    /// Gives a report to the view. The view stops this task before it drops its end of the
    /// channel, so an error comes only in that stop.
    async fn report(&self, report: Report) -> Result<(), CoreError> {
        let sent = self.parts.reports.send(report).await;
        sent.map_err(|_| CoreError::invalid("the log view of the source is gone"))
    }

    fn memory(&self) -> MutexGuard<'_, Memory> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
