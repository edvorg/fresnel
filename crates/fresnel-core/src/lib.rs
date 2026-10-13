//! The domain of fresnel: clusters, the watch loop, stores, search, operations and the journal.
//!
//! This crate has no dependency on egui. Only this crate and the programs can use kube, and
//! in this crate only `cluster/gateway.rs` holds a kube client.

pub mod cluster;
mod error;
pub mod kubeconfig;

pub use error::CoreError;
/// The connection layer. The crates above reach its types through this name.
pub use fresnel_net as net;
