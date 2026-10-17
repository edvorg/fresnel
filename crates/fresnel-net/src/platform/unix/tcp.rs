//! The local listener and the reset of a local connection on unix (`local`).

use std::io;
use std::os::fd::AsRawFd as _;

use nix::libc;
use socket2::Socket;
use tokio::net::TcpStream;

/// The first bind of a listener on macOS: with no `SO_REUSEADDR`, so that a listener of an
/// other program on `0.0.0.0` makes the port "in use". With the option, macOS gives the app
/// the port and the connections of that program (port forward prototype, P7).
#[cfg(target_vendor = "apple")]
pub(crate) fn own_port(socket: &Socket) -> io::Result<()> {
    socket.set_reuse_address(false)
}

/// The first bind of a listener on Linux: with `SO_REUSEADDR`. Linux refuses each bind next
/// to a listener, with the option too, and lets a bind take a port of old connections in
/// time-wait only when each of them has the option: they get it from their listener.
#[cfg(not(target_vendor = "apple"))]
pub(crate) fn own_port(socket: &Socket) -> io::Result<()> {
    socket.set_reuse_address(true)
}

/// The second bind, for a port that only old connections in time-wait hold.
pub(crate) fn share_port(socket: &Socket) -> io::Result<()> {
    socket.set_reuse_address(true)
}

/// The bytes in the send buffer: not sent yet, or sent with no acknowledgment of the client.
#[cfg(target_vendor = "apple")]
pub(crate) fn unacknowledged(stream: &TcpStream) -> io::Result<usize> {
    let mut bytes: libc::c_int = 0;
    let mut size = libc::socklen_t::try_from(size_of::<libc::c_int>()).unwrap_or(libc::socklen_t::MAX);
    // SAFETY: the descriptor is open while `stream` lives, and `bytes` and `size` live for the
    // call. `SO_NWRITE` writes one `c_int`, and `size` tells that room.
    let result = unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_NWRITE, (&raw mut bytes).cast(), &raw mut size)
    };
    match result {
        0 => Ok(usize::try_from(bytes).unwrap_or_default()),
        _ => Err(io::Error::last_os_error()),
    }
}

/// The bytes in the send buffer: not sent yet, or sent with no acknowledgment of the client.
#[cfg(not(target_vendor = "apple"))]
pub(crate) fn unacknowledged(stream: &TcpStream) -> io::Result<usize> {
    let mut bytes: libc::c_int = 0;
    // SAFETY: the descriptor is open while `stream` lives, and `bytes` lives for the call.
    // `TIOCOUTQ` writes one `c_int`.
    let result = unsafe { libc::ioctl(stream.as_raw_fd(), libc::TIOCOUTQ, &raw mut bytes) };
    match result {
        0 => Ok(usize::try_from(bytes).unwrap_or_default()),
        _ => Err(io::Error::last_os_error()),
    }
}
