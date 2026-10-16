//! The local listener and the reset of a local connection on Windows (`local`).

use std::io;
use std::os::windows::io::AsRawSocket as _;

use socket2::Socket;
use tokio::net::TcpStream;
use windows_sys::Win32::Networking::WinSock::{
    SIO_TCP_INFO, SO_EXCLUSIVEADDRUSE, SOCKET, SOCKET_ERROR, SOL_SOCKET, TCP_INFO_v0, WSAGetLastError, WSAIoctl,
    setsockopt,
};

/// The first bind of a listener: exclusive. With no such option, an other program that sets
/// `SO_REUSEADDR` can bind the port of the app and take its connections.
pub(crate) fn own_port(socket: &Socket) -> io::Result<()> {
    let on: i32 = 1;
    let size = i32::try_from(size_of::<i32>()).unwrap_or(i32::MAX);
    // SAFETY: the socket is open while `socket` lives, and `on` lives for the call. The option
    // reads one `i32`, and `size` tells it.
    let result = unsafe {
        setsockopt(raw(socket.as_raw_socket()), SOL_SOCKET, SO_EXCLUSIVEADDRUSE, (&raw const on).cast(), size)
    };
    match result {
        0 => Ok(()),
        _ => Err(last_error()),
    }
}

/// The second bind, for a port that only old connections in time-wait hold: no option.
pub(crate) fn share_port(_socket: &Socket) -> io::Result<()> {
    Ok(())
}

/// The bytes that the client did not acknowledge yet (`BytesInFlight` of `TCP_INFO_v0`).
pub(crate) fn unacknowledged(stream: &TcpStream) -> io::Result<usize> {
    let version: u32 = 0;
    let mut info = TCP_INFO_v0::default();
    let mut returned: u32 = 0;
    let (in_size, out_size) = (size_of::<u32>(), size_of::<TCP_INFO_v0>());
    // SAFETY: the socket is open while `stream` lives. The input is one `u32` and the output
    // one `TCP_INFO_v0`, each with its size, and each lives for the call. No overlapped call.
    let result = unsafe {
        WSAIoctl(
            raw(stream.as_raw_socket()),
            SIO_TCP_INFO,
            (&raw const version).cast(),
            u32::try_from(in_size).unwrap_or(u32::MAX),
            (&raw mut info).cast(),
            u32::try_from(out_size).unwrap_or(u32::MAX),
            &raw mut returned,
            std::ptr::null_mut(),
            None,
        )
    };
    match result {
        SOCKET_ERROR => Err(last_error()),
        _ => Ok(usize::try_from(info.BytesInFlight).unwrap_or(usize::MAX)),
    }
}

fn raw(socket: u64) -> SOCKET {
    usize::try_from(socket).unwrap_or(usize::MAX)
}

fn last_error() -> io::Error {
    // SAFETY: the call reads the error of this thread and has no input.
    io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
}
