//! The egui widgets and views of fresnel.
//!
//! [`Shell`] is the entry: it draws one frame from a copy of the state of the core and
//! returns the commands of the user. A view test gives it a copy that the test made.
//!
//! This crate uses the object types of `k8s-openapi` and never the types of kube. Thus a
//! kube upgrade does not change the UI.
//!
//! - `shell`: the layout of the window. `views`: the content of each tab.
//! - `widgets`: small parts. `theme`: colors and sizes. `format`: texts for numbers and times.

#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "new data can arrive during a frame, and a bad index stops the UI thread with all forwards and terminals"
)]

mod format;
mod shell;
#[cfg(test)]
mod testing;
mod theme;
mod views;
mod widgets;

pub use format::Clock;
pub use shell::Shell;
