//! The detail sections of the kinds of the Cluster group (plan section 11.3): Node,
//! Namespace and Event. The content of each one is the union of Headlamp and Lens
//! (`research/01` section 3.4.1, `research/02` section 4.5).
//!
//! - `node`: the addresses, the taints, the system, the capacity and the pods of a Node.
//! - `namespace`: the state, the quotas, the limit ranges and the count of the objects of
//!   each kind of a Namespace.
//! - `event`: an Event with its object as a link. `problem`: the state now of the problem of
//!   a warning Event, which the overview, the Events page and each panel show.

pub(crate) mod event;
pub(crate) mod namespace;
pub(crate) mod node;
pub(crate) mod problem;
