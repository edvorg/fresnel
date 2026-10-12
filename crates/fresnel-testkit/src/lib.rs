//! The test kit of fresnel: the fault proxy, the helper for k3s in Docker and a fake API server.
//!
//! Only tests use this crate. No other crate has it as a normal dependency.
//!
//! - [`FaultProxy`] makes network faults between a client and a server.
//! - [`K3s`] runs a real cluster in Docker and gives the port of its kube-apiserver.
//! - [`FakeApiServer`] is a local server for the faults that a real cluster cannot make.

mod background;
mod error;
mod fake;
mod k3s;
mod proxy;

pub use error::KitError;
pub use fake::{FakeApiServer, FakeCounters, Fault, SeenRequest};
pub use k3s::K3s;
pub use proxy::{FaultProxy, ProxyCounters, ProxyMode};
