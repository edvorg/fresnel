//! The stores on a real kube-apiserver: k3s in Docker behind the fault proxy.
//!
//! Each test makes faults of the network or of the server, and asserts a recovery time
//! and "the store is equal to the API" after each one (plan section 13.2).
//!
//! - `bed`: the cluster of a test and the app before it. `api`: the reads and the writes of
//!   the test itself, which use no proxy.
//! - `faults`: resets, a restart of the server, a silent connection and the renewal of
//!   each watch. `breaks`: a network with no answer for 30 seconds, 2 minutes and 10 minutes.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod api;
mod bed;
mod breaks;
#[path = "../common/mod.rs"]
mod common;
mod faults;
#[path = "../common/stores.rs"]
mod store_bed;
