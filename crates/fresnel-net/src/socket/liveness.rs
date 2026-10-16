//! When a socket sends a ping, and when a silence is a break (plan sections 8.5 and 16).
//!
//! A ping goes out after `ping_after` with no frame, and each frame answers it: a silent
//! connection breaks `ping_after + answer_limit` after its last frame at most. The limit of a
//! ping counts from the moment its bytes are out, so a slow write does not shorten it. A ping
//! behind a write that the peer does not take can get no answer: then the path decides
//! (`Due::AskPath`), and the socket decides the same for a ping behind data that the peer
//! may not have read (`run`). While the cluster answers on a channel, a peer that reads
//! nothing is a slow pod, which is no break.
//!
//! A full queue of the caller stops the read of the socket, so no frame can arrive. The count
//! of the silence starts again when the read goes on.

use std::time::{Duration, SystemTime};

use tokio::time::Instant;

/// What the socket must do at its deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Due {
    /// Send a ping now.
    Ping,
    /// No frame answered the ping in its limit: the connection is silent.
    Silent,
    /// The ping still waits behind a write. A live path gives it one more limit.
    AskPath,
}

#[derive(Clone, Copy, Debug)]
struct Ping {
    queued: Instant,
    /// When the socket took the last byte of the ping. `None` while a write before it waits.
    written: Option<Instant>,
    limit: Duration,
}

impl Ping {
    fn deadline(&self) -> Instant {
        self.written.unwrap_or(self.queued) + self.limit
    }
}

#[derive(Debug)]
pub(super) struct Liveness {
    ping_after: Duration,
    answer_limit: Duration,
    /// The last frame, or the end of a pause of the read.
    heard: Instant,
    /// The same moment on the wall clock, which the states of the channels have.
    heard_at: SystemTime,
    ping: Option<Ping>,
    paused: bool,
    /// A live path let the ping wait behind a write, so a change of the path decides again.
    on_path: bool,
}

impl Liveness {
    pub(super) fn new(now: Instant, ping_after: Duration, answer_limit: Duration) -> Self {
        Self {
            ping_after,
            answer_limit,
            heard: now,
            heard_at: SystemTime::now(),
            ping: None,
            paused: false,
            on_path: false,
        }
    }

    /// A frame arrived. It answers each ping.
    pub(super) fn heard(&mut self, now: Instant) {
        self.heard = now;
        self.heard_at = SystemTime::now();
        self.ping = None;
        self.on_path = false;
    }

    /// The read stops while the queue of the caller is full.
    pub(super) fn pause(&mut self) {
        self.paused = true;
    }

    /// The read goes on.
    pub(super) fn resume(&mut self, now: Instant) {
        self.paused = false;
        self.heard(now);
    }

    /// A ping with the answer limit went into the queue of the socket.
    pub(super) fn ping_queued(&mut self, now: Instant) {
        self.ping = Some(Ping { queued: now, written: None, limit: self.answer_limit });
    }

    /// After a wake or a network change: true when a test ping with `limit` must go out now.
    /// A ping whose bytes are out keeps its own deadline, which is near.
    pub(super) fn test(&mut self, now: Instant, limit: Duration) -> bool {
        if self.paused || self.ping.is_some_and(|ping| ping.written.is_some()) {
            return false;
        }
        self.ping = Some(Ping { queued: now, written: None, limit });
        true
    }

    /// Each byte that waited is out, the ping too.
    pub(super) fn written(&mut self, now: Instant) {
        if let Some(ping) = &mut self.ping
            && ping.written.is_none()
        {
            ping.written = Some(now);
            self.on_path = false;
        }
    }

    /// The time of the next decision. `None` while the read pauses.
    pub(super) fn deadline(&self) -> Option<Instant> {
        if self.paused {
            return None;
        }
        Some(self.ping.map_or(self.heard + self.ping_after, |ping| ping.deadline()))
    }

    /// What is due at `now`, if anything.
    pub(super) fn due(&self, now: Instant) -> Option<Due> {
        if now < self.deadline()? {
            return None;
        }
        Some(match self.ping {
            None => Due::Ping,
            Some(Ping { written: Some(_), .. }) => Due::Silent,
            Some(Ping { written: None, .. }) => Due::AskPath,
        })
    }

