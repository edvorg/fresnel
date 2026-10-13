//! The clusters of `fresnel-core` on real sockets: the fake API server of the test kit.
//! These tests use real time, and each one waits for a signal of the code.
//!
//! - `app`: the app core, started as from Finder.
//! - `gateway`: the six functions of the gateway.
//! - `registry`: the list of the clusters and their connections.
//! - `stores`: the stores of a cluster and their watch loops.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod app;
mod bed;
#[path = "../common/mod.rs"]
mod common;
mod gateway;
mod registry;
#[path = "../common/stores.rs"]
mod store_bed;
mod stores;
