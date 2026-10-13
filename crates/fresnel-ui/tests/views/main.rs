//! The views on a fake core, with no window.
//!
//! A view is a function of a copy of the state of the core. Each test builds that copy by
//! hand, for each state that the real core can have, and reads the frame through the
//! accessibility tree.
//!
//! - `fake`: the fake core and the harness. `data`: the objects of the k3s fixture.
//! - `connections`, `journal`, `pods`, `shell`: the tests.
//! - `snapshots`: PNG files of the views, for a look by a person.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod connections;
mod data;
mod fake;
mod journal;
mod pods;
mod shell;
mod snapshots;