    /// The cluster answers: the ping waits one more limit for a peer that reads nothing.
    pub(super) fn path_live(&mut self, now: Instant) {
        if let Some(ping) = &mut self.ping {
            ping.queued = now;
            ping.written = ping.written.map(|_| now);
            self.on_path = true;
        }
    }

    /// True while a change of the path decides about a ping with no answer.
    pub(super) fn on_path(&self) -> bool {
        self.on_path
    }

    /// The time since the last frame.
    pub(super) fn quiet(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.heard)
    }

    /// The wall clock time of the last frame.
    pub(super) fn heard_at(&self) -> SystemTime {
        self.heard_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::socket::SocketSettings;

    const S: Duration = Duration::from_secs(1);

    fn liveness(start: Instant) -> Liveness {
        let settings = SocketSettings::default();
        Liveness::new(start, settings.ping_after, settings.answer_limit)
    }

    #[test]
    fn the_defaults_find_a_silent_connection_in_15_s_which_is_below_the_20_s_of_the_plan() {
        assert_eq!(SocketSettings::default().longest_silence(), 15 * S);
        let start = Instant::now();
        let mut live = liveness(start);
        assert_eq!((live.deadline(), live.due(start + 9 * S)), (Some(start + 10 * S), None));
        assert_eq!(live.due(start + 10 * S), Some(Due::Ping));
        live.ping_queued(start + 10 * S);
        live.written(start + 10 * S);
        assert_eq!(live.due(start + 14 * S), None);
        assert_eq!(live.due(start + 15 * S), Some(Due::Silent));
        assert_eq!(live.quiet(start + 15 * S), 15 * S);
    }

    #[test]
    fn each_frame_answers_the_ping_and_moves_the_next_one() {
        let start = Instant::now();
        let mut live = liveness(start);
        live.ping_queued(start + 10 * S);
        live.written(start + 10 * S);
        live.heard(start + 12 * S);
        assert_eq!(live.deadline(), Some(start + 22 * S), "a data frame answers the ping as a pong does");
        live.heard(start + 20 * S);
        assert_eq!(live.deadline(), Some(start + 30 * S), "frames keep a busy connection with no ping");
    }

    /// The limit of the ping counts from the end of its write, and a ping behind a write that
    /// waits asks the path.
    #[test]
    fn a_ping_behind_a_waiting_write_asks_the_path_each_limit() {
        let start = Instant::now();
        let mut live = liveness(start);
        live.ping_queued(start + 10 * S);
        assert_eq!(live.due(start + 15 * S), Some(Due::AskPath));
        assert!(!live.on_path());
        live.path_live(start + 15 * S);
        assert!(live.on_path(), "a change of the path decides again");
        assert_eq!((live.due(start + 19 * S), live.due(start + 20 * S)), (None, Some(Due::AskPath)));
        live.path_live(start + 20 * S);
        live.written(start + 23 * S);
        assert!(!live.on_path());
        assert_eq!(live.deadline(), Some(start + 28 * S), "the limit counts from the write");
        assert_eq!(live.due(start + 28 * S), Some(Due::Silent));
        live.path_live(start + 28 * S);
        assert_eq!((live.on_path(), live.deadline()), (true, Some(start + 33 * S)), "a written ping waits too");
    }

    #[test]
    fn a_full_queue_of_the_caller_pauses_the_count_of_the_silence() {
        let start = Instant::now();
        let mut live = liveness(start);
        live.ping_queued(start + 10 * S);
        live.written(start + 10 * S);
        live.pause();
        assert_eq!((live.deadline(), live.due(start + 60 * S)), (None, None));
        live.resume(start + 60 * S);
        assert_eq!(live.deadline(), Some(start + 70 * S), "the resume counts as a frame");
    }

    #[test]
    fn a_wake_sends_a_test_ping_unless_a_written_ping_waits() {
        let start = Instant::now();
        let mut live = liveness(start);
        assert!(live.test(start + 2 * S, 3 * S));
        live.written(start + 2 * S);
        assert_eq!(live.deadline(), Some(start + 5 * S), "the short limit of a test");
        assert!(!live.test(start + 3 * S, 3 * S), "a written ping keeps its deadline");
        assert_eq!(live.deadline(), Some(start + 5 * S));
        live.pause();
        assert!(!live.test(start + 4 * S, 3 * S), "no answer can arrive while the read pauses");
    }
}
