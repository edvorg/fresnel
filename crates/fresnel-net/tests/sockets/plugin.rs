//! The fake credential plugin and the fake login shell of the tests: shell scripts, so the
//! tests that run them are for unix. The job of a child on Windows has its own tests in
//! `platform`.

use std::path::{Path, PathBuf};

use fresnel_net::credential::PluginSpec;
use fresnel_net::env::ProcessEnv;

/// A folder for the files of one test. Its drop removes it.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("fresnel-net-test-{label}-{:08x}", fastrand::u32(..)));
        std::fs::create_dir(&folder).expect("a new folder");
        Self(folder)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// The text of a file of the folder. Empty when the file is absent.
    pub(crate) fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.0.join(file)).unwrap_or_default()
    }

    pub(crate) fn write(&self, file: &str, text: &str) {
        std::fs::write(self.0.join(file), text).expect("a write to the test folder");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}

pub(crate) fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// The spec of the fake plugin. It gives the tokens `<token>-1`, `<token>-2` and so on.
pub(crate) fn plugin(state: &Scratch, token: &str) -> PluginSpec {
    PluginSpec {
        command: fixture("fake-plugin").display().to_string(),
        api_version: Some("client.authentication.k8s.io/v1beta1".to_owned()),
        env: vec![
            ("FAKE_PLUGIN_STATE".to_owned(), state.path().display().to_string()),
            ("FAKE_PLUGIN_TOKEN".to_owned(), token.to_owned()),
        ],
        ..PluginSpec::default()
    }
}

/// The environment of a program from Finder: the plugin needs `PATH` for its own tools.
pub(crate) fn finder_env() -> ProcessEnv {
    [("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")].into_iter().collect()
}

/// True while a process with this id exists.
pub(crate) fn process_exists(id: &str) -> bool {
    let id: i32 = id.trim().parse().expect("a process id");
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(id), None).is_ok()
}
