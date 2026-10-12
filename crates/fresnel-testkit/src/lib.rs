//! The test kit of fresnel: the fault proxy, the helper for k3s in Docker and a fake API server.
//!
//! Only tests use this crate. No other crate has it as a normal dependency.
//!
//! - [`FaultProxy`] makes network faults between a client and a server.

mod background;
mod error;
mod proxy;

pub use error::KitError;
pub use proxy::{FaultProxy, ProxyCounters, ProxyMode};
