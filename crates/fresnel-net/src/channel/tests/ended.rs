//! A request that waits for its answer when the channel ends its connection.

use super::*;

/// The channel ends a connection whose light request gets no answer, and the requests that
/// wait on it end with it. hyper names such a request "dispatch task is gone: runtime dropped
/// the dispatch task", which reads as a fault of the app: a soak showed that text in each
/// store after a server that stopped answering. The error tells why the connection ended.
#[tokio::test(start_paused = true)]
async fn a_request_on_a_connection_that_the_channel_ends_tells_why_it_ended() {
    let server = MemoryServer::new();
    server.mute_path("/api/v1/pods");
    let mut bed = Bed::on(server, settings());
    bed.until_state(ChannelState::Live).await;
    let fault = async {
        sleep(SECOND).await;
        bed.server.silence_open();
        bed.signal.raise(Cause::Wake);
    };
    let (answer, ()) = tokio::join!(bed.open("/api/v1/pods"), fault);
    let error = answer.err().expect("no answer");
    let text = "watch channel: the connection ended before the answer: no answer to the light request in 3s";
    assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Network, text));
    assert_eq!(bed.elapsed(), 4 * SECOND);
}
