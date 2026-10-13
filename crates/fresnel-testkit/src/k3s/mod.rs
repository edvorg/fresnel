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

use std::fs;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};

use crate::KitError;
use container::Container;
use network::Network;

/// Each container and each network of the test kit has this start of its name.
/// `docker ps --all --filter name=fresnel-test-` shows what a stopped test run left.
const NAME_PREFIX: &str = "fresnel-test-";
const K3S_KUBECONFIG: &str = "/etc/rancher/k3s/k3s.yaml";

/// A running k3s cluster. The drop removes the container, its network and its files.
///
/// The fields drop in this sequence. The network can go only after its container.
pub struct K3s {
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
    pub async fn start(label: &str) -> Result<Self, KitError> {
        let name = format!("{NAME_PREFIX}{label}-{:06x}", fastrand::u32(..0x0100_0000));
        let api = SocketAddr::from((Ipv4Addr::LOCALHOST, free_port()?));
        let network = Network::create(&name).await?;
        let container = Container::run(&name, &network, api.port()).await?;
        container.wait_ready().await?;
        let k3s_kubeconfig = container.exec(&["cat", K3S_KUBECONFIG]).await?;
        let files = Files::create(&name)?;
        kubeconfig::write(&files.kubeconfig(api), &kubeconfig::with_server(&k3s_kubeconfig, api)?)?;
        Ok(Self { container, _network: network, files, api, k3s_kubeconfig })
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
