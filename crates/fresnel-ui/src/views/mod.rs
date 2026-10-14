//! The views: the pages of the center of the window and the tabs of the dock.
//!
//! - `objects`: the page of the objects of one kind. `kinds`: the table of each kind.
//! - `pods`: the table and the sections of the Pods. `detail`: the detail panel.
//! - `keys`: the keys of a Secret or a ConfigMap, and the dialog of a new Secret.
//! - `workloads`: the sections of the workloads. `pod_spec`: the containers and volumes.
//! - `act`: the actions on objects and their dialogs. `operations`: the jobs of the core.
//! - `connections`, `journal`: the pages of the app itself. `search`: the quick search.

pub(crate) mod act;
pub(crate) mod connections;
pub(crate) mod detail;
pub(crate) mod journal;
pub(crate) mod keys;
pub(crate) mod kinds;
pub(crate) mod objects;
pub(crate) mod operations;
pub(crate) mod pod_spec;
pub(crate) mod pods;
pub(crate) mod search;
pub(crate) mod workloads;
