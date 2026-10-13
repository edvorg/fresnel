//! The credential of one cluster (plan section 8.2).
//!
//! One task owns the plugin, so two plugin processes for one cluster never run at the same
//! time. A request only reads the result, and it waits a short time at most. The token
//! stays in memory.
//!
//! - `plugin`: one run of the plugin. `document`: its answer.
//! - `task`: the task that decides when the plugin runs. `schedule`: its time rules.
//! - `state`: what the UI shows. `layer`: the token of a request and the HTTP 401 rule.

mod document;
mod error;
pub(crate) mod layer;
mod plugin;
mod schedule;
mod state;
mod task;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::future::BoxFuture;
use http::HeaderValue;
use tokio::sync::watch;
use tokio::time::timeout;

#[cfg(test)]
pub(crate) use document::Credential;
pub use error::{PluginError, PluginFailure};
pub use plugin::PluginSpec;
pub(crate) use plugin::{ExecPlugin, TokenSource};
pub use state::{CredentialFailure, CredentialState, RunReason};
use state::{Issued, Verdict, View};

use crate::changes::Changed;
use crate::clock::WallClock;
use crate::error::{ErrorClass, NetError};
use crate::signal::Signal;

/// The settings of the credential of a cluster. The values are first values (plan section 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialSettings {
    /// The time limit of one plugin run.
    pub plugin_limit: Duration,
    /// The longest wait of a request for a plugin run.
    pub request_wait: Duration,
    /// The plugin runs again this long before the token expires.
    pub refresh_lead: Duration,
    /// The shortest pause between two runs for an expiry.
    pub min_refresh_pause: Duration,
    /// The pause after the first failure. It doubles for each next failure, to `retry_max`.
    pub retry_first: Duration,
    /// The longest pause after a failure.
    pub retry_max: Duration,
    /// How long the proof of a token counts. In this time, an HTTP 401 is the answer of one
    /// path and starts no plugin run.
    pub proof_valid: Duration,
    /// The time limit of the proof request.
    pub proof_limit: Duration,
}

impl Default for CredentialSettings {
    fn default() -> Self {
        Self {
            plugin_limit: Duration::from_secs(20),
            request_wait: Duration::from_secs(2),
            refresh_lead: Duration::from_secs(60),
            min_refresh_pause: Duration::from_secs(10),
            retry_first: Duration::from_secs(2),
            retry_max: Duration::from_secs(60),
            proof_valid: Duration::from_secs(30),
            proof_limit: Duration::from_secs(3),
        }
    }
}

/// A copy of the credential facts of one cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialStatus {
    /// The state.
    pub state: CredentialState,
    /// The count of plugin runs that started.
    pub plugin_runs: u64,
    /// The count of HTTP 401 answers to a request with a token.
    pub unauthorized: u64,
    /// The count of proof requests.
    pub proofs: u64,
}

/// What the server said to the proof request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Proof {
    Accepted,
    /// HTTP 401.
    Refused,
    NoAnswer,
}

/// Proves a token with one small request to a path that each user can read.
pub(crate) trait Prover: Send + Sync + 'static {
    fn prove(&self, authorization: HeaderValue) -> BoxFuture<'_, Proof>;
}

struct Shared {
    settings: CredentialSettings,
    clock: WallClock,
    /// Tells the readers of the cluster that the state changed.
    changed: Changed,
    view: watch::Sender<View>,
    /// The highest generation of a token that the server refused. Each report sends a
    /// change, also with the same number, and no report waits in a queue.
    refused: watch::Sender<u64>,
    generation: AtomicU64,
    plugin_runs: AtomicU64,
    unauthorized: AtomicU64,
    proofs: AtomicU64,
}

impl Shared {
    /// Changes the state and the token in one step, and tells the readers of the cluster.
    fn publish(&self, change: impl FnOnce(&mut View)) {
        self.view.send_modify(change);
        self.changed.notify();
    }
}

/// The handle to the credential of one cluster. A clone is cheap.
#[derive(Clone)]
pub(crate) struct Credentials {
    shared: Arc<Shared>,
}

