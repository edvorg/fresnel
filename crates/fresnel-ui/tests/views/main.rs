//! The views on a fake core, with no window.
//!
//! A view is a function of a copy of the state of the core. Each test builds that copy by
//! hand, for each state that the real core can have, and reads the frame through the
//! accessibility tree.
//!
//! - `fake`: the fake core and the harness. `data`: the objects of the k3s fixture.
//! - `connections`, `journal`, `kinds`, `pods`, `shell`: the tests.
//! - `keys`: the keys of Secrets and ConfigMaps, and the dialog of a new Secret.
//! - `search`: the quick search. `navigation`: Back, Forward, keys, menus and filters.
//! - `workloads`: the views, actions, dialogs and jobs of the workloads (M3).
//! - `metrics`: the CPU and memory columns, the Metrics tab and the sampler (M6).
//! - `details`: the views of the kinds of the Cluster, Config, Network and Access control
//!   groups (M7).
//! - `frame_time`: the time of a frame with 10,000 pods. Its limit holds in a release build.
//! - `snapshots`: PNG files of the views, for a look by a person.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod connections;
mod data;
mod details;
mod fake;
mod frame_time;
mod journal;
mod keys;
mod kinds;
mod metrics;
mod navigation;
mod pods;
mod search;
mod shell;
mod snapshots;
mod workloads;
