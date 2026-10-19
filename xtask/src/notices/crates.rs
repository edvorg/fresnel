//! The crates of others in the program `fresnel` for the targets of a release, from cargo.
//!
//! The resolve of `cargo metadata` unifies the features of the whole workspace, the tests
//! included: a walk of its normal edges from the program named 22 crates that no build of the
//! program has, such as `egui_glow` and `glutin`. `cargo tree` resolves the features of the
//! program alone, as its build does, so it gives the list, and `cargo metadata` gives the folder,
//! the license and the authors of each crate. One `cargo tree` with both targets unifies their
//! features too and named 2 of them, `dlib` and `wayland-sys`, so each target gets its own.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

/// The targets of a release: the app for macOS on Apple silicon and the program for Windows x64.
pub(super) const TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"];
/// The package of the program of a release.
const PROGRAM: &str = "fresnel";

/// The part of the output of `cargo metadata` that the notices read.
#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
}

/// A package of `cargo metadata`.
#[derive(Deserialize)]
pub(super) struct Package {
    id: String,
    pub(super) name: String,
    pub(super) version: String,
    /// The SPDX expression of the manifest. A manifest with only `license-file` has none.
    pub(super) license: Option<String>,
    pub(super) authors: Vec<String>,
    pub(super) manifest_path: PathBuf,
}

/// The crates of others that the program contains for one of the targets: its normal
/// dependencies, with no build or dev dependency. The crates of the workspace are ours.
pub(super) fn shipped(root: &Path) -> io::Result<Vec<Package>> {
    // First, so that cargo has the source of each crate, also of the other target.
    let mut metadata = crate::cargo(root);
    metadata.args(["metadata", "--format-version", "1", "--locked"]);
    for target in TARGETS {
        metadata.args(["--filter-platform", target]);
    }
    let metadata: Metadata = serde_json::from_slice(&output(metadata, "cargo metadata")?)
        .map_err(|error| io::Error::other(format!("the output of cargo metadata: {error}")))?;

    let mut listed = BTreeSet::new();
    for target in TARGETS {
        let mut tree = crate::cargo(root);
        tree.args(["tree", "--locked", "--package", PROGRAM, "--edges", "normal", "--target", target]);
        tree.args(["--prefix", "none", "--format", "{p}"]);
        listed.append(&mut names(&String::from_utf8_lossy(&output(tree, "cargo tree")?))?);
    }
    select(metadata, &listed)
}

/// The name and the version of each crate of the text of `cargo tree`: the two first words
/// of each line, as `serde v1.0.229 (*)`.
fn names(tree: &str) -> io::Result<BTreeSet<(String, String)>> {
    let mut names = BTreeSet::new();
    for line in tree.lines().filter(|line| !line.trim().is_empty()) {
        let mut words = line.split_whitespace();
        let (Some(name), Some(version)) = (words.next(), words.next().and_then(|word| word.strip_prefix('v'))) else {
            return Err(io::Error::other(format!("cargo tree gave the line \"{line}\"")));
        };
        names.insert((name.to_owned(), version.to_owned()));
    }
    Ok(names)
}

/// The packages of `metadata` that `names` lists, except the members of the workspace. Each name
/// must have exactly one package: a crate that the notices miss would have no license text.
fn select(metadata: Metadata, names: &BTreeSet<(String, String)>) -> io::Result<Vec<Package>> {
    let members: BTreeSet<&str> = metadata.workspace_members.iter().map(String::as_str).collect();
    let (ours, others): (Vec<Package>, Vec<Package>) =
        metadata.packages.into_iter().partition(|package| members.contains(package.id.as_str()));
    let ours: BTreeSet<(String, String)> = ours.into_iter().map(|package| (package.name, package.version)).collect();
    let wanted: BTreeSet<&(String, String)> = names.difference(&ours).collect();
    let shipped: Vec<Package> = others
        .into_iter()
        .filter(|package| wanted.contains(&(package.name.clone(), package.version.clone())))
        .collect();
    let found: BTreeSet<(&str, &str)> =
        shipped.iter().map(|package| (package.name.as_str(), package.version.as_str())).collect();
    if found.len() != shipped.len() {
        return Err(io::Error::other("cargo metadata has two packages with the same name and version"));
    }
    let missing: Vec<String> = wanted
        .iter()
        .filter(|(name, version)| !found.contains(&(name.as_str(), version.as_str())))
        .map(|(name, version)| format!("{name} {version}"))
        .collect();
    if !missing.is_empty() {
        return Err(io::Error::other(format!("cargo metadata has no package {}", missing.join(", "))));
    }
    Ok(shipped)
}

/// The output of a command of cargo. A status that is not zero is an error with its text.
fn output(mut command: Command, name: &str) -> io::Result<Vec<u8>> {
    let output = command.stdin(Stdio::null()).output()?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!("{name} failed ({}): {}", output.status, text.trim())));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(id: &str, name: &str, version: &str) -> Package {
        Package {
            id: id.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            license: Some("MIT".to_owned()),
            authors: Vec::new(),
            manifest_path: PathBuf::from(format!("/registry/{name}-{version}/Cargo.toml")),
        }
    }

    fn metadata(packages: Vec<Package>) -> Metadata {
        Metadata { packages, workspace_members: vec!["path+file:///work/fresnel#0.1.0".to_owned()] }
    }

    #[test]
    fn the_names_are_the_crates_of_the_tree_one_time() {
        let tree = "fresnel v0.1.0 (/work/fresnel)\nserde v1.0.229\nobjc2 v0.6.5\nserde v1.0.229 (*)\n\
                    serde_derive v1.0.229 (proc-macro)\n";
        let found: Vec<String> =
            names(tree).expect("a tree").into_iter().map(|(name, version)| format!("{name} {version}")).collect();
        assert_eq!(found, ["fresnel 0.1.0", "objc2 0.6.5", "serde 1.0.229", "serde_derive 1.0.229"]);
        let error = names("error: the lock file needs an update\n").expect_err("no tree").to_string();
        assert_eq!(error, "cargo tree gave the line \"error: the lock file needs an update\"");
    }

    #[test]
    fn the_crates_are_those_of_the_tree_with_no_member_of_the_workspace() {
        let all = || {
            metadata(vec![
                package("path+file:///work/fresnel#0.1.0", "fresnel", "0.1.0"),
                package("registry+serde@1.0.229", "serde", "1.0.229"),
                package("registry+serde@1.0.200", "serde", "1.0.200"),
                package("registry+rcgen@0.14.10", "rcgen", "0.14.10"),
            ])
        };
        let tree: BTreeSet<(String, String)> =
            [("fresnel", "0.1.0"), ("serde", "1.0.229")].map(|(name, version)| (name.into(), version.into())).into();
        let shipped = select(all(), &tree).expect("the crates");
        let ids: Vec<&str> = shipped.iter().map(|package| package.id.as_str()).collect();
        assert_eq!(ids, ["registry+serde@1.0.229"]);

        let mut absent = tree.clone();
        absent.insert(("egui".into(), "0.36.2".into()));
        let error = select(all(), &absent).map(|_| ()).expect_err("no package of egui").to_string();
        assert_eq!(error, "cargo metadata has no package egui 0.36.2");
        let mut twice = all();
        twice.packages.push(package("registry+other@serde", "serde", "1.0.229"));
        select(twice, &tree).map(|_| ()).expect_err("two packages of serde 1.0.229");
    }
}
