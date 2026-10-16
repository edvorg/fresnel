//! The clusters of the app. One cluster is one kubeconfig context.
//!
//! - [`Registry`]: the list of the clusters, with the connected parts of each one.
//! - [`Gateway`]: the requests to one cluster. It is the only module with a kube client.
//! - `link`: the way from a kubeconfig entry to the connection layer.
//! - [`ClusterVersion`]: the version of the API server of a connected cluster.

mod gateway;
mod link;
mod registry;
mod version;

use std::fmt::{self, Display};
use std::sync::Arc;

pub use gateway::{Fate, Gateway, LogStream, WatchEvent, WatchStream, WriteError};
pub use registry::{ClusterView, Link, LinkState, Registry};
pub use version::{ClusterVersion, ServerVersion, VersionState};

/// The identity of one cluster in the app: the name of its kubeconfig context.
///
/// Each object, request and task carries this value (design rule 8). No "current cluster"
/// exists.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClusterId(Arc<str>);

impl ClusterId {
    /// The identity of the cluster of one context.
    pub fn new(context: impl Into<Arc<str>>) -> Self {
        Self(context.into())
    }

    /// The name of the context.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ClusterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
