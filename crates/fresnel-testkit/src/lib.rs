//! The test kit of fresnel: the fault proxy, the helper for k3s in Docker, a fake API server
//! and a home folder for a test.
//!
//! Only tests use this crate. No other crate has it as a normal dependency.
//!
//! - [`FaultProxy`] makes network faults between a client and a server.
//! - [`K3s`] runs a real cluster in Docker and gives the port of its kube-apiserver.
//! - [`FakeApiServer`] is a local server for the faults that a real cluster cannot make.
//! - [`Home`] is a home folder with a shell profile, for a start as from Finder.

mod background;
mod error;
mod fake;
mod home;
mod k3s;
mod proxy;

pub use error::KitError;
pub use fake::{FakeApiServer, FakeCounters, Fault, SeenRequest};
pub use home::{FINDER_PATH, Home};
pub use k3s::{K3s, K3sOptions, PodLine};
pub use proxy::{FaultProxy, ProxyCounters, ProxyMode};
