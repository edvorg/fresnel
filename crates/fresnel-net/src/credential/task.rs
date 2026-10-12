//! The task that keeps the token of one cluster current.
//!
//! It runs the plugin at the start, before the token expires, after an HTTP 401 and after a
//! failure. The rule for an HTTP 401 has three cases, by what the server said about the token:
//!
//! - Nothing yet: the plugin runs one time, and the request goes out again.
//! - The token answers an earlier HTTP 401, and the server refuses it too: one small request
//!   to a core path proves the token. If the proof passes, the HTTP 401 is the answer of one
//!   path, and only that request fails. If not, the credential has failed.
//! - The token is proven: an HTTP 401 changes nothing. After `proof_valid`, a new proof decides.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use tokio::sync::watch;

use super::document::Credential;
use super::schedule::{later, refresh_time, retry_pause};
use super::state::{CredentialFailure, CredentialState, Issued, RunReason, Standing, View};
use super::{Proof, Prover, Shared, TokenSource};
use crate::error::NetError;
use crate::signal::Listener;

pub(super) struct Task {
    shared: Arc<Shared>,
    source: Arc<dyn TokenSource>,
    prover: Arc<dyn Prover>,
    listener: Listener,
    refused: watch::Receiver<u64>,
    /// Failures in sequence. It sets the pause before the next run.
    failures: u32,
    /// True when the next token answers an HTTP 401.
    answers_refusal: bool,
    /// When the next run starts with no report from a request, and why.
    next: Option<(SystemTime, RunReason)>,
}

/// Gives the state "failed" to a run that ends in the middle of a plugin run, for example
/// with a panic. Without it, the state would show a plugin run that nobody waits for.
struct StateOnStop(Arc<Shared>);

impl Drop for StateOnStop {
    fn drop(&mut self) {
        let next_attempt = self.0.clock.now();
        self.0.publish(|view| {
            if matches!(view.state, CredentialState::Running { .. }) {
                view.state = CredentialState::Failed { failure: CredentialFailure::Stopped, failures: 1, next_attempt };
            }
        });
    }
}

impl Task {
    pub(super) fn new(
        shared: Arc<Shared>,
        source: Arc<dyn TokenSource>,
        prover: Arc<dyn Prover>,
        listener: Listener,
    ) -> Self {
        let mut refused = shared.refused.subscribe();
        // A report can come while no task runs.
        refused.mark_changed();
        Self { shared, source, prover, listener, refused, failures: 0, answers_refusal: false, next: None }
    }

    /// Runs until an error. The start function then runs a new task.
    pub(super) async fn run(mut self) -> Result<(), NetError> {
        let _state_on_stop = StateOnStop(Arc::clone(&self.shared));
        self.run_plugin(RunReason::Start).await;
        loop {
            let wake = self.next.map(|(at, _)| at);
            tokio::select! {
                changed = self.refused.changed() => {
                    if changed.is_err() {
                        return Err(NetError::invalid("the credential handle of the cluster is gone"));
                    }
                    let generation = *self.refused.borrow_and_update();
                    self.on_refused(generation).await;
                }
                // The wall clock sleep below starts again after each signal. Thus a wake
                // also checks the expiry time at once.
                _cause = self.listener.raised() => self.retry_now().await,
                () = self.shared.clock.sleep_until(wake) => {
                    if let Some((_, reason)) = self.next.take() {
                        self.run_plugin(reason).await;
                    }
                }
            }
        }
    }

    /// "Retry now", a wake and a network change start the run that a failure waits for.
    async fn retry_now(&mut self) {
        let failed = matches!(self.shared.view.borrow().state, CredentialState::Failed { .. });
        if failed {
            self.run_plugin(RunReason::Retry).await;
        }
    }

