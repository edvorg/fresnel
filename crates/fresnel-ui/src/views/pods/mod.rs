//! The Pods: the model of their table, with the status of each pod as `kubectl get pods`
//! prints it (plan section 11.2), and the sections of the detail panel of a pod. The page
//! of the Pods is the object page of each kind (`views/objects`).
//!
//! - `rows`: the rows of the table. `detail`: the sections of the panel of a pod.

pub(crate) mod detail;
mod rows;

pub(crate) use rows::PodModel;
