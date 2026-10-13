//! The home folder of one test, and the program run in it.

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

/// The `PATH` of a program that starts from Finder.
const FINDER_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
/// The longest run: the wait for the close of an idle connection, and a margin.
const LIMIT: Duration = Duration::from_secs(240);

/// A home folder with a shell profile. Its drop removes it.
pub(crate) struct Home(PathBuf);

impl Home {
    /// The profile adds the folder `plugins` to `PATH` and sets `KUBECONFIG` to these files
    /// of the folder `work`, as the profile of a user does. A shell reads the profile file
    /// of its own type.
    pub(crate) fn new(label: &str, kubeconfig_files: &[&str]) -> Self {
        let folder = std::env::temp_dir().join(format!("fresnel-cli-test-{label}-{:08x}", fastrand::u32(..)));
        for sub in ["", "plugins", "state", "work"] {
            std::fs::create_dir(folder.join(sub)).expect("a new folder");
        }
        let home = Self(folder);
        let files: Vec<String> = kubeconfig_files.iter().map(|file| format!("$HOME/work/{file}")).collect();
        let profile = format!("export PATH=\"$HOME/plugins:$PATH\"\nexport KUBECONFIG=\"{}\"\n", files.join(":"));
        for file in [".zprofile", ".bash_profile", ".profile"] {
            home.write(file, &profile);
        }
        // With no file for interactive shells, zsh asks a new user some questions.
        home.write(".zshrc", "");
        home
    }

    pub(crate) fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }

    pub(crate) fn write(&self, file: &str, text: &str) {
        std::fs::write(self.path(file), text).expect("a write to the test folder");
    }

    /// Adds a program to the folder `plugins`: a shell script with this text.
    pub(crate) fn add_program(&self, name: &str, script: &str) {
        let file = format!("plugins/{name}");
        self.write(&file, script);
        let executable = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(self.path(&file), executable).expect("the program can run");
    }

    pub(crate) fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.path(file)).unwrap_or_default()
    }

    pub(crate) fn json(&self, file: &str) -> Value {
        serde_json::from_str(&self.read(file)).expect("the JSON report")
    }

    /// Runs `fresnel-cli doctor` in this home folder, with the environment of Finder.
    pub(crate) async fn doctor(&self, arguments: &[&str]) -> Run {
        let mut command = Command::new("/usr/bin/env");
        command
            .args(["-i", &format!("HOME={}", self.0.display()), &format!("PATH={FINDER_PATH}")])
            .arg(env!("CARGO_BIN_EXE_fresnel-cli"))
            .arg("doctor")
            .args(arguments)
            .current_dir(&self.0)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        let output =
            timeout(LIMIT, command.output()).await.expect("the program ends in time").expect("the program starts");
        Run::of(output)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}

/// The result of one run of the program.
pub(crate) struct Run {
    pub(crate) status: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl Run {
    fn of(output: Output) -> Self {
        Self {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}
