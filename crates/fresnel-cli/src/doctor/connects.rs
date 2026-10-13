//! The connects to one cluster: the address lookup, the protocols of the server, and
//! connect samples with and with no hedge.
//!
//! Each sample is a new connection that closes after its TLS handshake. No sample sends a
//! request.

use std::net::SocketAddr;
use std::time::Duration;

use fresnel_core::net::probe::Probe;
use fresnel_core::net::{ErrorClass, NetError, Protocol};

use super::report::{Alpn, Connects, Dns, HELD, Step, Summary, ms};

/// The report keeps the errors of the first failed samples. A later one most often repeats them.
const KEPT_ERRORS: usize = 8;

pub(super) struct ConnectFacts {
    pub(super) dns: Step<Dns>,
    pub(super) alpn: Alpn,
    pub(super) plain: Option<Connects>,
    pub(super) hedged: Option<Connects>,
}

/// Measures with `count` samples of each connect type. `limit` is the longest wait for
/// one sample with no hedge.
pub(super) async fn measure(probe: &Probe, count: usize, limit: Duration) -> ConnectFacts {
    let lookup = match probe.lookup().await {
        Ok(lookup) => lookup,
        Err(error) => {
            // With no address, each other step would give this same error.
            let alpn = Alpn { http2: Step::failed(&error), http1: Step::failed(&error) };
            return ConnectFacts { dns: Step::failed(&error), alpn, plain: None, hedged: None };
        }
    };
    let addresses: Vec<String> = lookup.addresses.iter().map(ToString::to_string).collect();
    let dns = Step::Done(Dns { time_ms: ms(lookup.time), addresses });
    // The hedge makes these two connects safe against one held connection.
    let alpn = Alpn { http2: offers(probe, Protocol::Http2).await, http1: offers(probe, Protocol::Http1).await };
    if count == 0 {
        return ConnectFacts { dns, alpn, plain: None, hedged: None };
    }
    let plain = plain(probe, &lookup.addresses, count, limit).await;
    let hedged = hedged(probe, count).await;
    ConnectFacts { dns, alpn, plain: Some(plain), hedged: Some(hedged) }
}

/// True when the server accepts a connection that asks for this one protocol.
async fn offers(probe: &Probe, protocol: Protocol) -> Step<bool> {
    match probe.hedged(protocol).await {
        Ok(_) => Step::Done(true),
        Err(error) => Step::failed(error),
    }
}

/// Connects with no hedge, one after the other, to each address in turn.
async fn plain(probe: &Probe, addresses: &[SocketAddr], count: usize, limit: Duration) -> Connects {
    let mut samples = Samples::default();
    let (mut tcp, mut tls) = (Vec::new(), Vec::new());
    for address in addresses.iter().cycle().take(count) {
        match probe.plain(*address, Protocol::Http2, limit).await {
            Ok(connect) => {
                tcp.push(connect.tcp);
                tls.push(connect.tls);
                samples.connected(connect.tcp + connect.tls);
            }
            Err(error) => samples.failed(&error),
        }
    }
    Connects { tcp_ms: Summary::of(&tcp), tls_ms: Summary::of(&tls), ..samples.finish() }
}

/// Connects as a channel of the app makes them, one after the other.
async fn hedged(probe: &Probe, count: usize) -> Connects {
    let mut samples = Samples::default();
    let mut attempts = 0;
    for _ in 0..count {
        match probe.hedged(Protocol::Http2).await {
            Ok(connect) => {
                attempts += connect.attempts;
                samples.connected(connect.time);
            }
            Err(error) => samples.failed(&error),
        }
    }
    let hedge_delay = probe.status().hedge_delay;
    Connects { attempts: Some(attempts), hedge_delay_ms: Some(ms(hedge_delay)), ..samples.finish() }
}

/// The results of the samples of one connect type.
#[derive(Default)]
struct Samples {
    times: Vec<Option<Duration>>,
    held: usize,
    failed: usize,
    errors: Vec<String>,
}

impl Samples {
    fn connected(&mut self, time: Duration) {
        self.held += usize::from(time >= HELD);
        self.times.push(Some(time));
    }

    /// A sample with no answer in its limit is a held connection. Each other error is a
    /// definite failure, for example a refused connect.
    fn failed(&mut self, error: &NetError) {
        self.failed_with(error.class(), error.to_string());
    }

    fn failed_with(&mut self, class: ErrorClass, text: String) {
        let no_answer = class == ErrorClass::Timeout;
        self.held += usize::from(no_answer);
        self.failed += usize::from(!no_answer);
        self.times.push(None);
        if self.errors.len() < KEPT_ERRORS {
            self.errors.push(text);
        }
    }

    fn finish(self) -> Connects {
        let connected: Vec<Duration> = self.times.iter().flatten().copied().collect();
        let samples = self.times.len();
        // Tenths of a percent, as a whole number: a small count has an exact `f64`.
        let tenths = (self.held * 1000).checked_div(samples).and_then(|tenths| u32::try_from(tenths).ok());
        let held_percent = f64::from(tenths.unwrap_or_default()) / 10.0;
        Connects {
            samples,
            connected: connected.len(),
            held: self.held,
            held_percent,
            failed: self.failed,
            total_ms: Summary::of(&connected),
            times_ms: self.times.iter().map(|time| time.map(ms)).collect(),
            errors: self.errors,
            ..Connects::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_sample_is_held_at_one_second_or_with_no_answer_and_a_refusal_is_a_failure() {
        let mut samples = Samples::default();
        for time in [20 * MS, 999 * MS, 1000 * MS, 2500 * MS] {
            samples.connected(time);
        }
        samples.failed_with(ErrorClass::Timeout, "no answer in 5s".to_owned());
        samples.failed_with(ErrorClass::Network, "connection refused".to_owned());
        let connects = samples.finish();
        assert_eq!((connects.samples, connects.connected, connects.held, connects.failed), (6, 4, 3, 1));
        assert_eq!(connects.held_percent, 50.0);
        assert_eq!(connects.times_ms, [Some(20.0), Some(999.0), Some(1000.0), Some(2500.0), None, None]);
        assert_eq!(connects.total_ms, Some(Summary { min: 20.0, median: 1000.0, max: 2500.0 }));
        assert_eq!(connects.errors, ["no answer in 5s", "connection refused"]);
    }

    #[test]
    fn the_report_keeps_the_first_errors_and_no_sample_gives_no_percent() {
        let none = Samples::default().finish();
        assert_eq!((none.samples, none.held_percent, none.total_ms), (0, 0.0, None));

        let mut samples = Samples::default();
        for number in 0..30 {
            samples.failed_with(ErrorClass::Timeout, format!("error {number}"));
        }
        let connects = samples.finish();
        assert_eq!((connects.held, connects.held_percent, connects.errors.len()), (30, 100.0, KEPT_ERRORS));
        assert_eq!(connects.errors.last().map(String::as_str), Some("error 7"));
        // One held sample of three: tenths of a percent, with no rounding up.
        let mut third = Samples::default();
        third.connected(MS);
        third.connected(MS);
        third.failed_with(ErrorClass::Timeout, String::new());
        assert_eq!(third.finish().held_percent, 33.3);
    }
}
