//! The hedged connect (plan section 8.4).
//!
//! A new connection can get no answer for many seconds. The race starts one more attempt
//! after a short delay and uses the first attempt that connects.

use std::io;
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use tokio::time::{Instant, sleep_until, timeout};

use crate::error::{ErrorClass, NetError, chain};

/// The error texts of the last attempts go into the error of the race.
const KEPT_ERRORS: usize = 4;

/// The numbers of one race.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Race {
    /// The time between the starts of two attempts.
    pub(crate) delay: Duration,
    /// The most attempts that run at the same time.
    pub(crate) max_parallel: usize,
    /// An attempt with no answer ends after this time. A new attempt replaces it.
    pub(crate) attempt_limit: Duration,
    /// The time limit of the full race.
    pub(crate) limit: Duration,
    /// The race ends after this count of fast failures (refused, reset, TLS error). More
    /// attempts do not help then.
    pub(crate) hard_failures: usize,
    /// The most attempts of the race. The race ends when the last one ends.
    pub(crate) max_attempts: usize,
}

/// The attempt that connected first.
#[derive(Debug)]
pub(crate) struct Won<T> {
    pub(crate) io: T,
    /// The count of attempts that started.
    pub(crate) attempts: usize,
    /// The time of the winner from its own start.
    pub(crate) time: Duration,
}

/// Runs attempts until one connects. `attempt` gets the number of the attempt, from 0.
///
/// One timer controls the starts: the next attempt starts `delay` after the start of the
/// attempt before it, also after a fast failure. Thus a server that refuses each connect
/// gets no busy loop.
pub(crate) async fn race<T, F>(plan: Race, mut attempt: impl FnMut(usize) -> F) -> Result<Won<T>, NetError>
where
    F: Future<Output = io::Result<T>>,
{
    let started = Instant::now();
    let deadline = started + plan.limit;
    let mut running = FuturesUnordered::new();
    let mut errors = Vec::new();
    let (mut attempts, mut hard) = (0_usize, 0_usize);
    let mut next_start = started;
    // The class of the error of the race when its last allowed attempt ended.
    let mut spent = ErrorClass::Timeout;
    let class = loop {
        let more = attempts < plan.max_attempts.max(1);
        let free = more && running.len() < plan.max_parallel.max(1);
        if free && Instant::now() >= next_start {
            let (number, begun) = (attempts, Instant::now());
            let connect = timeout(plan.attempt_limit, attempt(number));
            running.push(async move { (number, begun, connect.await) });
            attempts += 1;
            next_start = begun + plan.delay;
        }
        if running.is_empty() && attempts >= plan.max_attempts.max(1) {
            break spent;
        }
        tokio::select! {
            Some((number, begun, result)) = running.next() => match result {
                // The drop of `running` closes the sockets of the other attempts.
                Ok(Ok(io)) => return Ok(Won { io, attempts, time: begun.elapsed() }),
                Ok(Err(error)) => {
                    note(&mut errors, format!("attempt {}: {}", number + 1, chain(&error)));
                    (hard, spent) = (hard + 1, ErrorClass::Network);
                    if hard >= plan.hard_failures.max(1) {
                        break ErrorClass::Network;
                    }
                }
                Err(_) => {
                    note(&mut errors, format!("attempt {}: no answer in {:?}", number + 1, plan.attempt_limit));
                    spent = ErrorClass::Timeout;
                }
            },
            () = sleep_until(next_start), if free => {}
            () = sleep_until(deadline) => {
                note(&mut errors, format!("{} attempts had no answer at the time limit", running.len()));
                break ErrorClass::Timeout;
            }
        }
    };
    let text = format!("no connection after {:?} and {attempts} attempts: {}", started.elapsed(), errors.join("; "));
    Err(NetError::new(class, text))
}

