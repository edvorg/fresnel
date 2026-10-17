//! The forward of the soak (plan sections 12.3 and 13, M5): one forward to a Service whose
//! pods serve a page, and a check each minute that fetches the page through the local port.
//!
//! A check tries for 20 s: the time in which a silent path shows. A check that fails while
//! the forward shows "active" for all that time is a gap with no visible state (design rule
//! 4). A check in a fault fails with a visible state, and the forward must work again in 5
//! minutes.
//! The local port stays open in each fault: a connect that the port refuses fails the soak.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use fresnel_core::app::ForwardRow;
use fresnel_core::cluster::ClusterId;
use fresnel_core::forward::{Bind, ForwardRule, ForwardState, LocalPort};
use fresnel_core::kinds::Kind;
use fresnel_core::net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use fresnel_core::store::ObjectKey;
use serde::Serialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::{Instant, sleep, timeout};

use super::look::Look;

/// The time from one check to the next one.
const PERIOD: Duration = Duration::from_secs(60);
/// The longest try of a check: a forward shows a silent path in this time (plan section 15).
pub(super) const LIMIT: Duration = Duration::from_secs(20);
/// The longest wait for one page. A connection in a break waits 5 s for its tunnel.
const ONE_TRY: Duration = Duration::from_secs(8);
/// The text of the page of the pods of the Service.
pub(super) const PAGE: &str = "soak-echo";
/// The most bytes of an answer that a check reads.
const MOST: usize = 64 * 1024;

/// The Service of the forward: `<namespace>/<name>:<port>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) port: u16,
}

impl Target {
    /// The target of an option, or the reason why the text is none.
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        let wrong = || format!("the value of --forward is <namespace>/<service>:<port>, not `{text}`");
        let (object, port) = text.split_once(':').ok_or_else(wrong)?;
        let (namespace, name) = object.split_once('/').ok_or_else(wrong)?;
        let port = port.parse().ok().filter(|port| *port > 0).ok_or_else(wrong)?;
        if namespace.is_empty() || name.is_empty() {
            return Err(wrong());
        }
        Ok(Self { namespace: namespace.to_owned(), name: name.to_owned(), port })
    }

    /// The rule of the forward: a free local port on the loopback addresses.
    pub(super) fn rule(&self, cluster: &ClusterId) -> ForwardRule {
        ForwardRule {
            cluster: cluster.clone(),
            kind: Kind::Service,
            key: ObjectKey::new(&self.namespace, &self.name),
            port: self.port,
            local: LocalPort::Automatic,
            bind: Bind::Localhost,
        }
    }
}

/// One check: a fetch of the page through the local port.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct Check {
    pub(super) ok: bool,
    pub(super) seconds: f64,
    /// The state of the forward at the end of the check.
    pub(super) state: String,
    pub(super) error: Option<String>,
    /// The check failed while the forward showed "active" for its whole time.
    pub(super) without_state: bool,
    /// The local port refused the connect.
    pub(super) port_closed: bool,
}

/// The checks of a minute.
pub(super) type Checks = Arc<Mutex<Vec<Check>>>;

/// What a minute of the report has of the forward.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(super) struct Minute {
    /// The state of the forward at the end of the minute, and its counts.
    pub(super) state: String,
    pub(super) connections: String,
    pub(super) checks: Vec<Check>,
}

impl Minute {
    /// The minute of the forward of `rule` in a copy of the core.
    pub(super) fn of(look: &Look, rule: &ForwardRule, checks: Vec<Check>) -> Self {
        let row = row(look, rule);
        let state = row.map_or_else(|| "absent".to_owned(), |row| row.view.state.to_string());
        let connections = row.map_or_else(String::new, |row| {
            let counts = row.view.connections;
            format!("{} in all, {} reset", counts.total, counts.reset)
        });
        Self { state, connections, checks }
    }
}

fn row<'a>(look: &'a Look, rule: &ForwardRule) -> Option<&'a ForwardRow> {
    look.forwards.iter().find(|row| &row.view.rule == rule)
}

/// Starts the check each minute. It stops when the handle drops.
pub(super) fn start(
    tasks: &Tasks,
    rule: ForwardRule,
    looks: &watch::Receiver<Arc<Look>>,
    checks: &Checks,
) -> TaskHandle {
    let (looks, checks) = (looks.clone(), Arc::clone(checks));
    tasks.start(TaskName::new("soak forward"), Restart::Always(Backoff::default()), move || {
        each_minute(rule.clone(), looks.clone(), Arc::clone(&checks))
    })
}

async fn each_minute(rule: ForwardRule, looks: watch::Receiver<Arc<Look>>, checks: Checks) -> Result<(), Infallible> {
    loop {
        sleep(PERIOD).await;
        let check = check(&rule, &looks, LIMIT).await;
        checks.lock().unwrap_or_else(PoisonError::into_inner).push(check);
    }
}

