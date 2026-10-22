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
//! - `logs`: the log views: the tab, the pane, the search, the copy and the saves (M9).
//! - `forwards`: the Forwards tab, the dialog of a forward and its keys (M5).
//! - `details`: the views of the kinds of the Cluster, Config, Network and Access control
//!   groups (M7). `overview`: the overviews of the Cluster and the Workloads groups (M7).
//! - `settings`: the settings view, read-only clusters, themes, keys and focus (M8).
//! - `editor`: the manifest editor of the dock (M11).
//! - `terminal`: the terminals of the dock: the tab, the keys, the pointer and the screen (M10).
//! - `frame_time`: the time of a frame with 10,000 pods. Its limit holds in a release build.
//!   `timing`: the rule of the time limits, with a second pass after a step over its limit.
//! - `snapshots`: PNG files of the views, for a look by a person.
//! - `showcase`: the pictures of the README on a made-up world of three clusters
//!   (`cargo xtask screenshots`).

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod connections;
mod data;
pub(crate) mod details;
mod edges;
mod editor;
mod fake;
mod forwards;
mod frame_time;
mod journal;
mod keys;
mod kinds;
mod logs;
mod metrics;
mod navigation;
mod overview;
mod pods;
mod search;
mod settings;
mod shell;
mod showcase;
mod snapshots;
mod terminal;
mod timing;
mod workloads;
