//! The clusters of the app: one entry for each context of the kubeconfig files.
//!
//! The registry owns the connections of each cluster. A reader gets a copy of each state,
//! and no call holds the lock of the list across a wait or a file read.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fresnel_net::env::ProcessEnv;
use fresnel_net::probe::Probe;
use fresnel_net::task::Tasks;
use fresnel_net::{Cause, ClusterStatus, NetSettings};
use tokio::runtime::Handle;

use super::gateway::Gateway;
use super::{ClusterId, link};
use crate::error::CoreError;
use crate::kubeconfig::{Access, Context};

/// The connected parts of one cluster. A clone is one more handle; the connections close
/// when the registry and each other owner dropped its handle.
#[derive(Clone, Debug)]
pub struct Link {
    /// The connection layer: its states, its signal and the warm upgrade connections.
    pub net: fresnel_net::Cluster,
    /// The requests to the cluster.
    pub gateway: Gateway,
}

/// The state of the connections of one cluster, as a copy.
#[derive(Clone, Debug)]
pub enum LinkState {
    /// The app holds no connection to the cluster.
    Off,
    /// The cluster is connected. The status tells the state of each channel.
    On(Box<ClusterStatus>),
    /// The last connect did not start, for example because of a bad kubeconfig entry.
    Failed(CoreError),
}

/// A copy of one cluster of the registry.
#[derive(Clone, Debug)]
pub struct ClusterView {
    /// The identity of the cluster.
    pub id: ClusterId,
    /// The kubeconfig context of the cluster.
    pub context: Context,
    /// The state of its connections.
    pub state: LinkState,
}

enum State {
    Off,
    On(Link),
    Failed(CoreError),
}

struct Entry {
    id: ClusterId,
    context: Context,
    state: State,
}

impl Entry {
    /// The connected parts, if the cluster is connected.
    fn link(&self) -> Option<Link> {
        match &self.state {
            State::On(link) => Some(link.clone()),
            State::Off | State::Failed(_) => None,
        }
    }
}

struct Inner {
    tasks: Tasks,
    runtime: Handle,
    env: ProcessEnv,
    settings: NetSettings,
    clusters: Mutex<Vec<Entry>>,
}

/// The list of the clusters. A clone is one more handle to the same list.
#[derive(Clone)]
pub struct Registry {
    inner: Arc<Inner>,
}

impl Registry {
    /// An empty registry. `tasks` starts the tasks of each cluster on `runtime`. `env` is
    /// the resolved environment for the credential plugins. `settings` are for each cluster.
    pub fn new(tasks: Tasks, runtime: Handle, env: ProcessEnv, settings: NetSettings) -> Self {
        Self { inner: Arc::new(Inner { tasks, runtime, env, settings, clusters: Mutex::new(Vec::new()) }) }
    }

    /// Takes the contexts of a load of the kubeconfig files. A cluster with the same
    /// context keeps its connections. A cluster that is gone or that changed loses them.
    pub fn set_contexts(&self, contexts: Vec<Context>) {
        let mut clusters = self.clusters();
        let mut old = std::mem::take(&mut *clusters);
        for context in contexts {
            let id = ClusterId::new(context.name.as_str());
            if clusters.iter().any(|entry| entry.id == id) {
                continue;
            }
            let same = old.iter().position(|entry| entry.id == id && entry.context == context);
            let state = same.map_or(State::Off, |index| old.swap_remove(index).state);
            clusters.push(Entry { id, context, state });
        }
        drop(clusters);
        // The connections of the other clusters close here, after the lock is free.
        drop(old);
    }