fn note(errors: &mut Vec<String>, error: String) {
    if errors.len() == KEPT_ERRORS {
        errors.remove(0);
    }
    errors.push(error);
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use futures_util::FutureExt as _;
    use tokio::time::sleep;

    use super::*;

    const MS: Duration = Duration::from_millis(1);
    const SECOND: Duration = Duration::from_secs(1);
    const PLAN: Race = Race {
        delay: Duration::from_millis(300),
        max_parallel: 4,
        attempt_limit: Duration::from_secs(3),
        limit: Duration::from_secs(10),
        hard_failures: 2,
        max_attempts: usize::MAX,
    };

    /// A race where attempt N needs `times_ms[N]` to connect. The last time repeats.
    /// `None` is a refused connect after 1 ms.
    async fn scripted(times_ms: &[Option<u64>]) -> (Result<Won<usize>, NetError>, Duration) {
        let started = Instant::now();
        let result = race(PLAN, |number| {
            let time = times_ms.get(number).or(times_ms.last()).copied().flatten();
            async move {
                sleep(time.map_or(MS, Duration::from_millis)).await;
                time.map(|_| number).ok_or_else(|| io::Error::other("connection refused"))
            }
        })
        .await;
        (result, started.elapsed())
    }

    #[tokio::test(start_paused = true)]
    async fn the_second_attempt_wins_when_the_first_is_held() {
        let (won, elapsed) = scripted(&[Some(20_000), Some(20)]).await;
        let won = won.expect("a connection");
        assert_eq!((won.io, won.attempts, won.time), (1, 2, 20 * MS));
        assert_eq!(elapsed, 320 * MS);
    }

    #[tokio::test(start_paused = true)]
    async fn a_fast_connect_starts_one_attempt() {
        let (won, elapsed) = scripted(&[Some(20)]).await;
        let won = won.expect("a connection");
        assert_eq!((won.io, won.attempts, elapsed), (0, 1, 20 * MS));
    }

    #[tokio::test(start_paused = true)]
    async fn four_attempts_run_at_most_and_a_new_one_replaces_each_that_ends() {
        let in_progress = Cell::new(0_usize);
        let most = Cell::new(0_usize);
        let error = race(PLAN, |_| {
            in_progress.set(in_progress.get() + 1);
            most.set(most.get().max(in_progress.get()));
            let done = Finished(&in_progress);
            async move {
                sleep(Duration::from_secs(60)).await;
                drop(done);
                Ok(())
            }
        })
        .await
        .expect_err("each attempt is held");
        assert_eq!(most.get(), 4);
        assert_eq!(error.class(), ErrorClass::Timeout);
        // Starts at 0, 0.3, 0.6 and 0.9 s, and each one starts again 3 s later, three times.
        let expected = "no connection after 10s and 16 attempts: attempt 10: no answer in 3s; attempt 11: no answer in 3s; \
                        attempt 12: no answer in 3s; 4 attempts had no answer at the time limit";
        assert_eq!(error.to_string(), expected);
    }

    struct Finished<'a>(&'a Cell<usize>);

    impl Drop for Finished<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn two_refused_connects_end_the_race_with_no_busy_loop() {
        let (result, elapsed) = scripted(&[None]).await;
        let error = result.expect_err("refused");
        assert_eq!(elapsed, 301 * MS, "the second attempt waits for the delay");
        assert_eq!(error.class(), ErrorClass::Network);
        let expected =
            "no connection after 301ms and 2 attempts: attempt 1: connection refused; attempt 2: connection refused";
        assert_eq!(error.to_string(), expected);
    }

    #[tokio::test(start_paused = true)]
    async fn a_race_of_one_attempt_ends_with_that_attempt() {
        let single = Race { max_attempts: 1, ..PLAN };
        let started = Instant::now();
        let held = race(single, |_| sleep(Duration::from_secs(60)).map(|()| Ok(()))).await.expect_err("held");
        assert_eq!((held.class(), started.elapsed()), (ErrorClass::Timeout, 3 * SECOND));
        assert_eq!(held.to_string(), "no connection after 3s and 1 attempts: attempt 1: no answer in 3s");

        let started = Instant::now();
        let refused = race(single, |_| sleep(MS).map(|()| Err::<(), _>(io::Error::other("connection refused"))));
        let refused = refused.await.expect_err("refused");
        assert_eq!((refused.class(), started.elapsed()), (ErrorClass::Network, MS), "no wait for a second attempt");
    }

    #[tokio::test(start_paused = true)]
    async fn one_refused_connect_does_not_end_the_race() {
        let (won, elapsed) = scripted(&[None, Some(50)]).await;
        assert_eq!((won.expect("a connection").io, elapsed), (1, 350 * MS));
    }
}
