//! The connection layer of fresnel.
//!
//! This crate has no dependency on kube, so a kube upgrade cannot change it.
//! It is the lowest crate of the workspace. Thus it also holds [`task`], the one way to
//! start a background task, which each crate above it uses.

pub mod task;
