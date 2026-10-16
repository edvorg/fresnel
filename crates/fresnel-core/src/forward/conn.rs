//! One local connection of a forward: it waits for a target, opens a tunnel, carries the
//! bytes in each direction and ends with a clear signal for the client (plan section 12.3).
//!
//! A connection that arrives during a break waits for the wait time, 5 s by default, and
//! then gets a reset. A tunnel that breaks gives the client a reset too, never a silent end:
//! only a clean end of the pod gives the end of the data. The two directions run at the same
//! time, so a pod that waits for the app to read its answer never stops the upload.
//!
//! The WebSocket protocol has no half close. After the end of the local input, the tunnel
//! stays while the pod sends, and closes after the pod is silent for the linger time, 1 s,
//! as kubectl (port forward prototype, problem 7 and R8).

use std::sync::Arc;

use fresnel_net::local::LocalStream;
use fresnel_net::{SocketEnd, SocketFailure};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::watch;
use tokio::time::{Instant, sleep, timeout_at};

use super::shared::{Gate, Outcome, Shared};
use crate::stream::{Tunnel, TunnelEnd};

/// The largest read of a local client: one message of the tunnel.
const READ_SIZE: usize = 32 * 1024;

/// How the data of a tunnel ended.
enum Down {
    /// The tunnel ended, with this end.
    Tunnel(Option<TunnelEnd>),
    /// The local client took no more bytes: it is gone.
    Client,
    /// The local input ended, and the pod was silent for the linger time.
    Quiet,
}

/// How the local input ended.
enum Up {
    End,
    /// A read failed: the client is gone.
    Client,
    /// The tunnel took no more bytes: it ended, and the other direction tells how.
    Tunnel,
}

/// Serves one local connection to its end.
pub(super) async fn serve(mut local: LocalStream, shared: Arc<Shared>, mut gate: watch::Receiver<Gate>) {
    let _open = shared.counts.accepted();
    let deadline = Instant::now() + shared.wait;
    let Some(mut tunnel) = open(&shared, &mut gate, deadline).await else {
        shared.counts.reset();
        local.reset().await;
        return;
    };
    let end = carry(&mut local, &mut tunnel, &shared).await;
    match end {
        Down::Tunnel(Some(TunnelEnd::Socket(end))) if end.is_clean() => {
            shared.report(Outcome::Clean);
            local.finish().await;
        }
        Down::Tunnel(Some(TunnelEnd::Socket(end @ SocketEnd::Failed(_)))) => {
            // A ping with no answer or a broken connection: the path can be lost, so the
            // channels of the cluster test theirs now.
            let path = matches!(end, SocketEnd::Failed(SocketFailure::Silent { .. } | SocketFailure::Broken(_)));
            shared.report(Outcome::Broke { text: format!("a tunnel broke: {end}"), path });
            shared.counts.reset();
            local.reset().await;
        }
        Down::Tunnel(end) => {
            let text = match end {
                Some(TunnelEnd::Refused(text)) => format!("the node refused a tunnel: {}", text.trim()),
                Some(TunnelEnd::Invalid(text)) => format!("a tunnel broke the protocol: {text}"),
                Some(TunnelEnd::Socket(end)) => format!("a tunnel ended: {end}"),
                None => "a tunnel ended with no end of its socket".to_owned(),
            };
            shared.report(Outcome::Broke { text, path: false });
            shared.counts.reset();
            local.reset().await;
        }
        Down::Quiet => {
            tunnel.close();
            shared.report(Outcome::Clean);
            local.finish().await;
        }
        Down::Client => tunnel.close(),
    }
}

/// Opens a tunnel to the target of the gate before the deadline. A failure tries again with
/// the next target, or after a short pause. `None` at the deadline.
async fn open(shared: &Shared, gate: &mut watch::Receiver<Gate>, deadline: Instant) -> Option<Tunnel> {
    let mut waited = false;
    loop {
        if !waited && gate.borrow().target.is_none() {
            waited = true;
            shared.report(Outcome::Waits);
        }
        let (feed, target) = match timeout_at(deadline, gate.wait_for(|gate| gate.target.is_some())).await {
            Ok(Ok(open)) => open.target.clone()?,
            Ok(Err(_)) | Err(_) => {
                let wait = shared.wait.as_secs_f64();
                shared.report(Outcome::NoTarget(format!("a local connection got no tunnel in {wait:.1} s: a reset")));
                return None;
            }
        };
        match Tunnel::open(&feed.gateway, &target.pod, target.port, shared.journal.clone()).await {
            Ok(tunnel) => return Some(tunnel),
            Err(error) => shared.report(Outcome::OpenFailed { target: target.clone(), error }),
        }
        // The report can give a new target at once; else the pause keeps a refusal from a loop.
        let pause = (Instant::now() + shared.retry_pause).min(deadline);
        if timeout_at(pause, gate.changed()).await.is_err() && pause >= deadline {
            let text = "a local connection got no tunnel before the end of its wait: a reset".to_owned();
            shared.report(Outcome::NoTarget(text));
            return None;
        }
    }
}

/// Carries the bytes in each direction to the end of the tunnel or of the client.
async fn carry(local: &mut LocalStream, tunnel: &mut Tunnel, shared: &Shared) -> Down {
    let Some((mut reader, mut writer)) = local.split() else {
        return Down::Client;
    };
    let sender = tunnel.writer();
    let (input_ended, mut ended) = watch::channel(false);
    let up = async {
        let mut buffer = vec![0_u8; READ_SIZE];
        loop {
            let read = match reader.read(&mut buffer).await {
                Ok(0) => {
                    input_ended.send_replace(true);
                    return Up::End;
                }
                Ok(read) => read,
                Err(_) => return Up::Client,
            };
            let Some(data) = buffer.get(..read) else { return Up::Client };
            if sender.send(data).await.is_err() {
                return Up::Tunnel;
            }
            shared.counts.sent(read);
        }
    };
    let down = async {
        let mut last = Instant::now();
        loop {
            let quiet = async {
                // The linger time starts after the end of the input and after each message.
                if ended.wait_for(|ended| *ended).await.is_err() {
                    std::future::pending::<()>().await;
                }
                sleep((last + shared.linger).saturating_duration_since(Instant::now())).await;
            };
            let data = tokio::select! {
                data = tunnel.next() => data,
                () = quiet => return Down::Quiet,
            };
            let Some(data) = data else { return Down::Tunnel(tunnel.end()) };
            last = Instant::now();
            if writer.write_all(&data).await.is_err() {
                return Down::Client;
            }
            shared.counts.received(data.len());
        }
    };
    tokio::pin!(up, down);
    tokio::select! {
        down = &mut down => down,
        up = &mut up => match up {
            Up::Client => Down::Client,
            Up::End | Up::Tunnel => down.await,
        },
    }
}
