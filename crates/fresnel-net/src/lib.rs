//! The connection layer of fresnel.
//!
//! This crate has no dependency on kube, so a kube upgrade cannot change it.
//! It is the lowest crate of the workspace. Thus it also holds [`task`], the one way to
//! start a background task, which each crate above it uses.

mod body;
mod changes;
mod channel;
mod clock;
mod connect;
pub mod credential;
pub mod env;
mod error;
mod process;
mod request;
mod signal;
pub mod task;
#[cfg(test)]
mod testing;
mod throttle;

pub use body::ResponseBody;
pub use changes::Changes;
pub use channel::group::GroupStatus;
pub use channel::{ChannelSettings, ChannelState, ChannelStatus};
pub use clock::WallClock;
pub use connect::{ConnectSettings, ConnectStatus, Endpoint};
pub use error::{ErrorClass, NetError};
pub use request::{Limits, RequestKind};
pub use signal::{Cause, Listener, Signal};
pub use throttle::{ThrottleSettings, ThrottleStatus};
