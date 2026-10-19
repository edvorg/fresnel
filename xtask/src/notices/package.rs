//! The license files of the package of a crate, and the forms that make two texts equal.

use std::fs;
use std::io;
use std::path::Path;

use super::crates::Package;

/// The start of the name of a license file at the root of a package, in capitals: LICENSE,
/// COPYING and NOTICE as most crates have them, LICENCE as `imbl` has it, and UNLICENSE, the
/// text of the Unlicense of `memchr` and the crates of its author.
const PREFIXES: [&str; 5] = ["LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"];

/// The notices of a package outside its root, each with the license whose text it is. The fonts
/// of egui, which the program embeds, have their licenses next to the font files: Hack (MIT and
/// the Bitstream Vera License), Noto Emoji, Ubuntu Light and emoji-icon-font. A file of the list
/// that a package does not have stops the run, so a new version of the crate gets a look.
const OTHER_NOTICES: [(&str, &str, Option<&str>); 4] = [
    ("epaint_default_fonts", "fonts/Hack-Regular.txt", None),
    ("epaint_default_fonts", "fonts/OFL.txt", Some("OFL-1.1")),
    ("epaint_default_fonts", "fonts/UFL.txt", Some("Ubuntu-font-1.0")),
    ("epaint_default_fonts", "fonts/emoji-icon-font-mit-license.txt", None),
];

/// A crate of others in the program, with the license files of its package.
#[derive(Debug)]
pub(super) struct Crate {
    pub(super) name: String,
    pub(super) version: String,
    /// The SPDX expression of its manifest.
    pub(super) license: String,
    pub(super) authors: Vec<String>,
    /// Its license files, sorted by name.
    pub(super) files: Vec<File>,
    /// True when the root of its package has no license file: it then gets the standard text of
    /// each license that no file of `files` is the text of.
    pub(super) bare: bool,
}

/// A license file of a package.
#[derive(Debug)]
pub(super) struct File {
    /// Its path in the package, with `/` between the folders.
    pub(super) name: String,
    /// Its text in the form of [`normal`].
    pub(super) text: String,
    /// The license whose text it is, when that is certain: the one license file of a crate with
    /// one license, or a notice of `OTHER_NOTICES`. Its text is then a standard text.
    pub(super) license: Option<String>,
}

impl Crate {
    /// The name and the version, as the file shows the crate.
    pub(super) fn label(&self) -> String {
        format!("{} {}", self.name, self.version)
    }
}

/// The crate of a package with the license files at the root of its package and its entries of
/// `OTHER_NOTICES`. A manifest with no SPDX expression, only a `license-file`, stops the run: no
/// crate of the program has one, and the file could not tell the license of the crate.
pub(super) fn read(package: Package) -> io::Result<Crate> {
    let label = format!("{} {}", package.name, package.version);
    let Some(license) = package.license else {
        return Err(io::Error::other(format!("{label}: its manifest has no license expression")));
    };
    let folder = package
        .manifest_path
        .parent()
        .ok_or_else(|| io::Error::other(format!("{label}: its manifest has no folder")))?;
    let entries = fs::read_dir(folder)
        .map_err(|error| io::Error::new(error.kind(), format!("the package of {label}: {error}")))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_license_file(&name) && entry.file_type()?.is_file() {
            names.push(name);
        }
    }
    names.sort();

    let licenses = identifiers(&license);
    let alone = match (licenses.as_slice(), names.len()) {
        ([license], 1) => Some(license.clone()),
        _ => None,
    };
    let mut files = Vec::new();
    for name in &names {
        files.push(File { text: text(folder, name, &label)?, name: name.clone(), license: alone.clone() });
    }
    let bare = files.is_empty();
    for (_, name, license) in OTHER_NOTICES.iter().filter(|(crate_name, _, _)| *crate_name == package.name) {
        let text = text(folder, name, &label)?;
        files.push(File { name: (*name).to_owned(), text, license: license.map(str::to_owned) });
    }
    files.sort_by(|one, other| one.name.cmp(&other.name));
    Ok(Crate { name: package.name, version: package.version, license, authors: package.authors, files, bare })
}

