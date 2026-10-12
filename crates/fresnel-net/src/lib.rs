//! The connection layer of fresnel.
//!
//! This crate has no dependency on kube, so a kube upgrade cannot change it.
//! It is the lowest crate of the workspace. Thus it also holds [`task`], the one way to
//! start a background task, which each crate above it uses.

mod changes;
mod clock;
mod error;
mod signal;
pub mod task;

pub use changes::Changes;
pub use clock::WallClock;
pub use error::{ErrorClass, NetError};
pub use signal::{Cause, Listener, Signal};
