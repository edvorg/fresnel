//! The domain of fresnel: clusters, the watch loop, stores, search, operations and the journal.
//!
//! - [`app`]: the app core, which the window reads and commands. [`commands`]: the table of
//!   the commands of the user, with their keys.
//! - [`cluster`]: the clusters and the requests to them. [`kubeconfig`]: their source.
//! - [`kinds`]: the registry of the resource types. [`store`]: the objects of each kind of
//!   a cluster, which the watch loop keeps current. [`index`]: the relations between them.
//! - [`manifest`]: the YAML of one object, read from the server. [`search`]: the quick
//!   search in the stores.
//! - [`published`]: the one type that publishes a state to its readers.
//! - [`journal`]: the list of events, warnings and errors.
//! - [`config`]: the settings file of the user. [`state_file`]: what the app keeps from one
//!   run to the next.
//!
//! This crate has no dependency on egui. Only this crate and the programs can use kube, and
//! in this crate only `cluster/gateway.rs` holds a kube client.

pub mod app;
pub mod cluster;
pub mod commands;
pub mod config;
mod error;
#[cfg(test)]
mod fixture;
pub mod index;
pub mod journal;
pub mod kinds;
pub mod kubeconfig;
pub mod manifest;
pub mod published;
pub mod search;
pub mod state_file;
pub mod store;
mod watch;

pub use error::CoreError;
/// The connection layer. The crates above reach its types through this name.
pub use fresnel_net as net;