/// Fetches the page until it comes, for `limit` at most. The forward has a visible state when
/// it shows an other state than "active" before a try or at the end: a forward that comes back
/// from "reconnecting" in a fault shows "active" at once, while a tunnel of the fault fails.
async fn check(rule: &ForwardRule, looks: &watch::Receiver<Arc<Look>>, limit: Duration) -> Check {
    let started = Instant::now();
    let (mut error, mut port_closed, mut shown) = (None, false, false);
    let mut see = |looks: &watch::Receiver<Arc<Look>>| {
        shown |= row(&looks.borrow(), rule).is_none_or(|row| row.view.state != ForwardState::Active);
    };
    while started.elapsed() < limit {
        see(looks);
        let address = row(&looks.borrow(), rule).and_then(|row| row.view.address());
        let one_try = ONE_TRY.min(limit);
        match address {
            Some(address) => match timeout(one_try, fetch(address)).await {
                Ok(Ok(())) => {
                    error = None;
                    break;
                }
                Ok(Err((text, refused))) => (error, port_closed) = (Some(text), port_closed || refused),
                Err(_) => error = Some(format!("no page in {:.1} s", one_try.as_secs_f64())),
            },
            None => error = Some("the forward has no local address".to_owned()),
        }
        sleep((limit / 20).min(Duration::from_secs(1))).await;
    }
    see(looks);
    let state = row(&looks.borrow(), rule).map(|row| row.view.state.clone());
    let without_state = error.is_some() && !shown;
    Check {
        ok: error.is_none(),
        seconds: started.elapsed().as_secs_f64(),
        state: state.map_or_else(|| "absent".to_owned(), |state| state.to_string()),
        error,
        without_state,
        port_closed,
    }
}

/// One fetch of the page. The error has its text and true for a port that refused the connect.
async fn fetch(address: SocketAddr) -> Result<(), (String, bool)> {
    let mut client = TcpStream::connect(address).await.map_err(|error| {
        let refused = error.kind() == std::io::ErrorKind::ConnectionRefused;
        (format!("connect: {error}"), refused)
    })?;
    let request = "GET / HTTP/1.0\r\nHost: soak\r\n\r\n";
    client.write_all(request.as_bytes()).await.map_err(|error| (format!("write: {error}"), false))?;
    let mut answer = Vec::new();
    let read = (&mut client).take(MOST as u64).read_to_end(&mut answer).await;
    read.map_err(|error| (format!("read: {error}"), false))?;
    let text = String::from_utf8_lossy(&answer);
    match text.starts_with("HTTP/1.") && text.contains(" 200 ") && text.contains(PAGE) {
        true => Ok(()),
        false => Err((format!("an other answer: {}", text.lines().next().unwrap_or("no line")), false)),
    }
}

/// The verdict of the forward over the minutes.
#[derive(Debug, Default, Serialize)]
pub(super) struct Verdict {
    pub(super) checks: usize,
    pub(super) failed: usize,
    pub(super) without_state: usize,
    pub(super) port_closed: usize,
    /// The most minutes in a row with no good check, after the first check.
    pub(super) quiet_minutes: usize,
}

impl Verdict {
    /// The verdict of the minutes so far, each with true in an outage of the environment, and
    /// its reasons for a failure. `quiet` is the most minutes in a row with no good check that
    /// pass; a minute of an outage ends a run. A check with no visible state or a closed port
    /// fails also in an outage.
    pub(super) fn of(minutes: &[(Option<&Minute>, bool)], quiet: usize) -> (Self, Vec<String>) {
        let checks: Vec<&Check> = minutes.iter().filter_map(|(minute, _)| *minute).flat_map(|m| &m.checks).collect();
        let (mut longest, mut run) = (0, 0);
        for (minute, outage) in minutes {
            let Some(minute) = minute.filter(|minute| !minute.checks.is_empty()) else {
                continue;
            };
            run = if *outage || minute.checks.iter().any(|check| check.ok) { 0 } else { run + 1 };
            longest = longest.max(run);
        }
        let verdict = Self {
            checks: checks.len(),
            failed: checks.iter().filter(|check| !check.ok).count(),
            without_state: checks.iter().filter(|check| check.without_state).count(),
            port_closed: checks.iter().filter(|check| check.port_closed).count(),
            quiet_minutes: longest,
        };
        let mut reasons = Vec::new();
        if verdict.without_state > 0 {
            let limit = LIMIT.as_secs();
            reasons
                .push(format!("{} checks of the forward: \"active\" and no page in {limit} s", verdict.without_state));
        }
        if verdict.port_closed > 0 {
            reasons.push(format!("{} checks of the forward found its local port closed", verdict.port_closed));
        }
        if verdict.quiet_minutes >= quiet {
            reasons.push(format!("the forward gave no page for {} minutes in a row", verdict.quiet_minutes));
        }
        (verdict, reasons)
    }
}

#[cfg(test)]
mod tests;