    /// Connects one cluster and gives its connected parts. A cluster that is connected
    /// gives the parts that it has. The call reads the certificate files of the kubeconfig
    /// entry, so it can block for a short time.
    pub fn connect(&self, id: &ClusterId) -> Result<Link, CoreError> {
        let (linked, access) = self.read(id, |entry| (entry.link(), entry.context.access.clone()))?;
        if let Some(link) = linked {
            return Ok(link);
        }
        let started = access.clone().and_then(|access| self.start(id, &access));
        let mut clusters = self.clusters();
        let entry = clusters.iter_mut().find(|entry| &entry.id == id).ok_or_else(|| no_cluster(id))?;
        match &entry.state {
            // A second caller was faster. The drop of `started` closes its connections.
            State::On(link) => return Ok(link.clone()),
            State::Off | State::Failed(_) if entry.context.access != access => {
                return Err(CoreError::invalid(format!("the kubeconfig entry of `{id}` changed during the connect")));
            }
            State::Off | State::Failed(_) => {}
        }
        entry.state = match &started {
            Ok(link) => State::On(link.clone()),
            Err(error) => State::Failed(error.clone()),
        };
        started
    }

    /// Closes the connections of one cluster.
    pub fn disconnect(&self, id: &ClusterId) {
        let old = self
            .clusters()
            .iter_mut()
            .find(|entry| &entry.id == id)
            .map(|entry| std::mem::replace(&mut entry.state, State::Off));
        drop(old);
    }

    /// The connected parts of one cluster. `None` for a cluster that is not connected.
    pub fn link(&self, id: &ClusterId) -> Option<Link> {
        self.read(id, Entry::link).ok().flatten()
    }

    /// A copy of each cluster with its state, in the sequence of the kubeconfig files.
    pub fn snapshot(&self) -> Vec<ClusterView> {
        // The status of a cluster is read outside the lock of the list.
        let copy = |entry: &Entry| {
            let failure = match &entry.state {
                State::Failed(error) => Some(error.clone()),
                State::Off | State::On(_) => None,
            };
            (entry.id.clone(), entry.context.clone(), entry.link(), failure)
        };
        let entries: Vec<_> = self.clusters().iter().map(copy).collect();
        let view = |(id, context, link, failure): (ClusterId, Context, Option<Link>, Option<CoreError>)| {
            let state = match (link, failure) {
                (Some(link), _) => LinkState::On(Box::new(link.net.status())),
                (None, Some(error)) => LinkState::Failed(error),
                (None, None) => LinkState::Off,
            };
            ClusterView { id, context, state }
        };
        entries.into_iter().map(view).collect()
    }

    /// A probe for the endpoint of one cluster, with the connect settings of the registry.
    /// The `doctor` command measures with it. The call reads the certificate files of the
    /// kubeconfig entry.
    pub fn probe(&self, id: &ClusterId) -> Result<Probe, CoreError> {
        let access = self.read(id, |entry| entry.context.access.clone())??;
        let spec = link::cluster_spec(id, &access, &self.inner.env, self.inner.settings.clone())?;
        Ok(Probe::new(&spec.endpoint, spec.settings.connect)?)
    }

    /// Raises the signal of each connected cluster: a wake, a network change or "retry now".
    pub fn raise(&self, cause: Cause) {
        let links: Vec<Link> = self.clusters().iter().filter_map(Entry::link).collect();
        links.iter().for_each(|link| link.net.signal().raise(cause));
    }

    fn start(&self, id: &ClusterId, access: &Access) -> Result<Link, CoreError> {
        let inner = &self.inner;
        let spec = link::cluster_spec(id, access, &inner.env, inner.settings.clone())?;
        let net = fresnel_net::Cluster::start(&inner.tasks, spec)?;
        let gateway = Gateway::new(id.clone(), net.clone(), inner.settings.limits.clone(), &inner.runtime);
        Ok(Link { net, gateway })
    }

    fn read<T>(&self, id: &ClusterId, read: impl FnOnce(&Entry) -> T) -> Result<T, CoreError> {
        self.clusters().iter().find(|entry| &entry.id == id).map(read).ok_or_else(|| no_cluster(id))
    }

    /// Each use holds the lock for one small change or one copy.
    fn clusters(&self) -> MutexGuard<'_, Vec<Entry>> {
        self.inner.clusters.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry").field("clusters", &self.clusters().len()).finish_non_exhaustive()
    }
}

fn no_cluster(id: &ClusterId) -> CoreError {
    CoreError::invalid(format!("the kubeconfig files have no context `{id}`"))
}
