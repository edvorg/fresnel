//! The domain of fresnel: clusters, the watch loop, stores, search, operations and the journal.
//!
//! - [`app`]: the app core, which the window reads and commands.
//! - [`cluster`]: the clusters and the requests to them. [`kubeconfig`]: their source.
//! - [`journal`]: the list of events, warnings and errors.
//!
//! This crate has no dependency on egui. Only this crate and the programs can use kube, and
//! in this crate only `cluster/gateway.rs` holds a kube client.

pub mod app;
pub mod cluster;
mod error;
pub mod journal;
pub mod kubeconfig;

pub use error::CoreError;
/// The connection layer. The crates above reach its types through this name.
pub use fresnel_net as net;
