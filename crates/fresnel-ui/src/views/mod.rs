//! The views: the pages of the center of the window and the tabs of the dock.
//!
//! - `objects`: the page of the objects of one kind. `kinds`: the table of each kind.
//! - `pods`: the table and the sections of the Pods. `detail`: the detail panel.
//! - `connections`, `journal`: the pages of the app itself.

pub(crate) mod connections;
pub(crate) mod detail;
pub(crate) mod journal;
pub(crate) mod kinds;
pub(crate) mod objects;
pub(crate) mod pods;
