//! The version of the API server of one connected cluster, for the cluster overview (plan
//! section 15, M7): `gitVersion` of `GET /version`, the server version of `kubectl version`.
//!
//! One task reads it after the connect and again each `REREAD`, so the upgrade of a control
//! plane shows. A failed read shows its reason and keeps the version of an earlier read
//! (design rule 5); the next read comes after `RETRY`. Each read has the time limit of the
//! gateway.

use std::convert::Infallible;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fresnel_net::WallClock;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks, panic_message};
use futures_util::FutureExt as _;
use serde::Deserialize;
use tokio::time::sleep;

use super::Gateway;
use crate::error::CoreError;
use crate::published::{Bin, Notice, Publisher, Reader};

/// The time between two reads. An upgrade of a control plane takes minutes, and a read is
/// one small request.
const REREAD: Duration = Duration::from_secs(600);
/// The time after a failed read: the period of the sampler of the metrics.
const RETRY: Duration = Duration::from_secs(15);

/// The version of the API server of one cluster, as a reader holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerVersion {
    /// The version of the last answer, for example `v1.33.5+k3s1`. A failed read keeps it.
    pub version: Option<Arc<str>>,
    /// What the last read gave.
    pub state: VersionState,
}

/// What the last read of the version gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionState {
    /// The first read is on its way.
    Reading,
    /// The last read got the version, at this time of the system.
    Read(SystemTime),
    /// The last read failed at `at`. The next one comes `RETRY` later.
    Failed {
        /// Why.
        error: CoreError,
        /// The time of the system of the failure.
        at: SystemTime,
    },
    /// The task panicked. It starts again.
    InternalError(Arc<str>),
}

/// The reads of the version. The gateway is the one source of the app; a unit test gives
/// its answers with a script.
pub(crate) trait Versions: Send + Sync + 'static {
    /// Reads `gitVersion` of the API server.
    fn version(&self) -> impl Future<Output = Result<Arc<str>, CoreError>> + Send;
}

/// The one field of the answer that the app reads. The other fields of `version.Info` can
/// be absent behind a proxy of a cluster, and the version is there still.
#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "gitVersion")]
    git_version: String,
}

impl Versions for Gateway {
    async fn version(&self) -> Result<Arc<str>, CoreError> {
        let request = http::Request::get("/version").body(Vec::new());
        let request = request.map_err(|error| CoreError::invalid(format!("the read of the version: {error}")))?;
        Ok(self.get::<Answer>(request).await?.git_version.into())
    }
}

/// The version of one cluster with its task. The task stops when this value drops.
pub struct ClusterVersion {
    publisher: Publisher<ServerVersion>,
    _task: TaskHandle,
}

impl ClusterVersion {
    /// Starts the task of the cluster of `gateway`. `notice` hears each new value.
    pub fn start(tasks: &Tasks, gateway: Gateway, bin: Bin, notice: Notice) -> Self {
        let name = TaskName::scoped("server version", gateway.cluster().as_str());
        Self::with_source(tasks, name, gateway, (bin, notice), WallClock::system())
    }

    pub(crate) fn with_source<S: Versions>(
        tasks: &Tasks,
        name: TaskName,
        source: S,
        (bin, notice): (Bin, Notice),
        clock: WallClock,
    ) -> Self {
        let first = ServerVersion { version: None, state: VersionState::Reading };
        let publisher = Publisher::new(first, bin, notice);
        let run = Arc::new((source, publisher.clone(), clock));
        let task = tasks.start(name, Restart::Always(Backoff::default()), move || self::run(Arc::clone(&run)));
        Self { publisher, _task: task }
    }

    /// A reader of the version.
    pub fn reader(&self) -> Reader<ServerVersion> {
        self.publisher.reader()
    }
}

/// One run of the task. A run ends only with a panic: the version then shows the state
/// "internal error", and the start function gives the report and starts a new run.
async fn run<S: Versions>(run: Arc<(S, Publisher<ServerVersion>, WallClock)>) -> Result<(), Infallible> {
    let (source, publisher, clock) = &*run;
    let panic = match AssertUnwindSafe(follow(source, publisher, clock)).catch_unwind().await {
        Ok(never) => match never {},
        Err(panic) => panic,
    };
    let state = VersionState::InternalError(format!("the task panicked: {}", panic_message(panic.as_ref())).into());
    publisher.publish(ServerVersion { state, ..ServerVersion::clone(&publisher.read()) });
    resume_unwind(panic)
}

/// Reads the version with no end.
async fn follow<S: Versions>(source: &S, publisher: &Publisher<ServerVersion>, clock: &WallClock) -> Infallible {
    loop {
        let read = source.version().await;
        let version = publisher.read().version.clone();
        let (value, pause) = match read {
            Ok(version) => (ServerVersion { version: Some(version), state: VersionState::Read(clock.now()) }, REREAD),
            Err(error) => (ServerVersion { version, state: VersionState::Failed { error, at: clock.now() } }, RETRY),
        };
        publisher.publish(value);
        sleep(pause).await;
    }
}

#[cfg(test)]
mod tests;
