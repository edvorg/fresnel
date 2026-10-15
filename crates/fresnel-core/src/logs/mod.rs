//! The logs of pods and workloads (plan section 12.7).
//!
//! A view asks for a [`LogRequest`]: the logs of a pod, or of each pod of a workload from
//! the owner index. Its task follows the pods and runs one stream for each container on the
//! log channel of the cluster. Each stream asks for the times of the lines, reads raw bytes
//! with a limit for one line, and parses the escape sequences at once: the text of a line
//! has none, so a search reads plain text. The lines of all streams go into one ring in the
//! sequence of their times. A stream that breaks or ends starts again from the second of
//! its latest line, drops the lines that come two times, and writes a marker line.
//!
//! - `request`: what a view asks for, and the request of one stream. `view`: what it reads.
//! - `session`: the task of a view. `source`: the task of one stream after the other of
//!   one container. `container`: the containers of a view and what the server gives.
//! - `ingest`: from bytes to lines. `merge`: the sequence of many sources. `ring`: the
//!   lines of a view with their limits. `line`: one line with its styles.
//! - `save`: a view as a file: its lines, or the full log of each source.

mod container;
mod ingest;
mod line;
mod merge;
mod request;
mod ring;
mod save;
mod session;
mod source;
#[cfg(test)]
mod testing;
mod view;

use std::time::Duration;

use fresnel_net::task::Backoff;

pub use line::{Color, LineKind, LogLine, Span, Style};
pub use request::{Containers, LogOptions, LogRequest, Since};
pub use ring::{Lines, RingLimits};
pub use save::{LogSave, SavePart, SaveState, SaveView};
pub(crate) use save::{SaveParts, Saves};
pub use session::{LogSession, SessionParts};
pub use view::{LogView, PodsState, SourceState, SourceView};

/// The settings of the log views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogSettings {
    /// The limits of the lines of each view (plan section 12.7).
    pub ring: RingLimits,
    /// The most bytes of one line. A longer line keeps its start: a program can write a
    /// text with no line end, and it must not fill the memory.
    pub line_bytes: usize,
    /// The longest wait of a line for the lines of the other sources of its view, so that
    /// the lines of many pods go into the view in the sequence of their times.
    pub hold: Duration,
    /// The most streams of one view. kubectl has 5 for a selector; a workload of the app
    /// can have more pods, and each stream is one stream of the log channel.
    pub streams: usize,
    /// The pauses before a new stream after failures in sequence: the rule of the watch
    /// loop, so a stream is back as fast as the stores after a fault of the network.
    pub retry: Backoff,
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            ring: RingLimits { lines: 200_000, bytes: 128 * 1024 * 1024 },
            line_bytes: 64 * 1024,
            hold: Duration::from_millis(250),
            streams: 50,
            retry: Backoff::default(),
        }
    }
}
