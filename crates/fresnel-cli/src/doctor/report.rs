//! The report of the `doctor` command. The JSON file is this value.
//!
//! The report holds times, counts, names and error texts. It never holds a token, the text
//! that a plugin wrote, or data of an object.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A connect of this time or more counts as held (the acceptance of milestone M0).
pub(super) const HELD: Duration = Duration::from_secs(1);

/// Milliseconds with one decimal place.
pub(super) fn ms(time: Duration) -> f64 {
    (time.as_secs_f64() * 10_000.0).round() / 10.0
}

/// Seconds with one decimal place.
pub(super) fn seconds(time: Duration) -> f64 {
    (time.as_secs_f64() * 10.0).round() / 10.0
}

/// The result of one step: its value, or why it gave none.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum Step<T> {
    Done(T),
    Failed { error: String },
}

impl<T> Step<T> {
    pub(super) fn failed(error: impl ToString) -> Self {
        Self::Failed { error: error.to_string() }
    }
}

/// The smallest, the middle and the largest of some times, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(super) struct Summary {
    pub(super) min: f64,
    pub(super) median: f64,
    pub(super) max: f64,
}

impl Summary {
    /// `None` for no time.
    pub(super) fn of(times: &[Duration]) -> Option<Self> {
        let mut sorted = times.to_vec();
        sorted.sort_unstable();
        let (min, median, max) = (sorted.first()?, sorted.get(sorted.len() / 2)?, sorted.last()?);
        Some(Self { min: ms(*min), median: ms(*median), max: ms(*max) })
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Report {
    /// The program and its version.
    pub(super) tool: String,
    /// The start of the run, as an RFC 3339 time.
    pub(super) time: String,
    pub(super) environment: Environment,
    pub(super) kubeconfig: Kubeconfig,
    pub(super) clusters: Vec<Cluster>,
    /// One line for each task of the app that ended in the run. An empty list is the normal case.
    pub(super) task_reports: Vec<String>,
}

/// The environment for the credential plugins.
#[derive(Debug, Serialize)]
pub(super) struct Environment {
    /// "login shell" with its path, or "process" when the login shell gave nothing.
    pub(super) source: String,
    /// Why the login shell gave nothing.
    pub(super) problem: Option<String>,
    pub(super) resolve_ms: f64,
    pub(super) path_entries: usize,
    /// The folders in `PATH` of this process. A program from Finder has 4.
    pub(super) process_path_entries: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct Kubeconfig {
    pub(super) sources: Vec<String>,
    pub(super) absent: Vec<String>,
    pub(super) contexts: usize,
    pub(super) problems: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct Cluster {
    pub(super) context: String,
    /// Host and port. `None` when the kubeconfig entry cannot be used.
    pub(super) server: Option<String>,
    /// "plugin <command>", "client certificate" or "none".
    pub(super) auth: Option<String>,
    pub(super) plugin: Option<Plugin>,
    /// Each measurement, or why the command made none.
    pub(super) result: Step<Measured>,
}

/// Where the credential plugin of the context is.
#[derive(Debug, Serialize)]
pub(super) struct Plugin {
    pub(super) command: String,
    /// The folder of the program. `None` when no folder of `PATH` has it.
    pub(super) folder: Option<String>,
    /// True when the `PATH` of this process has the program with no login shell.
    pub(super) in_process_path: bool,
    pub(super) search_ms: f64,
}

#[derive(Debug, Serialize)]
pub(super) struct Measured {
    pub(super) dns: Step<Dns>,
    pub(super) alpn: Alpn,
    /// Connects with no hedge. `None` when the option `--samples` is 0 or no address is known.
    pub(super) plain: Option<Connects>,
    /// Connects as the app makes them.
    pub(super) hedged: Option<Connects>,
    pub(super) session: Step<Session>,
}

#[derive(Debug, Serialize)]
pub(super) struct Dns {
    /// Zero for a host that is an address.
    pub(super) time_ms: f64,
    pub(super) addresses: Vec<String>,
}

/// What the server answers to a connection that asks for exactly one protocol.
#[derive(Debug, Serialize)]
pub(super) struct Alpn {
    /// HTTP/2, for the channels.
    pub(super) http2: Step<bool>,
    /// HTTP/1.1, for exec, attach and port forward.
    pub(super) http1: Step<bool>,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct Connects {
    pub(super) samples: usize,
    pub(super) connected: usize,
    /// Samples of 1 s or more, and samples with no answer in the limit.
    pub(super) held: usize,
    pub(super) held_percent: f64,
    /// Samples with a definite failure, for example a refused connect.
    pub(super) failed: usize,
    /// The time from the start to a connection after TLS.
    pub(super) total_ms: Option<Summary>,
    /// With no hedge: the TCP connect.
    pub(super) tcp_ms: Option<Summary>,
    /// With no hedge: the TLS handshake.
    pub(super) tls_ms: Option<Summary>,
    /// With the hedge: the attempts of all samples. More than the samples shows slow first attempts.
    pub(super) attempts: Option<usize>,
    /// With the hedge: the delay before a second attempt, after these samples.
    pub(super) hedge_delay_ms: Option<f64>,
    /// The total time of each sample. `null` is a sample with no connection.
    pub(super) times_ms: Vec<Option<f64>>,
    /// The error of each failed sample, the first ones.
    pub(super) errors: Vec<String>,
}

/// The cluster with its channels connected, as in the app.
#[derive(Debug, Serialize)]
pub(super) struct Session {
    pub(super) credential: Option<Credential>,
    /// From the connect until each channel had an answer. `None`: not in the limit.
    pub(super) live_ms: Option<f64>,
    pub(super) channels: Vec<Channel>,
    /// `SETTINGS_MAX_CONCURRENT_STREAMS` of the server.
    pub(super) stream_limit: Option<usize>,
    /// The round trips of small requests on the open connection.
    pub(super) ping_ms: Option<Summary>,
    pub(super) version: Step<Version>,
    pub(super) idle_close: Option<IdleClose>,
}

#[derive(Debug, Serialize)]
pub(super) struct Credential {
    pub(super) plugin_runs: u64,
    /// From the connect to the first token: the search, the start and the run of the plugin.
    pub(super) first_token_ms: Option<f64>,
    /// The state, with the failure type of the plugin.
    pub(super) state: String,
    /// What the plugin wrote. It is for the terminal only.
    #[serde(skip)]
    pub(super) detail: String,
}

#[derive(Debug, Serialize)]
pub(super) struct Channel {
    pub(super) purpose: &'static str,
    pub(super) state: String,
    pub(super) connections: u64,
    pub(super) stream_limit: Option<usize>,
    pub(super) stream_budget: usize,
    /// The round trip of the light request of the channel.
    pub(super) light_request_ms: Option<f64>,
    pub(super) last_error: Option<String>,
}

/// The answer of `GET /version`.
#[derive(Debug, Deserialize, Serialize)]
pub(super) struct Version {
    #[serde(rename(deserialize = "gitVersion"))]
    pub(super) git_version: String,
    pub(super) platform: String,
}

/// What the server did with a connection that sent pings and no request.
#[derive(Debug, Serialize)]
pub(super) struct IdleClose {
    pub(super) waited_s: f64,
    /// `None`: the connection was open at the end of the wait.
    pub(super) closed_after_s: Option<f64>,
    pub(super) reason: Option<String>,
    /// True when the channels with a light request each 30 s kept their first connection.
    pub(super) light_request_kept_open: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn the_summary_gives_the_smallest_the_middle_and_the_largest_time() {
        assert_eq!(Summary::of(&[]), None);
        assert_eq!(Summary::of(&[7 * MS]), Some(Summary { min: 7.0, median: 7.0, max: 7.0 }));
        let times = [30 * MS, 10 * MS, 2500 * MS, 20 * MS, Duration::from_micros(12_340)];
        assert_eq!(Summary::of(&times), Some(Summary { min: 10.0, median: 20.0, max: 2500.0 }));
        assert_eq!(ms(Duration::from_micros(12_340)), 12.3);
    }

    #[test]
    fn a_step_is_its_value_or_an_error_text_and_the_plugin_text_stays_out() {
        let done = serde_json::to_string(&Step::Done(Dns { time_ms: 1.5, addresses: vec!["10.0.0.1:443".to_owned()] }));
        assert_eq!(done.expect("JSON"), r#"{"time_ms":1.5,"addresses":["10.0.0.1:443"]}"#);
        let failed = serde_json::to_string(&Step::<Dns>::failed("no such host"));
        assert_eq!(failed.expect("JSON"), r#"{"error":"no such host"}"#);

        let credential = Credential {
            plugin_runs: 1,
            first_token_ms: None,
            state: "failed".to_owned(),
            detail: "secret".to_owned(),
        };
        let text = serde_json::to_string(&credential).expect("JSON");
        assert_eq!(text, r#"{"plugin_runs":1,"first_token_ms":null,"state":"failed"}"#);
    }
}
