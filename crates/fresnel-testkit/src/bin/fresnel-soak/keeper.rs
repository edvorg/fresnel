//! The test environment of the soak. k3s itself can fail: in a 24-hour soak, its API server
//! panicked in the restart of a fault and stayed down for 38 minutes. That time is an outage
//! of the environment, not a fault of the app. The keeper looks at the API server between
//! the faults, restores one that does not answer (a restart of the container, then a recreate
//! of the cluster), and writes the outage into the file of the faults:
//!
//! `outage from <seconds>: <text>` and `outage to <seconds>: <text>`, with the seconds of the
//! wall clock since 1970. `fresnel-cli soak` reports the minutes of an outage apart.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fresnel_testkit::K3s;
use tokio::time::{Instant, sleep};

use super::faults::note_line;
use super::scene;

/// The longest time with no answer of the API server before the keeper restores it. A
/// restart of the API server answers again in 10 to 75 s.
const DOWN_LIMIT: Duration = Duration::from_secs(150);
/// The time between two looks at the API server.
const LOOK_PERIOD: Duration = Duration::from_secs(10);
/// The recreates after a restart that did not bring the API server back. A k3s on a busy
/// machine panicked at its start with a slow datastore three times in a row, and a later
/// start worked.
const RECREATES: u32 = 3;

pub(crate) struct Keeper<'a> {
    k3s: &'a K3s,
    /// The pause of the lines of the logger, for a scene after a recreate.
    line_pause: &'static str,
    /// The last time of the wall clock with an answer of the API server.
    last_answer: SystemTime,
    pub(crate) outages: u64,
}

impl<'a> Keeper<'a> {
    pub(crate) fn new(k3s: &'a K3s, line_pause: &'static str) -> Self {
        Self { k3s, line_pause, last_answer: SystemTime::now(), outages: 0 }
    }

    /// Waits for `time`, and looks at the API server each `LOOK_PERIOD`. A server with no
    /// answer for `DOWN_LIMIT` gets restored, and the wait goes on after it.
    pub(crate) async fn wait(&mut self, file: &std::path::Path, time: Duration) -> Result<(), String> {
        let end = Instant::now() + time;
        while Instant::now() < end {
            sleep(LOOK_PERIOD.min(end.saturating_duration_since(Instant::now()))).await;
            if self.answers().await {
                self.last_answer = SystemTime::now();
            } else if self.last_answer.elapsed().unwrap_or_default() >= DOWN_LIMIT {
                self.restore(file).await?;
            }
        }
        Ok(())
    }

    /// Takes the end of a fault of the harness: a server that came back answered now, and
    /// one that did not gets restored.
    pub(crate) async fn after_fault(&mut self, file: &std::path::Path, came_back: bool) -> Result<(), String> {
        if came_back {
            self.last_answer = SystemTime::now();
            return Ok(());
        }
        self.restore(file).await
    }

    async fn answers(&self) -> bool {
        self.k3s.kubectl(&["get", "--raw", "/readyz", "--request-timeout=5s"]).await.is_ok()
    }

    /// The outage starts at the last answer. A restart of the container comes first; a
    /// cluster that does not start again gets a recreate, and its first objects again, up to
    /// `RECREATES` times.
    async fn restore(&mut self, file: &std::path::Path) -> Result<(), String> {
        self.outages += 1;
        let since = seconds(self.last_answer);
        note_line(file, &format!("outage from {since}: the API server of k3s does not answer; a restart follows"));
        let mut failed = match self.k3s.restart().await {
            Ok(()) => None,
            Err(error) => Some(format!("the restart did not bring the API server back: {error}")),
        };
        let mut recreates = 0;
        while let Some(error) = failed.take() {
            println!("fresnel-soak: {error}");
            if recreates == RECREATES {
                return Err(error);
            }
            recreates += 1;
            failed = self.k3s.recreate().await.err().map(|error| format!("recreate {recreates} failed: {error}"));
        }
        if recreates > 0 {
            scene::create(self.k3s, self.line_pause).await?;
        }
        let how = match recreates {
            0 => "a restart of the container".to_owned(),
            1 => "a recreate of the cluster".to_owned(),
            _ => format!("{recreates} recreates of the cluster"),
        };
        self.last_answer = SystemTime::now();
        let to = seconds(self.last_answer);
        note_line(file, &format!("outage to {to}: the API server answers after {how}"));
        Ok(())
    }
}

/// The seconds of a time of the wall clock since 1970.
fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}