impl Credentials {
    pub(crate) fn new(settings: CredentialSettings, clock: WallClock, changed: Changed) -> Self {
        let shared = Shared {
            settings,
            clock,
            changed,
            view: watch::Sender::new(View { state: CredentialState::None, token: None }),
            refused: watch::Sender::new(0),
            generation: AtomicU64::new(0),
            plugin_runs: AtomicU64::new(0),
            unauthorized: AtomicU64::new(0),
            proofs: AtomicU64::new(0),
        };
        Self { shared: Arc::new(shared) }
    }

    /// One run of the credential task. The start function calls it again after an end.
    pub(crate) fn task(
        &self,
        source: Arc<dyn TokenSource>,
        prover: Arc<dyn Prover>,
        signal: &Signal,
    ) -> impl Future<Output = Result<(), NetError>> + Send + 'static + use<> {
        task::Task::new(Arc::clone(&self.shared), source, prover, signal.listen()).run()
    }

    pub(crate) fn status(&self) -> CredentialStatus {
        CredentialStatus {
            state: self.shared.view.borrow().state.clone(),
            plugin_runs: self.shared.plugin_runs.load(Ordering::Relaxed),
            unauthorized: self.shared.unauthorized.load(Ordering::Relaxed),
            proofs: self.shared.proofs.load(Ordering::Relaxed),
        }
    }

    /// The token of this moment, with no wait. The light request of a channel uses it.
    pub(crate) fn authorization_now(&self) -> Option<HeaderValue> {
        self.shared.view.borrow().token.as_ref().map(|token| token.header.clone())
    }

    /// The token for a request. It waits `request_wait` at most, and only while no token
    /// exists and one can still come.
    pub(crate) async fn token(&self) -> Result<Arc<Issued>, NetError> {
        match self.wait(View::token).await {
            Some(Ok(token)) => Ok(token),
            Some(Err(failure)) => Err(auth_error(&failure)),
            None => Err(self.wait_error()),
        }
    }

    /// Tells the task that the server answered HTTP 401 to this token, and waits for its
    /// decision. `Some` is a newer token for a second send. `None` means that the answer of
    /// the server goes to the caller: the token is good on a core path, or nothing is known.
    pub(crate) async fn after_refusal(&self, refused: &Issued) -> Result<Option<Arc<Issued>>, NetError> {
        self.shared.unauthorized.fetch_add(1, Ordering::Relaxed);
        self.shared.refused.send_modify(|highest| *highest = (*highest).max(refused.generation));
        let (clock, valid) = (&self.shared.clock, self.shared.settings.proof_valid);
        match self.wait(|view| view.verdict(refused.generation, clock.now(), valid)).await {
            Some(Verdict::Replaced(newer)) => Ok(Some(newer)),
            Some(Verdict::PathOnly) => Ok(None),
            Some(Verdict::Unavailable(failure)) => Err(auth_error(&failure)),
            None if self.shared.view.borrow().token.is_some() => Ok(None),
            None => Err(self.wait_error()),
        }
    }

    /// Waits until the cluster has a token, with no time limit. A channel waits here before
    /// its light request: a request with no token would end its connection.
    pub(crate) async fn token_exists(&self) {
        self.answered(|view| view.token.as_ref().map(|_| ())).await;
    }

    /// Waits until `answer` gives a value, for `request_wait` at most.
    async fn wait<T>(&self, answer: impl Fn(&View) -> Option<T>) -> Option<T> {
        timeout(self.shared.settings.request_wait, self.answered(answer)).await.ok()
    }

    /// Waits until `answer` gives a value.
    async fn answered<T>(&self, answer: impl Fn(&View) -> Option<T>) -> T {
        let mut view = self.shared.view.subscribe();
        loop {
            // The guard of the borrow ends with this statement, before the wait.
            let now = answer(&view.borrow_and_update());
            match now {
                Some(value) => return value,
                // The sender lives in `shared`, so the wait cannot fail.
                None if view.changed().await.is_err() => std::future::pending::<()>().await,
                None => {}
            }
        }
    }

    /// The error of a request that did not get a token in the wait: the credential state.
    fn wait_error(&self) -> NetError {
        let wait = self.shared.settings.request_wait;
        let state = self.shared.view.borrow().state.clone();
        NetError::new(ErrorClass::Auth, format!("no credential in {wait:?}: {state}"))
    }
}

fn auth_error(failure: &CredentialFailure) -> NetError {
    NetError::new(ErrorClass::Auth, failure.to_string())
}

#[cfg(test)]
mod tests;
