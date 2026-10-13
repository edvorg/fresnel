//! The address lookup of one cluster (plan section 8.3).
//!
//! A lookup of the system can hang for 30 seconds, and it cannot be stopped. Thus each
//! caller waits a short time only, one lookup runs at a time, and the last good addresses
//! stay in use.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::FutureExt as _;
use futures_util::future::{BoxFuture, Shared};
use tokio::time::timeout;

use crate::error::NetError;

/// One lookup: the host and the port give addresses, or the text of an error.
pub(crate) type Lookup = Box<dyn Fn(String, u16) -> BoxFuture<'static, Result<Vec<SocketAddr>, String>> + Send + Sync>;

type Pending = Shared<BoxFuture<'static, Result<Vec<SocketAddr>, String>>>;

pub(crate) struct Resolver {
    host: String,
    port: u16,
    limit: Duration,
    lookup: Lookup,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The lookup in progress. Each caller waits for this one.
    pending: Option<Pending>,
    last_good: Vec<SocketAddr>,
    last_error: Option<String>,
}

impl Resolver {
    /// A resolver on the lookup of the system.
    pub(crate) fn new(host: String, port: u16, limit: Duration) -> Self {
        Self::with_lookup(host, port, limit, Box::new(system_lookup))
    }

    pub(crate) fn with_lookup(host: String, port: u16, limit: Duration, lookup: Lookup) -> Self {
        Self { host, port, limit, lookup, state: Mutex::new(State::default()) }
    }

    /// The addresses of the host. After a lookup with no result in the limit, the last good
    /// addresses are the answer.
    pub(crate) async fn resolve(&self) -> Result<Vec<SocketAddr>, NetError> {
        if let Ok(ip) = self.host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, self.port)]);
        }
        let pending =
            self.state().pending.get_or_insert_with(|| (self.lookup)(self.host.clone(), self.port).shared()).clone();
        let result = timeout(self.limit, pending).await;
        let mut state = self.state();
        // Only a lookup that ended leaves the state. A lookup in progress stays: it can be
        // a slow one, or a newer one that some other caller started a moment before.
        if state.pending.as_ref().is_some_and(|pending| pending.peek().is_some()) {
            state.pending = None;
        }
        let problem = match result {
            Ok(Ok(addresses)) if !addresses.is_empty() => {
                state.last_good.clone_from(&addresses);
                state.last_error = None;
                return Ok(addresses);
            }
            Ok(Ok(_)) => "the name has no address".to_owned(),
            Ok(Err(error)) => error,
            Err(_) => format!("no answer in {:?}", self.limit),
        };
        state.last_error = Some(problem.clone());
        if state.last_good.is_empty() {
            return Err(NetError::network(format!("cannot resolve {}: {problem}", self.host)));
        }
        Ok(state.last_good.clone())
    }

    /// The last good addresses, and the error of the last lookup if it failed.
    pub(crate) fn status(&self) -> (Vec<SocketAddr>, Option<String>) {
        let state = self.state();
        (state.last_good.clone(), state.last_error.clone())
    }

    /// Each use holds the lock for one small change or one copy.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The lookup of the system. It gives each address of the host, or the text of the error.
pub(crate) fn system_lookup(host: String, port: u16) -> BoxFuture<'static, Result<Vec<SocketAddr>, String>> {
    async move {
        let addresses = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|error| error.to_string())?;
        Ok(addresses.collect())
    }
    .boxed()
}

#[cfg(test)]
mod tests {
    use std::pin::pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use tokio::time::{Instant, sleep};

    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn address(last: u8) -> SocketAddr {
        SocketAddr::from(([10, 0, 0, last], 6443))
    }

