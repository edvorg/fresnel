//! A real cluster for tests: k3s in a Docker container.
//!
//! The API address of this helper is the kube-apiserver of k3s. k3s publishes a different
//! port by default, its supervisor. The supervisor serves the same API with other HTTP/2
//! settings: a stream limit of 250 in place of 100, and no close of an idle connection.
//! A test on the supervisor port proves nothing about a real cluster.

mod container;
mod docker;
mod kubeconfig;
mod network;
mod pod_line;

use std::fs;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};

use crate::KitError;
use container::Container;
use network::Network;
pub use pod_line::PodLine;

/// Each container and each network of the test kit has this start of its name.
/// `docker ps --all --filter name=fresnel-test-` shows what a stopped test run left.
const NAME_PREFIX: &str = "fresnel-test-";
const K3S_KUBECONFIG: &str = "/etc/rancher/k3s/k3s.yaml";
/// The most containers that one start makes. On a CI runner, k3s sometimes ends some
/// seconds after its start, when a second cluster starts at the same moment: its kubelet
/// finds the cgroup `kubepods` with no controller. A new container then starts well.
const START_ATTEMPTS: u32 = 3;

/// What a test cluster has besides the defaults of the helper.
#[derive(Clone, Debug, Default)]
pub struct K3sOptions {
    metrics_server: bool,
    args: Vec<String>,
    /// The node name and the more arguments of each agent.
    agents: Vec<(String, Vec<String>)>,
}

impl K3sOptions {
    /// The metrics-server of k3s. The helper turns it off by default: it pulls an image at
    /// each start, and only the tests of the metrics read it.
    pub fn metrics_server(mut self) -> Self {
        self.metrics_server = true;
        self
    }

    /// The name of the node. Docker gives each container an other host name, and k3s takes
    /// it as the name of its node: two clusters have a node of the same name only with this.
    pub fn node_name(mut self, name: &str) -> Self {
        self.args.push(format!("--node-name={name}"));
        self
    }

    /// One more node: a k3s agent in a container of its own, with this node name and these
    /// more arguments of the agent, for example a taint. A drain needs a node besides the
    /// one of the server, where the pods of the cluster run.
    pub fn agent(mut self, name: &str, args: &[&str]) -> Self {
        self.agents.push((name.to_owned(), args.iter().map(|arg| (*arg).to_owned()).collect()));
        self
    }
}

/// A running k3s cluster. The drop removes the containers, the network and the files.
///
/// The fields drop in this sequence. The network can go only after its containers.
pub struct K3s {
    _agents: Vec<Container>,
    container: Container,
    _network: Network,
    files: Files,
    api: SocketAddr,
    /// The kubeconfig that k3s wrote, as the source for each kubeconfig file.
    k3s_kubeconfig: String,
}

impl K3s {
    /// Starts a cluster and waits until its API server is ready and a test can make a pod.
    /// `label` becomes a part of the container name, so that `docker ps` shows which test
    /// owns the container.
    ///
    /// A k3s that ends by itself during its start gets a new container, a limited count of
    /// times. Each such end is a line of the test output.
    pub async fn start(label: &str) -> Result<Self, KitError> {
        Self::start_with(label, K3sOptions::default()).await
    }

    /// Starts a cluster whose API server does not serve one group version of the API, for
    /// example `node.k8s.io/v1`, as an older cluster does not serve a newer kind.
    pub async fn start_without(label: &str, group_version: &str) -> Result<Self, KitError> {
        let off = format!("--kube-apiserver-arg=runtime-config={group_version}=false");
        Self::start_with(label, K3sOptions { args: vec![off], ..K3sOptions::default() }).await
    }

    /// Starts a cluster with these options, as [`K3s::start`] does.
    pub async fn start_with(label: &str, options: K3sOptions) -> Result<Self, KitError> {
        with_new_attempts(|| Self::start_one(label, &options)).await
    }

    /// One container. Its drop removes it, also after an error of this start.
    async fn start_one(label: &str, options: &K3sOptions) -> Result<Self, KitError> {
        let name = format!("{NAME_PREFIX}{label}-{:06x}", fastrand::u32(..0x0100_0000));
        let api = SocketAddr::from((Ipv4Addr::LOCALHOST, free_port()?));
        let network = Network::create(&name).await?;
        let off = (!options.metrics_server).then_some("--disable=metrics-server");
        // An agent joins the cluster with the token of its server.
        let token = format!("{:016x}", fastrand::u64(..));
        let token_arg = format!("--token={token}");
        let joins = (!options.agents.is_empty()).then_some(token_arg.as_str());
        let extra: Vec<&str> = off.into_iter().chain(joins).chain(options.args.iter().map(String::as_str)).collect();
        let container = Container::run(&name, &network, api.port(), &extra).await?;
        container.wait_ready().await?;
        let mut agents = Vec::new();
        for (number, (node, args)) in (1..).zip(&options.agents) {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let server = (network.address, token.as_str());
            agents.push(Container::agent(&format!("{name}-{node}"), &network, (number, node), server, &args).await?);
        }
        for (node, _) in &options.agents {
            container.wait_for_node(node).await?;
        }
        let k3s_kubeconfig = container.exec(&["cat", K3S_KUBECONFIG]).await?;
        let files = Files::create(&name)?;
        kubeconfig::write(&files.kubeconfig(api), &kubeconfig::with_server(&k3s_kubeconfig, api)?)?;
        Ok(Self { _agents: agents, container, _network: network, files, api, k3s_kubeconfig })
    }

