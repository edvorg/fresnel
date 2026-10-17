//! The local side of a port forward (plan section 12.3): the listener of the app and the
//! connections of the local clients.
//!
//! `TcpListener::bind` of tokio sets `SO_REUSEADDR`. With it, macOS lets the app bind
//! `127.0.0.1:P` while an other program holds `0.0.0.0:P`, and the app takes the connections
//! of that program (port forward prototype, P7). Windows lets an other program take the port
//! of the app with the same option. So the first bind is exclusive on each system; Linux
//! refuses a port with a listener also with the option, which it needs for the next point.
//! Only a port that no program listens on, with old connections in time-wait, gets a second
//! bind that shares the port: a forward that starts again gets its port back at once.
//!
//! - `stream`: one local connection, and its close with an error for the client.

mod stream;
#[cfg(test)]
mod tests;

use std::fmt::{self, Display};
use std::future::poll_fn;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::task::Poll;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

pub use stream::LocalStream;

use crate::platform;

/// The connections that wait for their accept.
const BACKLOG: i32 = 1024;
/// A program that listens on this computer answers a connect in much less time.
const LISTENER_LIMIT: Duration = Duration::from_millis(500);
/// The time in which a connect that no program took ends with its reset.
const RESET_LIMIT: Duration = Duration::from_millis(100);
/// The tries of an automatic port: the system gives a port that is free on the first address,
/// and an other program can hold it on a later one.
const AUTOMATIC_TRIES: usize = 8;

/// Why a listener did not bind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindError {
    /// An other program listens on the port.
    InUse(SocketAddr),
    /// The system refused the address: a port below 1024 with no right, or an address of no
    /// interface. The text of the system.
    Refused {
        /// The address.
        address: SocketAddr,
        /// What the system said.
        text: String,
    },
}

impl Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InUse(address) => {
                write!(f, "the local port {} is in use: an other program listens on {address}", address.port())
            }
            Self::Refused { address, text } => write!(f, "the system refused the local address {address}: {text}"),
        }
    }
}

impl std::error::Error for BindError {}

/// The listener of one forward on one port of one or more local addresses. Its sockets close
/// when it drops; the connections that it gave stay open.
#[derive(Debug)]
pub struct LocalListener {
    sockets: Vec<TcpListener>,
    addresses: Vec<SocketAddr>,
}

impl LocalListener {
    /// Binds `port` on each address, or a free port of the system with `port` 0: the same
    /// one on each address. The call needs a runtime context.
    pub async fn bind(addresses: &[IpAddr], port: u16) -> Result<Self, BindError> {
        if port != 0 {
            return Self::bind_all(addresses, port).await;
        }
        let mut tries = 1;
        loop {
            match Self::bind_all(addresses, 0).await {
                Err(BindError::InUse(_)) if tries < AUTOMATIC_TRIES => tries += 1,
                other => return other,
            }
        }
    }

    /// Binds each address. Port 0 takes the port of the system for the first address.
    async fn bind_all(addresses: &[IpAddr], mut port: u16) -> Result<Self, BindError> {
        let mut listener = Self { sockets: Vec::new(), addresses: Vec::new() };
        for ip in addresses {
            let socket = bind_one(SocketAddr::new(*ip, port)).await?;
            let address = socket
                .local_addr()
                .map_err(|error| BindError::Refused { address: SocketAddr::new(*ip, port), text: error.to_string() })?;
            port = address.port();
            listener.sockets.push(socket);
            listener.addresses.push(address);
        }
        if listener.sockets.is_empty() {
            let address = SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port);
            return Err(BindError::Refused { address, text: "the forward has no local address".to_owned() });
        }
        Ok(listener)
    }

    /// The bound addresses, each with its port.
    pub fn addresses(&self) -> &[SocketAddr] {
        &self.addresses
    }

    /// The next connection of a client on any address.
    pub async fn accept(&self) -> io::Result<LocalStream> {
        let (stream, peer) = poll_fn(|context| {
            for socket in &self.sockets {
                if let Poll::Ready(accepted) = socket.poll_accept(context) {
                    return Poll::Ready(accepted);
                }
            }
            Poll::Pending
        })
        .await?;
        Ok(LocalStream::new(stream, peer))
    }
}

/// Binds one address: exclusive first, and shared only when no program listens there.
async fn bind_one(address: SocketAddr) -> Result<TcpListener, BindError> {
    if wildcard_taken(address) && something_listens(address).await {
        return Err(BindError::InUse(address));
    }
    match socket(address, false) {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            if address.port() == 0 || something_listens(address).await {
                return Err(BindError::InUse(address));
            }
            // Only old connections in time-wait hold the port.
            socket(address, true).map_err(|error| refusal(address, &error))
        }
        Err(error) => Err(refusal(address, &error)),
    }
}

fn refusal(address: SocketAddr, error: &io::Error) -> BindError {
    match error.kind() {
        io::ErrorKind::AddrInUse => BindError::InUse(address),
        _ => BindError::Refused { address, text: error.to_string() },
    }
}

/// True when a socket has the port of `address` on the address of each interface. Windows
/// gives the app `127.0.0.1:P` while an other program listens on `0.0.0.0:P`, and the app
/// would take the local clients of that program. Old connections in time-wait can hold the
/// port too, so only a connect that a program answers then makes the port "in use".
fn wildcard_taken(address: SocketAddr) -> bool {
    if address.port() == 0 || address.ip().is_unspecified() {
        return false;
    }
    let wildcard = match address {
        SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    };
    // The probe never listens, and it closes at once.
    matches!(bound(SocketAddr::new(wildcard, address.port()), false), Err(error) if error.kind() == io::ErrorKind::AddrInUse)
}

/// A listening socket.
fn socket(address: SocketAddr, shared: bool) -> io::Result<TcpListener> {
    let socket = bound(address, shared)?;
    socket.listen(BACKLOG)?;
    TcpListener::from_std(socket.into())
}

/// A socket bound to the address that does not listen yet. A socket of IPv6 takes IPv6 only,
/// so `::` and `0.0.0.0` are two sockets on each system.
fn bound(address: SocketAddr, shared: bool) -> io::Result<Socket> {
    let socket = Socket::new(Domain::for_address(address), Type::STREAM, Some(Protocol::TCP))?;
    if address.is_ipv6() {
        socket.set_only_v6(true)?;
    }
    match shared {
        true => platform::share_port(&socket)?,
        false => platform::own_port(&socket)?,
    }
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    Ok(socket)
}

/// True when a program accepts a connect on the address. macOS can report a connect to a
/// port of old connections in time-wait as made and reset it at once (in 4 of 25 runs of
/// the unit tests), so a connection that ends with an error at once has no program.
async fn something_listens(address: SocketAddr) -> bool {
    let Ok(Ok(stream)) = timeout(LISTENER_LIMIT, TcpStream::connect(address)).await else {
        return false;
    };
    // A connection in the queue of a listener gets no byte and no error.
    if timeout(RESET_LIMIT, stream.readable()).await.is_err() {
        return true;
    }
    let mut byte = [0_u8; 1];
    !matches!(stream.try_read(&mut byte), Err(error) if error.kind() != io::ErrorKind::WouldBlock)
}
