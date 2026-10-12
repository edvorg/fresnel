//! One connection of the proxy: the wait, the connect to the server and the two directions.

use std::io;
use std::time::Duration;

use socket2::SockRef;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::sleep;

use super::drain::drain;
use super::pipe::{Gate, pipe};
use super::state::Admission;

/// The sockets of one connection. Each one closes with a reset when it drops, until it
/// sent its end of data. Thus each end that is not clean gives the two peers an error.
struct Sockets {
    client: TcpStream,
    server: Option<TcpStream>,
}

/// How a connection ended with no error of a socket.
enum End {
    /// The two sides closed, or the server did not accept the connect.
    Closed,
    /// A reset order of the driver.
    Reset,
}

/// Serves one client connection until the two sides closed, an error, or a reset order.
pub(super) async fn serve(client: TcpStream, admission: Admission, reset_order: watch::Receiver<u64>) {
    let ended = until_end(client, &admission, reset_order).await;
    admission.shared.count(|counters| match ended {
        Ok(End::Closed) => {}
        Ok(End::Reset) => counters.reset += 1,
        Err(_socket_error) => counters.broken += 1,
    });
}

/// The sockets close when this function returns. Thus a counter changes after the close.
async fn until_end(client: TcpStream, admission: &Admission, mut reset_order: watch::Receiver<u64>) -> io::Result<End> {
    // Before the first wait: a reset order can come before the relay ran one time.
    reset_on_close(&client)?;
    let mut sockets = Sockets { client, server: None };
    tokio::select! {
        carried = relay(&mut sockets, admission) => carried.map(|()| End::Closed),
        Ok(()) = reset_order.changed() => sockets.drain().await.map(|()| End::Reset),
    }
}

async fn relay(sockets: &mut Sockets, admission: &Admission) -> io::Result<()> {
    let shared = &admission.shared;
    let gate = Gate::new(shared, admission.number);
    sleep(admission.wait).await;
    gate.pass().await;
    let Ok(connected) = TcpStream::connect(shared.upstream).await else {
        shared.count(|counters| counters.refused += 1);
        return Ok(());
    };
    let server = sockets.server.insert(connected);
    reset_on_close(server)?;
    let (client_read, client_write) = sockets.client.split();
    let (server_read, server_write) = server.split();
    tokio::try_join!(pipe(client_read, server_write, gate), pipe(server_read, client_write, gate))?;
    Ok(())
}

/// Small writes go out with no delay, so that the proxy adds no latency of its own.
fn reset_on_close(socket: &TcpStream) -> io::Result<()> {
    socket.set_nodelay(true)?;
    SockRef::from(socket).set_linger(Some(Duration::ZERO))
}

impl Sockets {
    /// Prepares the reset of the two sockets. They drop when the task ends.
    async fn drain(&self) -> io::Result<()> {
        let client = drain(&self.client).await;
        match &self.server {
            Some(server) => client.and(drain(server).await),
            None => client,
        }
    }
}
