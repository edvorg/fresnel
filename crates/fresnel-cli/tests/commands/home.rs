//! The home folder of one test, and the program run in it.

use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::Duration;

use fresnel_testkit::FINDER_PATH;
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

/// The longest run: the wait for the close of an idle connection, and a margin.
const LIMIT: Duration = Duration::from_secs(240);

/// The home folder of the test kit, with the program run in it.
pub(crate) struct Home(fresnel_testkit::Home);

impl Home {
    /// The profile of the folder sets `KUBECONFIG` to these files of its folder `work`.
    pub(crate) fn new(label: &str, kubeconfig_files: &[&str]) -> Self {
        Self(fresnel_testkit::Home::new(label, kubeconfig_files).expect("a home folder"))
    }

    pub(crate) fn path(&self, file: &str) -> PathBuf {
        self.0.path(file)
    }

    pub(crate) fn write(&self, file: &str, text: &str) {
        self.0.write(file, text).expect("a write to the test folder");
    }

    /// Adds a program to the folder `plugins`: a shell script with this text.
    pub(crate) fn add_program(&self, name: &str, script: &str) {
        self.0.add_program(name, script).expect("the program can run");
    }

    pub(crate) fn read(&self, file: &str) -> String {
        self.0.read(file)
    }

    pub(crate) fn json(&self, file: &str) -> Value {
        serde_json::from_str(&self.read(file)).expect("the JSON report")
    }

    /// One command of `fresnel-cli` in this home folder, with the environment of Finder.
    pub(crate) fn command(&self, command_name: &str, arguments: &[&str]) -> Command {
        let folder = self.0.folder();
        let mut command = Command::new("/usr/bin/env");
        command
            .args(["-i", &format!("HOME={}", folder.display()), &format!("PATH={FINDER_PATH}")])
            .arg(env!("CARGO_BIN_EXE_fresnel-cli"))
            .arg(command_name)
            .args(arguments)
            .current_dir(folder)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        command
    }

    /// Runs one command to its end.
    pub(crate) async fn run(&self, command_name: &str, arguments: &[&str]) -> Run {
        let mut command = self.command(command_name, arguments);
        Run::of(timeout(LIMIT, command.output()).await.expect("the program ends in time").expect("the program starts"))
    }
}

/// The result of one run of the program.
pub(crate) struct Run {
    pub(crate) status: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl Run {
    pub(crate) fn of(output: Output) -> Self {
        Self {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}
