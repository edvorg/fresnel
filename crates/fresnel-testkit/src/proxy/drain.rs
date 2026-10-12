//! The wait before a reset, so that the reset arrives.
//!
//! The reset is a close with linger 0. On the loopback of macOS, such a reset can be lost
//! when bytes of the proxy are still on the way. The peer then waits with no end, which is a
//! different fault. Measured with no wait: 3 of 400 resets during a transfer were lost in
//! one run, and 11 of 200 in a second run. Thus the proxy first waits until the peer
//! acknowledged all bytes.

use std::io;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::{Instant, sleep};

/// The longest wait. A peer that does not read gets its reset then.
const LIMIT: Duration = Duration::from_secs(2);
/// The kernel gives no signal for an empty send buffer, so the proxy reads its size again.
const STEP: Duration = Duration::from_millis(2);

/// Waits until the peer acknowledged all bytes of this socket. The caller writes no more bytes.
pub(super) async fn drain(socket: &TcpStream) -> io::Result<()> {
    let limit = Instant::now() + LIMIT;
    while unacknowledged_bytes(socket)? > 0 && Instant::now() < limit {
        sleep(STEP).await;
    }
    Ok(())
}

/// The bytes in the send buffer of the socket: not sent, or sent and not acknowledged.
#[cfg(target_vendor = "apple")]
fn unacknowledged_bytes(socket: &TcpStream) -> io::Result<usize> {
    use std::os::fd::AsRawFd;

    let mut bytes: libc::c_int = 0;
    let mut size = size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `bytes` and `size` live for the call, and SO_NWRITE writes one `c_int`.
    let result = unsafe {
        libc::getsockopt(socket.as_raw_fd(), libc::SOL_SOCKET, libc::SO_NWRITE, (&raw mut bytes).cast(), &raw mut size)
    };
    if result == 0 { Ok(usize::try_from(bytes).unwrap_or(0)) } else { Err(io::Error::last_os_error()) }
}

/// Only the loopback of macOS loses such a reset, so other systems do not wait.
#[cfg(not(target_vendor = "apple"))]
fn unacknowledged_bytes(_socket: &TcpStream) -> io::Result<usize> {
    Ok(0)
}