    /// A resolver whose lookup number N takes `seconds[N]`. `None` is an error.
    fn resolver(seconds: &'static [Option<u64>]) -> (Resolver, Arc<AtomicU32>) {
        let lookups = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&lookups);
        let lookup: Lookup = Box::new(move |_, _| {
            let number = count.fetch_add(1, Ordering::SeqCst);
            let step = seconds.get(number as usize).copied().expect("a scripted lookup");
            async move {
                let Some(step) = step else { return Err("no such host".to_owned()) };
                sleep(Duration::from_secs(step)).await;
                Ok(vec![address(u8::try_from(number).expect("a small number"))])
            }
            .boxed()
        });
        (Resolver::with_lookup("api.example.test".to_owned(), 6443, 2 * SECOND, lookup), lookups)
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_lookup_gives_the_last_good_addresses_at_the_limit() {
        let (resolver, lookups) = resolver(&[Some(0), Some(30), Some(0)]);
        assert_eq!(resolver.resolve().await, Ok(vec![address(0)]));

        let started = Instant::now();
        let (first, second) = tokio::join!(resolver.resolve(), resolver.resolve());
        assert_eq!((first, second), (Ok(vec![address(0)]), Ok(vec![address(0)])));
        assert_eq!(started.elapsed(), 2 * SECOND, "two callers wait one limit, not two");
        assert_eq!(resolver.status(), (vec![address(0)], Some("no answer in 2s".to_owned())));
        assert_eq!(resolver.resolve().await, Ok(vec![address(0)]));
        assert_eq!(lookups.load(Ordering::SeqCst), 2, "one lookup at a time: the slow one is still in progress");

        sleep(27 * SECOND).await;
        assert_eq!(resolver.resolve().await, Ok(vec![address(1)]), "the slow lookup ended and gave its addresses");
        assert_eq!(resolver.status(), (vec![address(1)], None));
        assert_eq!(resolver.resolve().await, Ok(vec![address(2)]));
    }

    /// Two callers wait for one lookup, and the second one reads its result late.
    #[tokio::test(start_paused = true)]
    async fn a_caller_that_reads_a_result_late_does_not_end_a_newer_lookup() {
        let (resolver, lookups) = resolver(&[Some(1), Some(30), Some(0)]);
        let (mut first, mut second) = (pin!(resolver.resolve()), pin!(resolver.resolve()));
        assert!(first.as_mut().now_or_never().is_none() && second.as_mut().now_or_never().is_none());
        sleep(SECOND).await;
        assert_eq!(first.as_mut().now_or_never(), Some(Ok(vec![address(0)])));

        // A third caller starts the next lookup before the second caller runs.
        let mut third = pin!(resolver.resolve());
        assert!(third.as_mut().now_or_never().is_none());
        assert_eq!(second.as_mut().now_or_never(), Some(Ok(vec![address(0)])));
        let mut fourth = pin!(resolver.resolve());
        assert!(fourth.as_mut().now_or_never().is_none());
        assert_eq!(lookups.load(Ordering::SeqCst), 2, "one lookup at a time: the fourth caller waits for the second");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_lookup_is_an_error_only_with_no_good_address() {
        let (resolver, _) = resolver(&[None, Some(5), Some(0), None]);
        let error = resolver.resolve().await.expect_err("no address");
        assert_eq!(error.to_string(), "cannot resolve api.example.test: no such host");
        let error = resolver.resolve().await.expect_err("no address");
        assert_eq!(error.to_string(), "cannot resolve api.example.test: no answer in 2s");
        sleep(3 * SECOND).await;
        assert_eq!(resolver.resolve().await, Ok(vec![address(1)]));
        assert_eq!(resolver.resolve().await, Ok(vec![address(2)]));
        assert_eq!(resolver.resolve().await, Ok(vec![address(2)]), "the last good address after an error");
    }

    #[tokio::test(start_paused = true)]
    async fn an_address_needs_no_lookup() {
        let (_, lookups) = resolver(&[]);
        let resolver = Resolver::new("127.0.0.1".to_owned(), 6443, 2 * SECOND);
        assert_eq!(resolver.resolve().await, Ok(vec![SocketAddr::from(([127, 0, 0, 1], 6443))]));
        assert_eq!(lookups.load(Ordering::SeqCst), 0);
    }
}
