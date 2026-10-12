//! The test kit of fresnel: the fault proxy, the helper for k3s in Docker and a fake API server.
//!
//! Only tests use this crate. No other crate has it as a normal dependency.
//!
//! - [`FaultProxy`] makes network faults between a client and a server.
//! - [`K3s`] runs a real cluster in Docker and gives the port of its kube-apiserver.

mod background;
mod error;
mod k3s;
mod proxy;

pub use error::KitError;
pub use k3s::K3s;
pub use proxy::{FaultProxy, ProxyCounters, ProxyMode};
