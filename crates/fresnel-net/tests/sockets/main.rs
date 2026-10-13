//! The connection layer on real sockets: the fake API server of the test kit, most often
//! behind the fault proxy. These tests use real time, and each one waits for a state of the
//! cluster or for a counter of the kit.
//!
//! - `acceptance`: the network numbers of milestone M0.
//! - `credential`: the real plugin process. `upgrade`: the warm upgrade connections.
//! - `environment`: the login shell resolve with a real shell process.
//! - `recovery`: a second fault during a recovery.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod acceptance;
mod common;
mod credential;
mod environment;
mod recovery;
mod upgrade;
