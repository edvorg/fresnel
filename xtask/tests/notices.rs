//! The license files of a release, for the program of this workspace.
//!
//! The test gets the crates of the program from `cargo tree` for each target alone, and the
//! crates of the tests and the builds from the resolve of `cargo metadata`, so it does not repeat
//! the code of the notices.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;
use xtask::notices::{FILES, Notices};

const TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The output of a command of cargo in the workspace.
fn cargo(arguments: &[&str]) -> String {
    let output = xtask::cargo(&root()).args(arguments).output().expect("cargo starts");
    assert!(output.status.success(), "cargo {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).expect("the output of cargo is UTF-8")
}

/// The crates of the part Crates of the file, as "name version", with the license of each one.
fn named(text: &str) -> BTreeMap<String, String> {
    let start = text.find("\nPart 2: Crates\n").expect("the part Crates");
    let end = text.find("\nPart 3: Texts\n").expect("the part Texts");
    let mut named = BTreeMap::new();
    for entry in text[start..end].split("\n\n").map(str::trim).filter(|entry| !entry.is_empty()) {
        if entry.starts_with("Part 2: Crates\n") {
            continue;
        }
        let mut lines = entry.lines();
        let label = lines.next().expect("the name of a crate");
        let license = lines.next().and_then(|line| line.strip_prefix("  License: ")).expect("the license of a crate");
        assert!(named.insert(label.to_owned(), license.to_owned()).is_none(), "{label} is named two times");
    }
    named
}

/// The crates of others in the program for one target, with their licenses, as cargo tree shows
/// them. `{l}` comes first: cargo writes the mark of a crate that it showed before after `{p}`.
fn shipped(target: &str, ours: &BTreeSet<String>) -> BTreeMap<String, String> {
    let arguments = ["tree", "--locked", "-p", "fresnel", "-e", "normal", "--prefix", "none", "--format", "{l}|{p}"];
    let tree = cargo(&[arguments.as_slice(), &["--target", target]].concat());
    let mut shipped = BTreeMap::new();
    for line in tree.lines().filter(|line| !line.is_empty()) {
        let (license, package) = line.split_once('|').expect("the license and the package");
        let mut words = package.split_whitespace();
        let name = words.next().expect("a name");
        let version = words.next().and_then(|word| word.strip_prefix('v')).expect("a version");
        if !ours.contains(name) {
            shipped.insert(format!("{name} {version}"), license.to_owned());
        }
    }
    shipped
}

/// The crates that the resolve of `cargo metadata` reaches from `starts` along the edges that
/// `follow` takes, as "name version".
fn reached(metadata: &Value, starts: &[&str], follow: impl Fn(&Value) -> bool) -> BTreeSet<String> {
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .expect("the nodes")
        .iter()
        .map(|node| (node["id"].as_str().expect("an id"), node))
        .collect();
    let mut seen = BTreeSet::new();
    let mut next: Vec<&str> = starts.to_vec();
    while let Some(id) = next.pop() {
        if seen.insert(id) {
            let deps = nodes[id]["deps"].as_array().expect("the edges");
            let taken = deps.iter().filter(|dep| dep["dep_kinds"].as_array().expect("kinds").iter().any(&follow));
            next.extend(taken.map(|dep| dep["pkg"].as_str().expect("a package")));
        }
    }
    let packages = metadata["packages"].as_array().expect("the packages");
    let label = |package: &Value| {
        format!("{} {}", package["name"].as_str().expect("a name"), package["version"].as_str().expect("a version"))
    };
    packages.iter().filter(|package| seen.contains(package["id"].as_str().expect("an id"))).map(label).collect()
}

#[test]
fn the_file_names_each_crate_of_the_program_and_no_crate_of_a_test_or_a_build() {
    let metadata: Value = serde_json::from_str(&cargo(&[
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--filter-platform",
        TARGETS[0],
        "--filter-platform",
        TARGETS[1],
    ]))
    .expect("the JSON of cargo metadata");
    let members: Vec<&str> = metadata["workspace_members"]
        .as_array()
        .expect("the members")
        .iter()
        .map(|id| id.as_str().expect("an id"))
        .collect();
    let packages = metadata["packages"].as_array().expect("the packages");
    let ours: BTreeSet<String> = packages
        .iter()
        .filter(|package| members.contains(&package["id"].as_str().expect("an id")))
        .map(|package| package["name"].as_str().expect("a name").to_owned())
        .collect();
    let program = packages.iter().find(|package| package["name"] == "fresnel").expect("the program")["id"]
        .as_str()
        .expect("an id");

    let notices = Notices::of_workspace(&root()).expect("the notices");
    let named = named(notices.third_party());
    let (macos, windows) = (shipped(TARGETS[0], &ours), shipped(TARGETS[1], &ours));
    assert!(macos.keys().any(|label| !windows.contains_key(label) && label.starts_with("objc2-app-kit ")));
    assert!(windows.keys().any(|label| !macos.contains_key(label) && label.starts_with("windows-sys ")));
    let mut each = macos;
    each.extend(windows);
    let differences: Vec<String> = named
        .iter()
        .filter(|(label, license)| each.get(*label) != Some(*license))
        .map(|(label, license)| format!("the file names {label}, {license}"))
        .chain(each.keys().filter(|label| !named.contains_key(*label)).map(|label| format!("the file misses {label}")))
        .collect();
    assert_eq!(differences, Vec::<String>::new(), "each crate of the program, with its license");
    assert_eq!(notices.crates(), named.len());

    // A crate of the build that is no normal dependency of the program: of a test, of a build
    // script, of xtask, or of the other programs.
    let normal = reached(&metadata, &[program], |kind| kind["kind"].is_null());
    let others = reached(&metadata, &members, |_| true);
    let not_shipped: BTreeSet<&String> = others.difference(&normal).collect();
    for absent in ["egui_kittest ", "rcgen ", "oxipng ", "fresnel-testkit "] {
        assert!(not_shipped.iter().any(|label| label.starts_with(absent)), "{absent} is not shipped: {not_shipped:?}");
    }
    let wrong: Vec<&String> =
        named.keys().filter(|label| !normal.contains(*label) || not_shipped.contains(label)).collect();
    assert_eq!(wrong, Vec::<&String>::new());
}

#[test]
fn the_file_has_the_font_license_the_licenses_of_fresnel_and_the_texts_of_the_packages() {
    let notices = Notices::of_workspace(&root()).expect("the notices");
    let text = notices.third_party();
    let read = |file: &str| std::fs::read_to_string(root().join(file)).expect("a license file of the repository");
    let font = read("crates/fresnel-ui/fonts/OFL.txt");
    assert!(text.contains("\n\n\nPart 1: JetBrains Mono\n======================\n\n"), "the part of the font");
    assert!(
        text.contains(&format!("License 1.1:\n\n{}\n\n\nPart 2: Crates\n", font.trim_end())),
        "the OFL of JetBrains Mono"
    );
    assert!(
        text.contains(&format!("\nLICENSE-MIT\n-----------\n\n{}\n\nLICENSE-APACHE\n", read("LICENSE-MIT").trim_end()))
    );
    assert!(text.ends_with(&format!("\nLICENSE-APACHE\n--------------\n\n{}\n", read("LICENSE-APACHE").trim_end())));

    // A licence of another spelling, the notices of the fonts of egui, and a crate with none. An
    // entry has each of its lines as one line, so a wrap does not break a name from its number.
    let entry = |name: &str| {
        let start = text.find(&format!("\n{name} ")).expect("the entry of the crate") + 1;
        text[start..].split("\n\n").next().expect("an entry").replace("\n    ", " ")
    };
    let imbl = entry("imbl");
    assert!(imbl.contains("\n  License: MPL-2.0+\n  Texts: LICENCE.md (text "), "{imbl}");
    let fonts = entry("epaint_default_fonts");
    for notice in ["Hack-Regular.txt", "OFL.txt", "UFL.txt", "emoji-icon-font-mit-license.txt"] {
        assert!(fonts.contains(&format!("fonts/{notice} (text ")), "{notice}: {fonts}");
    }
    let egui = entry("egui");
    assert!(egui.contains("No license file at the root of its package; the standard texts: MIT (text "), "{egui}");
    assert!(egui.contains("\n  Authors in its manifest: Emil Ernerfeldt"), "{egui}");
    assert!(
        text.contains("Ubuntu Font Licence") || text.contains("UBUNTU FONT LICENCE"),
        "the license of Ubuntu Light"
    );
    // The licenses of code that a crate copied from others, below the root of its package, which
    // the license of ring names; a source file with such a name in small letters is none.
    let ring = entry("ring");
    let once_cell = "src/polyfill/once_cell/LICENSE";
    for file in [format!("{once_cell}-APACHE"), format!("{once_cell}-MIT"), "third_party/fiat/LICENSE".to_owned()] {
        assert!(ring.contains(&format!("{file} (text ")), "{file}: {ring}");
    }
    assert!(text.contains("\nCopyright 2015-2020 the fiat-crypto authors (see the AUTHORS file)\n"), "of fiat");
    let regex = entry("regex-syntax");
    assert!(regex.contains(", src/unicode_tables/LICENSE-UNICODE (text "), "{regex}");
    let foundation = entry("objc2-foundation");
    assert!(foundation.contains("\n  No license file at the root") && !foundation.contains("copying"), "{foundation}");

    // The same workspace gives the same bytes, with no path of this computer.
    assert_eq!(Notices::of_workspace(&root()).expect("the notices again").third_party(), text);
    let workspace = root().canonicalize().expect("the workspace");
    assert!(!text.contains(&*workspace.to_string_lossy()) && !text.contains("index.crates.io-"), "a path");
}

#[test]
fn the_files_go_into_a_folder_with_the_licenses_of_the_repository_unchanged() {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("notices-{}", std::process::id()));
    if folder.exists() {
        std::fs::remove_dir_all(&folder).expect("the folder of an earlier run goes away");
    }
    std::fs::create_dir_all(&folder).expect("a new folder");
    let notices = Notices::of_workspace(&root()).expect("the notices");
    notices.write_into(&folder).expect("the files");

    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .expect("the folder")
        .map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["LICENSE-APACHE", "LICENSE-MIT", "THIRD-PARTY-LICENSES"]);
    assert_eq!(FILES, ["LICENSE-MIT", "LICENSE-APACHE", "THIRD-PARTY-LICENSES"]);
    for name in ["LICENSE-MIT", "LICENSE-APACHE"] {
        let read = |path: PathBuf| std::fs::read(path).expect("a license file");
        assert_eq!(read(folder.join(name)), read(root().join(name)), "{name}");
    }
    let written = std::fs::read_to_string(folder.join("THIRD-PARTY-LICENSES")).expect("the notices");
    assert_eq!(written, notices.third_party());
    std::fs::remove_dir_all(&folder).expect("the folder goes away");
}
