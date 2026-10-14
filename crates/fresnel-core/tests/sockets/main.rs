//! The clusters of `fresnel-core` on real sockets: the fake API server of the test kit.
//! These tests use real time, and each one waits for a signal of the code.
//!
//! - `app`: the app core, started as from Finder.
//! - `gateway`: the six functions of the gateway.
//! - `registry`: the list of the clusters and their connections.
//! - `stores`: the stores of a cluster and their watch loops. `store_faults`: the stores with
//!   a second fault during a recovery, a fault at the worst moment, a slow server and bad data.
//!   `demand`: the stores on demand.
//! - `indexes`: the indexes of a cluster on its stores.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod app;
mod bed;
#[path = "../common/mod.rs"]
mod common;
mod demand;
mod gateway;
mod indexes;
mod registry;
#[path = "../common/stores.rs"]
mod store_bed;
mod store_faults;
mod stores;
