//! The workloads on the fake core (milestone M3): the detail view of each kind, each action
//! with its dialog and its order, the permissions, the bulk actions and the Operations tab.
//!
//! - `scene`: the objects, the permissions and the jobs. `detail`: the view of each kind.
//! - `actions`: each action from the header, a menu, a key and the quick search.
//! - `permissions`: a forbidden action is off with its reason. `bulk`: the selected rows.
//! - `changes`: the store changes while a dialog asks.
//! - `operations`: each state of a job in the Operations tab. `snapshots`: PNG files.

mod actions;
mod bulk;
mod changes;
mod detail;
mod operations;
mod permissions;
pub(crate) mod scene;
mod snapshots;
