//! The borders between the crates: what a crate must not depend on.
//!
//! `fresnel-net` replaces parts of the transport of kube, so a kube upgrade must not be
//! able to change it. `fresnel-core` is the domain, and it must work with no GUI toolkit.
//! A rule that only a document states does not hold, thus a test and a command check it.

use std::io;
use std::path::Path;
use std::process::Stdio;

/// One rule: `package` has no crate of `forbidden` among its dependencies, also not through
/// some other crate. A dependency of its tests does not count.
struct Rule {
    package: &'static str,
    forbidden: &'static [&'static str],
    reason: &'static str,
}

const RULES: [Rule; 2] = [
    Rule {
        package: "fresnel-net",
        forbidden: &["kube"],
        reason: "a kube upgrade must not change the connection layer",
    },
    Rule {
        package: "fresnel-core",
        forbidden: &["egui", "eframe", "epaint"],
        reason: "the domain must work with no GUI toolkit",
    },
];

/// The result of the check: the count of rules, and one line for each problem.
#[derive(Debug)]
pub struct Borders {
    /// The count of rules that the check read.
    pub rules: usize,
    /// One line for each problem.
    pub problems: Vec<String>,
}

/// Checks each rule for the workspace at `root`. A package whose dependencies the check
/// cannot list is a problem.
pub fn check(root: &Path) -> Borders {
    let mut problems = Vec::new();
    for rule in &RULES {
        match tree(root, rule.package) {
            Ok(tree) => problems.extend(rule.problems(&tree)),
            Err(error) => problems.push(format!("{}: cannot list its dependencies: {error}", rule.package)),
        }
    }
    Borders { rules: RULES.len(), problems }
}

impl Rule {
    /// The problems in the text of `cargo tree` for the package: one line for each crate.
    fn problems(&self, tree: &str) -> Vec<String> {
        let mut names = tree.lines().filter_map(|line| line.split_whitespace().next());
        // The first line is the package itself. Some other text is not a list of crates.
        if names.next() != Some(self.package) {
            return vec![format!("{}: the text of `cargo tree` does not start with the package", self.package)];
        }
        let mut found: Vec<&str> = names.filter(|name| self.forbids(name)).collect();
        found.sort_unstable();
        found.dedup();
        found.into_iter().map(|name| format!("{} depends on {name}: {}", self.package, self.reason)).collect()
    }

    /// True for a forbidden crate and for each crate of its family, as `kube-core` for
    /// `kube` and `egui_extras` for `egui`.
    fn forbids(&self, name: &str) -> bool {
        let in_family = |rest: &str| rest.is_empty() || rest.starts_with(['-', '_']);
        self.forbidden.iter().any(|forbidden| name.strip_prefix(forbidden).is_some_and(in_family))
    }
}

/// The dependencies of a package for each platform, one crate on each line. The cargo that
/// runs this task lists them, with no change of the lock file.
fn tree(root: &Path, package: &str) -> io::Result<String> {
    let mut listing = crate::cargo(root);
    listing.args(["tree", "--locked", "--edges", "normal,build", "--target", "all", "--prefix", "none"]);
    let output = listing.args(["--package", package]).stdin(Stdio::null()).output()?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!("cargo tree failed ({}): {}", output.status, text.trim())));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
    }

    fn rule(package: &'static str, forbidden: &'static [&'static str]) -> Rule {
        Rule { package, forbidden, reason: "the rule of the test" }
    }

    #[test]
    fn each_rule_holds_for_this_workspace() {
        let borders = check(root());
        assert_eq!((borders.rules, borders.problems), (2, Vec::<String>::new()));
    }

    /// The check on the real output of cargo: it must find a crate that is there.
    #[test]
    fn a_rule_finds_a_dependency_through_some_other_crate_and_not_one_of_a_test() {
        // `fresnel-ui` has no kube in its manifest. It gets kube through `fresnel-core`.
        let through_core = rule("fresnel-ui", &["kube"]);
        let problems = through_core.problems(&tree(root(), "fresnel-ui").expect("the tree"));
        assert_eq!(problems.first().map(String::as_str), Some("fresnel-ui depends on kube: the rule of the test"));
        assert!(problems.contains(&"fresnel-ui depends on kube-core: the rule of the test".to_owned()), "{problems:?}");

        // The test kit is a dependency of the tests of `fresnel-net` only.
        let of_tests = rule("fresnel-net", &["fresnel-testkit", "rcgen"]);
        assert_eq!(of_tests.problems(&tree(root(), "fresnel-net").expect("the tree")), Vec::<String>::new());
    }

    #[test]
    fn a_family_of_crates_is_forbidden_and_a_crate_with_a_similar_name_is_not() {
        let tree = "fresnel-core v0.1.0 (/work/crates/fresnel-core)\negui v0.36.2\negui_extras v0.36.2 (*)\n\
                    eguilder v1.0.0\nepaint v0.36.2\nepaint-default-fonts v0.36.2\nserde v1.0.229\negui v0.36.2 (*)\n";
        let problems = RULES[1].problems(tree);
        let crates: Vec<&str> = problems.iter().filter_map(|problem| problem.split(' ').nth(3)).collect();
        assert_eq!(crates, ["egui:", "egui_extras:", "epaint:", "epaint-default-fonts:"]);
        assert_eq!(problems[0], "fresnel-core depends on egui: the domain must work with no GUI toolkit");
    }

    #[test]
    fn a_text_that_is_no_tree_and_a_package_that_does_not_exist_are_problems() {
        let other_text = RULES[0].problems("error: the lock file needs an update\n");
        assert_eq!(other_text, ["fresnel-net: the text of `cargo tree` does not start with the package"]);
        let absent = tree(root(), "no-such-package").expect_err("cargo knows no such package").to_string();
        assert!(absent.starts_with("cargo tree failed ("), "{absent}");
    }
}
