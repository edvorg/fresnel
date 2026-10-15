//! What the program gives to the core: the runtime, the files, the detectors and the
//! settings. The settings file replaces some settings at the start (`config::apply`).

use std::path::PathBuf;
use std::time::Duration;

use fresnel_net::NetSettings;
use fresnel_net::detect::Detector;
use fresnel_net::env::{EnvSettings, ProcessEnv};
use tokio::runtime::Handle;

use crate::journal::Journal;
use crate::logs::LogSettings;
use crate::metrics::MetricsSettings;
use crate::store::StoreSettings;

/// The settings of the core.
#[derive(Clone, Debug)]
pub struct CoreSettings {
    /// The read of the login shell environment.
    pub env: EnvSettings,
    /// The connection layer of each cluster.
    pub net: NetSettings,
    /// The stores of each cluster.
    pub stores: StoreSettings,
    /// The metrics sampler of each cluster.
    pub metrics: MetricsSettings,
    /// The log views.
    pub logs: LogSettings,
    /// The wake detector and the network detector look each time this period ends.
    pub detect_period: Duration,
    /// The most repaints in one second that the core asks the UI for.
    pub repaints_per_second: u32,
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            env: EnvSettings::default(),
            net: NetSettings::default(),
            stores: StoreSettings::default(),
            metrics: MetricsSettings::default(),
            logs: LogSettings::default(),
            detect_period: Duration::from_secs(2),
            repaints_per_second: 60,
        }
    }
}

/// What the program gives to the core.
pub struct CoreParts {
    /// The runtime of each task.
    pub runtime: Handle,
    /// The environment of the process. The login shell starts from it.
    pub base_env: ProcessEnv,
    /// The journal. The program gives it the log file as its sink.
    pub journal: Journal,
    /// The state file: the core reads the selection and the table layouts of the last run
    /// from it, and writes each change of them to it.
    pub state_file: PathBuf,
    /// The settings file of the user. The core reads it at its start and after each change,
    /// and writes it only for a save of the settings view.
    pub config_file: PathBuf,
    /// The folder of the files that the user saves, such as a log. The first save makes it.
    pub downloads: PathBuf,
    /// The detectors of a wake and of a network change.
    pub detectors: Vec<Box<dyn Detector>>,
    /// The settings.
    pub settings: CoreSettings,
}
