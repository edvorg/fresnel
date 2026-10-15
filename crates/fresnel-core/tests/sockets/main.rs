//! The clusters of `fresnel-core` on real sockets: the fake API server of the test kit.
//! These tests use real time, and each one waits for a signal of the code.
//!
//! - `app`: the app core, started as from Finder.
//! - `gateway`: the six functions of the gateway. `manifest`: the YAML of one object.
//! - `ops`: the fates of the writes of the operations. `run_again`: "run again" of a Job
//!   at its worst moment, between the delete of the old Job and the create of the new one.
//! - `registry`: the list of the clusters and their connections.
//! - `stores`: the stores of a cluster and their watch loops. `store_faults`: the stores with
//!   a second fault during a recovery, a fault at the worst moment, a slow server and bad data.
//!   `demand`: the stores on demand.
//! - `indexes`: the indexes of a cluster on its stores.
//! - `metrics`: the reads of the metrics sampler and the state of each refusal.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod app;
mod bed;
#[path = "../common/mod.rs"]
mod common;
mod demand;
mod gateway;
mod indexes;
mod manifest;
mod metrics;
mod ops;
mod registry;
mod run_again;
#[path = "../common/stores.rs"]
mod store_bed;
mod store_faults;
mod stores;
