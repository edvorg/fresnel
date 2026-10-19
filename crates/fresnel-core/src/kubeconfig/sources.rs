//! Where the kubeconfig files of the user are.

use std::path::{Path, PathBuf};

use fresnel_net::env::ProcessEnv;

const VARIABLE: &str = "KUBECONFIG";

/// The files of the user in the sequence of their priority: each file of `KUBECONFIG`, then
/// `.kube/config` of the home folder of kubectl.
///
/// The home folder is the first folder of [`ProcessEnv::kube_homes`] whose file `has_file`
/// finds, as kubectl looks on Windows in `%HOME%`, `%HOMEDRIVE%%HOMEPATH%` and then
/// `%USERPROFILE%`. With no such file, it is the home folder of the user, whose file can come
/// later. `has_file` looks at the disk, so a caller in a task runs this on a worker thread.
///
/// `env` is the resolved environment of the login shell. A program that starts from Finder
/// has no `KUBECONFIG` in its own environment.
pub fn default_sources(env: &ProcessEnv, has_file: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let listed = env.get(VARIABLE).map(std::env::split_paths).into_iter().flatten();
    let mut sources: Vec<PathBuf> = Vec::new();
    let file = |home: PathBuf| home.join(".kube").join("config");
    let found = env.kube_homes().into_iter().map(file).find(|path| has_file(path));
    let default = found.or_else(|| env.home().map(PathBuf::from).map(file));
    for path in listed.chain(default) {
        // `kubectl` skips an empty entry of the list.
        if !path.as_os_str().is_empty() && !sources.contains(&path) {
            sources.push(path);
        }
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(pairs: &[(&str, &str)]) -> Vec<String> {
        with_files(pairs, &[])
    }

    /// The sources when only the files `files` exist.
    fn with_files(pairs: &[(&str, &str)], files: &[&str]) -> Vec<String> {
        let env: ProcessEnv = pairs.iter().copied().collect();
        let has_file = |path: &Path| files.iter().any(|file| Path::new(file) == path);
        default_sources(&env, has_file).iter().map(|path| path.display().to_string()).collect()
    }

    /// Unix paths and separators.
    #[cfg(unix)]
    #[test]
    fn the_files_of_the_variable_come_first_and_the_default_file_comes_last() {
        let home = ("HOME", "/Users/tester");
        assert_eq!(sources(&[home]), ["/Users/tester/.kube/config"]);
        assert_eq!(
            sources(&[home, ("KUBECONFIG", "/work/a.yaml::/work/b.yaml:")]),
            ["/work/a.yaml", "/work/b.yaml", "/Users/tester/.kube/config"]
        );
        assert_eq!(
            sources(&[home, ("KUBECONFIG", "/Users/tester/.kube/config:/work/a.yaml")]),
            ["/Users/tester/.kube/config", "/work/a.yaml"],
            "a file is in the list one time"
        );
        assert_eq!(sources(&[("KUBECONFIG", "/work/a.yaml")]), ["/work/a.yaml"]);
        assert_eq!(sources(&[]), Vec::<String>::new());
    }

    /// An empty home variable names no folder, so no default file comes: `.kube/config`
    /// would be a path below the current folder of the process.
    #[test]
    fn an_empty_home_variable_gives_no_default_file() {
        let mut env = ProcessEnv::default();
        env.set_home("");
        assert_eq!(default_sources(&env, |_| true), Vec::<PathBuf>::new());
        env.set(VARIABLE, "a.yaml");
        assert_eq!(default_sources(&env, |_| true), [PathBuf::from("a.yaml")]);
    }

    #[cfg(windows)]
    #[test]
    fn the_default_file_is_in_the_profile_folder_of_the_user() {
        let home = ("USERPROFILE", r"C:\Users\tester");
        assert_eq!(sources(&[home]), [r"C:\Users\tester\.kube\config"]);
        let listed = sources(&[home, ("KubeConfig", r"C:\work\a.yaml;;C:\Users\tester\.kube\config")]);
        assert_eq!(listed, [r"C:\work\a.yaml", r"C:\Users\tester\.kube\config"], "one list, with no case in a name");
        assert_eq!(sources(&[("HOME", "/home/tester")]), Vec::<String>::new(), "HOME is not the home folder");
    }

    /// kubectl takes the first of `%HOME%`, `%HOMEDRIVE%%HOMEPATH%` and `%USERPROFILE%` that has
    /// a `.kube\config`, as for a user of Git Bash, whose `HOME` is an other folder.
    #[cfg(windows)]
    #[test]
    fn the_default_file_is_the_first_one_that_kubectl_finds() {
        let vars =
            [("USERPROFILE", r"C:\Users\tester"), ("HOME", r"D:\home"), ("HOMEDRIVE", "E:"), ("HOMEPATH", r"\me")];
        let (home, drive, profile) = (r"D:\home\.kube\config", r"E:\me\.kube\config", r"C:\Users\tester\.kube\config");
        assert_eq!(with_files(&vars, &[profile, drive, home]), [home]);
        assert_eq!(with_files(&vars, &[profile, drive]), [drive]);
        assert_eq!(with_files(&vars, &[profile]), [profile]);
        assert_eq!(with_files(&vars, &[]), [profile], "with no file, the one of the profile, which can come later");
    }
}
