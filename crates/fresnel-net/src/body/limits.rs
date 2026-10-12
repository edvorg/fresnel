//! The time limits of a response body (plan section 8.10).

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::time::{Instant, Sleep, sleep, sleep_until};

use crate::error::NetError;

/// The limits of one body: the time with no data, and the time for the full answer.
pub(crate) struct BodyLimits {
    idle: Option<(Duration, Pin<Box<Sleep>>)>,
    total: Option<(Duration, Pin<Box<Sleep>>)>,
}

impl BodyLimits {
    /// `total` counts from `started`, the start of the request. `None` with no limit.
    pub(crate) fn new(idle: Option<Duration>, total: Option<Duration>, started: Instant) -> Option<Self> {
        if idle.is_none() && total.is_none() {
            return None;
        }
        Some(Self {
            idle: idle.map(|idle| (idle, Box::pin(sleep(idle)))),
            total: total.map(|total| (total, Box::pin(sleep_until(started + total)))),
        })
    }

    /// A piece of the body arrived: the time with no data starts again.
    pub(super) fn data_arrived(&mut self) {
        if let Some((idle, timer)) = &mut self.idle {
            timer.as_mut().reset(Instant::now() + *idle);
        }
    }

    /// The error of a limit that passed. With `None`, the timers wake the reader later.
    pub(super) fn poll_passed(&mut self, cx: &mut Context<'_>) -> Option<NetError> {
        if let Some((total, timer)) = &mut self.total
            && let Poll::Ready(()) = timer.as_mut().poll(cx)
        {
            return Some(NetError::timeout(format!("the answer was not complete in {total:?}")));
        }
        if let Some((idle, timer)) = &mut self.idle
            && let Poll::Ready(()) = timer.as_mut().poll(cx)
        {
            return Some(NetError::timeout(format!("the answer had no data for {idle:?}")));
        }
        None
    }
}
