//! The k3s container of one test cluster.

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::time::{Instant, sleep};

use super::docker::{docker, docker_blocking, docker_with_errors};
use super::network::Network;
use crate::KitError;

/// The image of the tests. The API version of `k8s-openapi` in the workspace fits it.
const IMAGE: &str = "rancher/k3s:v1.33.5-k3s1";
/// The port of the kube-apiserver in the container. k3s binds it to 127.0.0.1 by default.
const API_PORT: u16 = 6444;
const READY_LIMIT: Duration = Duration::from_secs(180);
const READY_STEP: Duration = Duration::from_millis(250);
/// Without a limit, `kubectl` can wait with no end for a server that starts.
const READY_REQUEST_LIMIT: &str = "--request-timeout=5s";
/// The count of the last lines of k3s in the error of a cluster that stopped.
const LOG_LINES: &str = "30";
/// The seconds that Docker waits for the end of k3s before it kills the process.
const STOP_SECONDS: &str = "3";

/// A k3s container. Its drop removes it with its volumes.
pub(super) struct Container {
    pub(super) name: String,
}

impl Container {
    /// Starts k3s and publishes its kube-apiserver on `host_port` of 127.0.0.1.
    ///
    /// The supervisor port 6443 gets no published port, so no test can use it by mistake.
    /// The parts of k3s that no test needs are off, which makes the start faster.
    /// k3s runs its own container runtime, which needs the mode `--privileged`. `extra` is
    /// one more argument of the k3s server.
    pub(super) async fn run(
        name: &str,
        network: &Network,
        host_port: u16,
        extra: Option<&str>,
    ) -> Result<Self, KitError> {
        // Docker can create a container and then fail to start it. The drop removes it.
        let container = Self { name: name.to_owned() };
        let address = network.address.to_string();
        let publish = format!("{}:{host_port}:{API_PORT}", Ipv4Addr::LOCALHOST);
        let run = ["run", "--detach", "--privileged", "--name", name, "--network", name, "--ip", &address];
        let k3s = [
            "server",
            "--disable=traefik",
            "--disable=servicelb",
            "--disable=metrics-server",
            // Docker can publish the port only when the server listens on the container address.
            "--kube-apiserver-arg=bind-address=0.0.0.0",
            // The kubelet refuses each image pull above 5 in one second. A test that starts
            // many pods at once would see some of them fail and wait, a different set each time.
            "--kubelet-arg=registry-qps=0",
        ];
        let extra: Vec<&str> = extra.into_iter().collect();
        docker(&[&run[..], &["--publish", &publish, IMAGE], &k3s[..], &extra].concat()).await?;
        Ok(container)
    }

    /// Runs a program in the container and gives its output.
    pub(super) async fn exec(&self, program: &[&str]) -> Result<String, KitError> {
        docker(&[&["exec", &self.name], program].concat()).await
    }

    /// Waits until the API server reports that it is ready, and then until the namespace
    /// `default` has its service account. The account comes some seconds after the server,
    /// and the server refuses each new pod before it. Docker gives no signal for these
    /// states, so the helper asks again.
    pub(super) async fn wait_ready(&self) -> Result<(), KitError> {
        self.wait_for("the API server is not ready", &["get", "--raw", "/readyz"]).await?;
        let account = ["get", "serviceaccount", "default", "--namespace", "default", "--output", "name"];
        self.wait_for("the namespace `default` has no service account", &account).await
    }

    /// Runs a `kubectl` command again and again until it works. `problem` is the text of
    /// the error at the time limit.
    ///
    /// k3s can stop with an error, also after its API server was ready. A wait for a
    /// container that is gone would only cost its limit and hide the cause, so the error
    /// then comes at once, with the last lines that k3s wrote.
    async fn wait_for(&self, problem: &str, kubectl: &[&str]) -> Result<(), KitError> {
        let limit = Instant::now() + READY_LIMIT;
        loop {
            let error = match self.exec(&[&["kubectl"], kubectl, &[READY_REQUEST_LIMIT]].concat()).await {
                Ok(_) => return Ok(()),
                Err(error) => error,
            };
            if !self.is_running().await {
                let lines = docker_with_errors(&["logs", "--tail", LOG_LINES, &self.name]).await;
                let lines = lines.unwrap_or_else(|error| format!("no log: {error}"));
                return Err(KitError::k3s_stopped(format!("{}: k3s stopped. Its last lines:\n{lines}", self.name)));
            }
            if Instant::now() >= limit {
                return Err(KitError::because(format!("{}: {problem} after {READY_LIMIT:?}", self.name), error));
            }
            sleep(READY_STEP).await;
        }
    }

    /// True while the process of k3s runs in the container.
    async fn is_running(&self) -> bool {
        let state = docker(&["inspect", "--format", "{{.State.Running}}", &self.name]).await;
        state.is_ok_and(|state| state.trim() == "true")
    }

    pub(super) async fn restart(&self) -> Result<(), KitError> {
        docker(&["restart", "--time", STOP_SECONDS, &self.name]).await?;
        self.wait_ready().await
    }

    pub(super) async fn stop(&self) -> Result<(), KitError> {
        docker(&["stop", "--time", STOP_SECONDS, &self.name]).await.map(drop)
    }

    pub(super) async fn start_again(&self) -> Result<(), KitError> {
        docker(&["start", &self.name]).await?;
        self.wait_ready().await
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        if let Err(error) = docker_blocking(&["rm", "--force", "--volumes", &self.name]) {
            eprintln!("fresnel-testkit: {error}");
        }
    }
}