/// True for the name of a license file, in any case: `LICENSE-MIT`, `license-apache-2.0`, `COPYING`.
fn is_license_file(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// The text of the file `name` of the package in `folder`, in the form of [`normal`].
fn text(folder: &Path, name: &str, label: &str) -> io::Result<String> {
    let bytes = fs::read(folder.join(name))
        .map_err(|error| io::Error::new(error.kind(), format!("{name} of {label}: {error}")))?;
    let text = String::from_utf8(bytes)
        .map_err(|error| io::Error::other(format!("{name} of {label} is not UTF-8: {error}")))?;
    Ok(normal(&text))
}

/// The form of a text that the file shows: LF at each line end, no space at the end of a line,
/// no empty line at the start or the end, no byte order mark. Some packages have CRLF, so the
/// same license text is then the same text.
pub(super) fn normal(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines: Vec<&str> = text.lines().map(str::trim_end).skip_while(|line| line.is_empty()).collect();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// The license identifiers of an SPDX expression, each one time, in their order: the words
/// with no operator and no parenthesis. An exception after `WITH` is a word too. Old manifests
/// write `MIT/Apache-2.0` for `MIT OR Apache-2.0`.
pub(super) fn identifiers(expression: &str) -> Vec<String> {
    let mut identifiers: Vec<String> = Vec::new();
    let words = expression.split(|character: char| character.is_whitespace() || "()/".contains(character));
    for word in words.filter(|word| !word.is_empty()) {
        let operator = ["OR", "AND", "WITH"].iter().any(|operator| word.eq_ignore_ascii_case(operator));
        if !operator && !identifiers.iter().any(|known| known == word) {
            identifiers.push(word.to_owned());
        }
    }
    identifiers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identifiers_are_the_licenses_of_an_expression_each_one_time() {
        assert_eq!(identifiers("MIT OR Apache-2.0"), ["MIT", "Apache-2.0"]);
        assert_eq!(identifiers("(MIT OR Apache-2.0) AND Unicode-3.0"), ["MIT", "Apache-2.0", "Unicode-3.0"]);
        assert_eq!(identifiers("MIT/Apache-2.0"), ["MIT", "Apache-2.0"]);
        assert_eq!(identifiers("Apache-2.0 / MIT"), ["Apache-2.0", "MIT"]);
        assert_eq!(identifiers("MPL-2.0+"), ["MPL-2.0+"]);
        assert_eq!(
            identifiers("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT"),
            ["Apache-2.0", "LLVM-exception", "MIT"]
        );
        assert_eq!(identifiers("MIT or Zlib"), ["MIT", "Zlib"]);
    }

    #[test]
    fn two_forms_of_one_text_are_the_same_text() {
        let unix = "MIT License\n\nCopyright (c) 2014 A\n";
        assert_eq!(normal(unix), "MIT License\n\nCopyright (c) 2014 A");
        assert_eq!(normal("\u{feff}\r\n\r\nMIT License  \r\n\r\nCopyright (c) 2014 A\t\r\n\r\n"), normal(unix));
        assert_eq!(normal("\n\n"), "");
        assert_ne!(normal("Copyright (c) 2014 A"), normal("Copyright (c) 2014 B"));
    }

    #[test]
    fn a_license_file_has_one_of_the_names_in_any_case() {
        for name in ["LICENSE", "LICENSE-MIT", "license-apache-2.0", "LICENCE.md", "COPYING", "NOTICE", "UNLICENSE"] {
            assert!(is_license_file(name), "{name}");
        }
        for name in ["README.md", "Cargo.toml", "src", "MIT-LICENSE"] {
            assert!(!is_license_file(name), "{name}");
        }
    }
}
