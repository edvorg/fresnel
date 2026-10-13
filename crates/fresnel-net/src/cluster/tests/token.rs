//! No request goes to the server while the cluster has no token (design rule 3).
//!
//! A kube-apiserver ends an HTTP/2 connection after its answer to a request with no
//! credential. A channel that sent its light request before the first token thus lost its
//! connection and connected again, for the full time of a slow plugin run.

use futures_util::future::BoxFuture;

use super::*;
use crate::channel::ChannelStatus;
use crate::credential::{Credential, CredentialFailure, PluginError};
use crate::signal::Cause;

/// A plugin that needs `run_time` for each run.
struct SlowTokens {
    tokens: Arc<Tokens>,
    run_time: Duration,
}

impl TokenSource for SlowTokens {
    fn run(&self, limit: Duration) -> BoxFuture<'_, Result<Credential, PluginError>> {
        async move {
            sleep(self.run_time).await;
            self.tokens.run(limit).await
        }
        .boxed()
    }
}

fn slow(tokens: &[&'static str], run_time: Duration) -> Option<Arc<dyn TokenSource>> {
    Some(Arc::new(SlowTokens { tokens: Tokens::new(tokens), run_time }))
}

/// The first channel of each purpose: watches, requests, log streams.
fn first_channels(status: &ClusterStatus) -> [&ChannelStatus; 3] {
    [&status.watches.channels[0], &status.requests.channels[0], &status.logs.channels[0]]
}

fn all_are(status: &ClusterStatus, state: &ChannelState) -> bool {
    first_channels(status).iter().all(|channel| &channel.state == state)
}

/// The value of the `Authorization` header of each light request that the server saw.
fn tokens_of_light_requests(bed: &Bed) -> Vec<Option<String>> {
    let light = bed.server.seen().into_iter().filter(|seen| seen.path == "/version");
    let token = |seen: crate::testing::Seen| {
        seen.headers.get(AUTHORIZATION).map(|value| value.to_str().expect("text").to_owned())
    };
    light.map(token).collect()
}

#[tokio::test(start_paused = true)]
async fn a_channel_sends_no_request_before_the_first_token() {
    let mut bed = Bed::build(slow(&["one"], 20 * SECOND));
    bed.until("each channel waits for the token", |status| all_are(status, &ChannelState::WaitsForToken)).await;
    assert_eq!((bed.elapsed(), bed.server.dials()), (Duration::ZERO, 3), "each channel connects at once");

    // A wake tests each connection with a light request. With no token, it sends nothing.
    sleep(10 * SECOND).await;
    bed.cluster.signal().raise(Cause::Wake);
    sleep(9 * SECOND).await;
    let status = bed.cluster.status();
    for channel in first_channels(&status) {
        assert_eq!(
            (&channel.state, channel.connections, &channel.last_error),
            (&ChannelState::WaitsForToken, 1, &None),
            "the connection stays open during the plugin run"
        );
    }
    assert_eq!(bed.server.seen().len(), 0, "the server saw a request with no token");

    let live = bed.until("each channel is live", |status| all_are(status, &ChannelState::Live)).await;
    assert_eq!(bed.elapsed(), 20 * SECOND, "the light request goes out when the token arrives");
    assert!(first_channels(&live).iter().all(|channel| channel.connections == 1), "{live:?}");
    assert_eq!(bed.server.dials(), 3, "one connection for each channel");
    assert_eq!(tokens_of_light_requests(&bed), [(); 3].map(|()| Some("Bearer one".to_owned())));
}

/// After an HTTP 401, the cluster has no token while the plugin runs again. An idle
/// channel sends no light request in that time.
#[tokio::test(start_paused = true)]
async fn an_idle_channel_sends_no_light_request_while_the_plugin_runs_again() {
    let mut bed = Bed::build(slow(&["one", "two"], 40 * SECOND));
    bed.until("each channel is live", |status| all_are(status, &ChannelState::Live)).await;
    assert_eq!(bed.elapsed(), 40 * SECOND);

    // The server refuses the token, so the plugin runs from second 45 to second 85. The
    // light request of each idle channel is due in that time.
    bed.server.refuse_path("/api/");
    sleep(5 * SECOND).await;
    let refused = bed.fetch(RequestKind::Get, "/api/v1/pods").await.expect_err("no token in the wait of a request");
    assert_eq!((refused.class(), bed.elapsed()), (ErrorClass::Auth, 47 * SECOND), "{refused}");
    sleep(37 * SECOND).await;
    assert_eq!(tokens_of_light_requests(&bed), [(); 3].map(|()| Some("Bearer one".to_owned())));

    // The new token arrives. Each channel sends the light request that was due.
    sleep(2 * SECOND).await;
    let mut tokens = tokens_of_light_requests(&bed);
    assert_eq!(tokens.split_off(3), [(); 3].map(|()| Some("Bearer two".to_owned())));
    let status = bed.cluster.status();
    assert!(all_are(&status, &ChannelState::Live), "{status:?}");
    assert_eq!(bed.server.dials(), 3, "no channel lost its connection");
}

/// A token that the server refuses, and no request of a caller: only the light request can
/// tell. Its HTTP 401 goes to the credential, as the HTTP 401 of a request does, and the
/// channel ends the connection in the clean way, so a watch that is open continues.
#[tokio::test(start_paused = true)]
async fn an_http_401_to_a_light_request_goes_to_the_credential_and_an_open_watch_continues() {
    let mut bed = Bed::start(Some(&["one", "two", "three"])).await;
    let mut events = bed.send(RequestKind::Watch, "/watch").await.expect("a watch").into_body();
    let dials = bed.server.dials();

    // The server refuses each token from now on. A wake makes each channel test its connection.
    bed.server.refuse_path("/version");
    bed.cluster.signal().raise(Cause::Wake);
    let refused = bed
        .until("a failed credential", |status| {
            let state = status.credential.as_ref().map(|credential| &credential.state);
            matches!(state, Some(CredentialState::Failed { failure: CredentialFailure::Refused, .. }))
        })
        .await;
    let credential = refused.credential.expect("a credential");
    assert_eq!((credential.plugin_runs, credential.proofs), (2, 1), "one more plugin run, then one proof");
    // A report counts while its token is the token of the cluster: one at least for each token.
    assert!(credential.unauthorized >= 2, "{} reports", credential.unauthorized);
    assert!(bed.elapsed() < 2 * SECOND, "the failure showed after {:?}", bed.elapsed());
    let seen = tokens_of_light_requests(&bed);
    assert!(seen.iter().all(Option::is_some), "a light request went out with no token: {seen:?}");

    // The stream of the watch is on the old connection of its channel.
    bed.server.send_event("event\n");
    let event = timeout(SECOND, events.frame()).await.expect("an event in time").expect("a frame").expect("data");
    assert_eq!(event.into_data().expect("data"), "event\n");

    // With no token, no channel sends a request. Each one keeps a connection and waits.
    let status =
        bed.until("each channel waits for a token", |status| all_are(status, &ChannelState::WaitsForToken)).await;
    for channel in first_channels(&status) {
        assert_eq!(channel.last_error.as_deref(), Some("the server refused the light request (HTTP 401)"));
    }
    let (requests, connections) = (bed.server.seen().len(), bed.server.dials());
    // Each channel connects again 3 times at most: the pause of a channel grows, and the
    // proof is done before a fourth connect.
    assert!(connections - dials <= 9, "{} connections for two refused tokens", connections - dials);
    sleep(SECOND).await;
    assert_eq!((bed.server.seen().len(), bed.server.dials()), (requests, connections), "a request with no token");
}
