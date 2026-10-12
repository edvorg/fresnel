//! The egui widgets and views of fresnel.
//!
//! This crate uses the object types of `k8s-openapi` and never the types of kube. Thus a
//! kube upgrade does not change the UI.

#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "new data can arrive during a frame, and a bad index stops the UI thread with all forwards and terminals"
)]