    /// The name of the container.
    pub fn name(&self) -> &str {
        &self.container.name
    }

    /// The address of the kube-apiserver of the cluster, on 127.0.0.1. It stays the same
    /// after a restart.
    pub fn api_addr(&self) -> SocketAddr {
        self.api
    }

    /// The kubeconfig of the cluster admin. Its server is [`K3s::api_addr`].
    pub fn kubeconfig(&self) -> PathBuf {
        self.files.kubeconfig(self.api)
    }

    /// A kubeconfig of the cluster admin whose server is `proxy`: a fault proxy on 127.0.0.1
    /// before [`K3s::api_addr`]. The certificate of the API server is valid for 127.0.0.1.
    pub fn kubeconfig_through(&self, proxy: SocketAddr) -> Result<PathBuf, KitError> {
        let path = self.files.kubeconfig(proxy);
        kubeconfig::write(&path, &kubeconfig::with_server(&self.k3s_kubeconfig, proxy)?)?;
        Ok(path)
    }

    /// Runs `kubectl` in the container as the cluster admin and gives its output. A test
    /// uses it to change the cluster and to read the state that the app must show.
    pub async fn kubectl(&self, arguments: &[&str]) -> Result<String, KitError> {
        self.container.exec(&[&["kubectl"], arguments].concat()).await
    }

    /// Starts the container again and waits until the API server is ready.
    pub async fn restart(&self) -> Result<(), KitError> {
        self.container.restart().await
    }

    /// Stops the container. The API address then refuses each connection.
    pub async fn stop(&self) -> Result<(), KitError> {
        self.container.stop().await
    }

    /// Starts the container after [`K3s::stop`] and waits until the API server is ready.
    pub async fn resume(&self) -> Result<(), KitError> {
        self.container.start_again().await
    }
}

/// Runs `start` again while its error is a k3s that ended by itself, `START_ATTEMPTS` times
/// at most. Each other error is the result at once.
async fn with_new_attempts<T, F>(mut start: impl FnMut() -> F) -> Result<T, KitError>
where
    F: Future<Output = Result<T, KitError>>,
{
    let mut attempt = 1;
    loop {
        match start().await {
            Err(error) if error.is_k3s_stopped() && attempt < START_ATTEMPTS => {
                eprintln!("fresnel-testkit: start {attempt} of {START_ATTEMPTS} failed: {error}");
                attempt += 1;
            }
            started => return started,
        }
    }
}

/// A port of 127.0.0.1 that is free now. Docker needs the number before the container
/// starts, and the port must not change at a restart.
fn free_port() -> Result<u16, KitError> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()?.port())
}

/// The folder with the kubeconfig files of one cluster. Its drop removes it.
struct Files {
    folder: PathBuf,
}

impl Files {
    fn create(name: &str) -> Result<Self, KitError> {
        use std::os::unix::fs::DirBuilderExt;

        let folder = std::env::temp_dir().join(name);
        let created = fs::DirBuilder::new().mode(0o700).create(&folder);
        created.map_err(|error| KitError::because(format!("cannot create {}", folder.display()), error))?;
        Ok(Self { folder })
    }

    /// One file for each server address, so that each proxy of a test has its kubeconfig.
    fn kubeconfig(&self, server: SocketAddr) -> PathBuf {
        Path::new(&self.folder).join(format!("kubeconfig-{}.yaml", server.port()))
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.folder) {
            eprintln!("fresnel-testkit: cannot remove {}: {error}", self.folder.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// Runs a start that fails with these errors, one for each attempt, and then works.
    async fn started_after(errors: &[fn() -> KitError]) -> (Result<u32, String>, u32) {
        let attempts = Cell::new(0);
        let start = || {
            let attempt = attempts.replace(attempts.get() + 1);
            let error = errors.get(attempt as usize).map(|error| error());
            async move { error.map_or(Ok(7), Err) }
        };
        let started = with_new_attempts(start).await.map_err(|error| error.to_string());
        (started, attempts.get())
    }

    #[tokio::test]
    async fn a_k3s_that_ended_by_itself_gets_a_new_start_and_each_other_error_does_not() {
        let stopped: fn() -> KitError = || KitError::k3s_stopped("k3s stopped");
        let no_docker: fn() -> KitError = || KitError::new("docker: cannot start");
        assert_eq!(started_after(&[]).await, (Ok(7), 1));
        assert_eq!(started_after(&[stopped, stopped]).await, (Ok(7), 3));
        assert_eq!(started_after(&[stopped, stopped, stopped]).await, (Err("k3s stopped".to_owned()), 3));
        assert_eq!(started_after(&[no_docker]).await, (Err("docker: cannot start".to_owned()), 1));
        assert_eq!(started_after(&[stopped, no_docker]).await, (Err("docker: cannot start".to_owned()), 2));
    }
}