    async fn run_plugin(&mut self, reason: RunReason) {
        let shared = Arc::clone(&self.shared);
        let since = shared.clock.now();
        self.answers_refusal |= reason == RunReason::Unauthorized;
        shared.publish(|view| {
            let last_failure = match &view.state {
                CredentialState::Failed { failure, .. } => Some(failure.clone()),
                CredentialState::Running { last_failure, .. } => last_failure.clone(),
                CredentialState::None | CredentialState::Valid { .. } => None,
            };
            view.state = CredentialState::Running { reason, since, last_failure };
            // The server refused the token, so a request must wait for the new one.
            if reason == RunReason::Unauthorized {
                view.token = None;
            }
        });
        shared.plugin_runs.fetch_add(1, Ordering::Relaxed);
        match self.source.run(shared.settings.plugin_limit).await {
            Ok(credential) => self.accept(credential),
            Err(error) => self.fail(CredentialFailure::Plugin(error)),
        }
    }

    fn accept(&mut self, credential: Credential) {
        let now = self.shared.clock.now();
        let standing = if std::mem::take(&mut self.answers_refusal) {
            // The count of failures stays: the server can refuse this token too.
            Standing::AnswersRefusal
        } else {
            self.failures = 0;
            Standing::Fresh
        };
        let settings = &self.shared.settings;
        self.next = credential.expires_at.map(|at| (refresh_time(now, at, settings), RunReason::Expiry));
        let generation = self.shared.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let issued = Issued { header: credential.header, generation, standing, since: now };
        let state = CredentialState::Valid { until: credential.expires_at };
        self.shared.publish(|view| *view = View { state, token: Some(Arc::new(issued)) });
    }

    /// A failed plugin run keeps the token: after a run for an expiry it can still be valid.
    /// A refusal removes it, so that no request goes to the server with it.
    fn fail(&mut self, failure: CredentialFailure) {
        self.failures = self.failures.saturating_add(1);
        let next_attempt = later(self.shared.clock.now(), retry_pause(self.failures, &self.shared.settings));
        self.next = Some((next_attempt, RunReason::Retry));
        let refused = failure == CredentialFailure::Refused;
        self.answers_refusal |= refused;
        let failures = self.failures;
        self.shared.publish(|view| {
            if refused {
                view.token = None;
            }
            view.state = CredentialState::Failed { failure, failures, next_attempt };
        });
    }

    /// A request reports that the server refused the token of `generation`.
    async fn on_refused(&mut self, generation: u64) {
        // No token: an earlier report removed it. Other generation: a newer token exists.
        let current = self.shared.view.borrow().token.clone();
        let Some(token) = current.filter(|token| token.generation == generation) else {
            return;
        };
        let (now, valid) = (self.shared.clock.now(), self.shared.settings.proof_valid);
        match token.standing {
            Standing::Proven if token.has_recent(Standing::Proven, now, valid) => {}
            // The token was good before. If it is not good now, a new token can help.
            Standing::Proven => match self.prove(&token).await {
                Proof::Refused => self.run_plugin(RunReason::Unauthorized).await,
                Proof::Accepted | Proof::NoAnswer => {}
            },
            // The plugin gave this token a moment before. One more run would give the same result.
            Standing::AnswersRefusal if token.has_recent(Standing::AnswersRefusal, now, valid) => {
                match self.prove(&token).await {
                    Proof::Refused => self.fail(CredentialFailure::Refused),
                    Proof::Accepted | Proof::NoAnswer => {}
                }
            }
            Standing::Fresh | Standing::AnswersRefusal => self.run_plugin(RunReason::Unauthorized).await,
        }
    }

    /// Asks the server about the token on a core path. An accepted token becomes proven.
    async fn prove(&mut self, token: &Issued) -> Proof {
        self.shared.proofs.fetch_add(1, Ordering::Relaxed);
        let proof = self.prover.prove(token.header.clone()).await;
        if proof == Proof::Accepted {
            self.failures = 0;
            let proven = Issued {
                header: token.header.clone(),
                generation: token.generation,
                standing: Standing::Proven,
                since: self.shared.clock.now(),
            };
            self.shared.publish(|view| view.token = Some(Arc::new(proven)));
        }
        proof
    }
}
