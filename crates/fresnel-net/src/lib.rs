//! The connection layer of fresnel (plan section 8).
//!
//! [`Cluster`] is the entry: one value for each cluster. It owns the connections, runs the
//! credential plugin, and gives a tower service and copies of each state.
//!
//! This crate has no dependency on kube, so a kube upgrade cannot change it.
//! It is the lowest crate of the workspace. Thus it also holds [`task`], the one way to
//! start a background task, which each crate above it uses.

mod body;
mod changes;
mod channel;
mod clock;
mod cluster;
mod connect;
pub mod credential;
pub mod detect;
pub mod env;
mod error;
mod light;
mod platform;
pub mod probe;
mod process;
mod request;
mod signal;
pub mod task;
#[cfg(test)]
mod testing;
mod throttle;
mod upgrade;

pub use body::ResponseBody;
pub use changes::Changes;
pub use channel::group::GroupStatus;
pub use channel::outage::{OutageSettings, OutageStatus};
pub use channel::{ChannelSettings, ChannelState, ChannelStatus};
pub use clock::WallClock;
pub use cluster::{Cluster, ClusterSpec, ClusterStatus, NetSettings};
pub use connect::dial::Protocol;
pub use connect::{ConnectSettings, ConnectStatus, Endpoint};
pub use error::{ErrorClass, NetError};
pub use request::{Limits, RequestKind};
pub use signal::{Cause, Listener, Signal};
pub use throttle::{ThrottleSettings, ThrottleStatus};
pub use upgrade::{UpgradeSettings, UpgradeStatus, WarmGuard};
