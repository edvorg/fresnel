//! The throttle of the server: HTTP 429 with `Retry-After` (plan section 8.11).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use http::header::RETRY_AFTER;
use http::{HeaderMap, StatusCode};
use tokio::time::{Instant, sleep_until};

use crate::changes::Changed;
use crate::clock::WallClock;
use crate::error::{ErrorClass, NetError};

/// The settings of the throttle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThrottleSettings {
    /// The wait after an HTTP 429 with no usable `Retry-After` header.
    pub default_wait: Duration,
    /// The longest wait that the app obeys. A larger `Retry-After` counts as this value.
    pub longest_wait: Duration,
    /// How many times one request goes out again after HTTP 429.
    pub repeats: u32,
}

impl Default for ThrottleSettings {
    fn default() -> Self {
        Self { default_wait: Duration::from_secs(1), longest_wait: Duration::from_secs(30), repeats: 3 }
    }
}

/// A copy of the throttle facts of one cluster.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThrottleStatus {
    /// The server takes no request until this time. `None` when it takes requests.
    pub until: Option<SystemTime>,
    /// The count of HTTP 429 answers.
    pub answers: u64,
}

pub(crate) struct Throttle {
    settings: ThrottleSettings,
    clock: WallClock,
    /// The end of the wait, on the monotonic clock for the wait and on the wall clock for the UI.
    until: Mutex<Option<(Instant, SystemTime)>>,
    answers: AtomicU64,
    changed: Changed,
}

impl Throttle {
    pub(crate) fn new(settings: ThrottleSettings, clock: WallClock, changed: Changed) -> Self {
        Self { settings, clock, until: Mutex::new(None), answers: AtomicU64::new(0), changed }
    }

    /// Waits until the server takes requests again. A request whose time limit ends before
    /// that fails at once: its wait would be lost time.
    pub(crate) async fn gate(&self, deadline: Instant) -> Result<(), NetError> {
        let Some(until) = self.until().map(|(until, _)| until) else {
            return Ok(());
        };
        if until > deadline {
            let left = until.saturating_duration_since(Instant::now());
            let text = format!("the server throttles requests for {left:?} more (HTTP 429)");
            return Err(NetError::new(ErrorClass::Server, text));
        }
        sleep_until(until).await;
        Ok(())
    }

    /// Reads the head of an answer. For HTTP 429 it records the wait that the server asks
    /// for and gives `true`: the request can go out again after the gate.
    pub(crate) fn note(&self, status: StatusCode, headers: &HeaderMap) -> bool {
        if status != StatusCode::TOO_MANY_REQUESTS {
            return false;
        }
        let asked =
            headers.get(RETRY_AFTER).and_then(|value| value.to_str().ok()).and_then(|text| text.trim().parse().ok());
        let wait = asked.map_or(self.settings.default_wait, Duration::from_secs).min(self.settings.longest_wait);
        let until = (Instant::now() + wait, self.clock.now() + wait);
        {
            let mut current = self.until.lock().unwrap_or_else(PoisonError::into_inner);
            // A second answer with a shorter wait does not shorten the wait of the first.
            if current.is_none_or(|(current, _)| current < until.0) {
                *current = Some(until);
            }
        }
        self.answers.fetch_add(1, Ordering::Relaxed);
        self.changed.notify();
        true
    }

    pub(crate) fn repeats(&self) -> u32 {
        self.settings.repeats
    }

    pub(crate) fn status(&self) -> ThrottleStatus {
        ThrottleStatus { until: self.until().map(|(_, wall)| wall), answers: self.answers.load(Ordering::Relaxed) }
    }

    /// The end of the wait, if it is in the future.
    fn until(&self) -> Option<(Instant, SystemTime)> {
        let mut until = self.until.lock().unwrap_or_else(PoisonError::into_inner);
        if until.is_some_and(|(until, _)| until <= Instant::now()) {
            *until = None;
        }
        *until
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;
    use crate::clock::testing::{epoch, paused_clock};

    const SECOND: Duration = Duration::from_secs(1);

    fn throttle() -> Throttle {
        Throttle::new(ThrottleSettings::default(), paused_clock(), Changed::new())
    }

    fn retry_after(value: &'static str) -> HeaderMap {
        HeaderMap::from_iter([(RETRY_AFTER, HeaderValue::from_static(value))])
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_waits_for_the_time_that_the_server_asks_for() {
        let throttle = throttle();
        assert!(!throttle.note(StatusCode::OK, &retry_after("5")), "only HTTP 429 is a throttle");
        assert!(throttle.note(StatusCode::TOO_MANY_REQUESTS, &retry_after("5")));
        assert!(throttle.note(StatusCode::TOO_MANY_REQUESTS, &retry_after("2")));
        assert_eq!(throttle.status(), ThrottleStatus { until: Some(epoch() + 5 * SECOND), answers: 2 });

        let started = Instant::now();
        throttle.gate(started + 10 * SECOND).await.expect("the wait fits into the limit");
        assert_eq!(started.elapsed(), 5 * SECOND);
        assert_eq!(throttle.status().until, None);
        throttle.gate(started).await.expect("no wait");
        assert_eq!(started.elapsed(), 5 * SECOND);
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_with_a_shorter_limit_fails_at_once() {
        let throttle = throttle();
        throttle.note(StatusCode::TOO_MANY_REQUESTS, &retry_after("20"));
        let started = Instant::now();
        let error = throttle.gate(started + 10 * SECOND).await.expect_err("the wait is longer than the limit");
        assert_eq!((error.class(), started.elapsed()), (ErrorClass::Server, Duration::ZERO));
        assert_eq!(error.to_string(), "the server throttles requests for 20s more (HTTP 429)");
    }

    #[tokio::test(start_paused = true)]
    async fn a_bad_header_gives_the_default_wait_and_a_long_wait_has_a_limit() {
        for (headers, wait) in
            [(HeaderMap::new(), 1), (retry_after("soon"), 1), (retry_after("86400"), 30), (retry_after(" 3 "), 3)]
        {
            let throttle = throttle();
            throttle.note(StatusCode::TOO_MANY_REQUESTS, &headers);
            assert_eq!(throttle.status().until, Some(epoch() + wait * SECOND), "{headers:?}");
        }
    }
}
