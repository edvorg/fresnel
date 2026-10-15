//! Where the kubeconfig files of the user are.

use std::path::PathBuf;

use fresnel_net::env::ProcessEnv;

const VARIABLE: &str = "KUBECONFIG";

/// The files of the user in the sequence of their priority: each file of `KUBECONFIG`, then
/// `.kube/config` of the home folder (`%USERPROFILE%\.kube\config` on Windows).
///
/// `env` is the resolved environment of the login shell. A program that starts from Finder
/// has no `KUBECONFIG` in its own environment.
pub fn default_sources(env: &ProcessEnv) -> Vec<PathBuf> {
    let listed = env.get(VARIABLE).map(std::env::split_paths).into_iter().flatten();
    let mut sources: Vec<PathBuf> = Vec::new();
    let default = env.home().map(|home| PathBuf::from(home).join(".kube").join("config"));
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
        let env: ProcessEnv = pairs.iter().copied().collect();
        default_sources(&env).iter().map(|path| path.display().to_string()).collect()
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
        assert_eq!(default_sources(&env), Vec::<PathBuf>::new());
        env.set(VARIABLE, "a.yaml");
        assert_eq!(default_sources(&env), [PathBuf::from("a.yaml")]);
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
}
