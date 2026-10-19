//! The text of `THIRD-PARTY-LICENSES`: a head, the font JetBrains Mono, the crates with their
//! licenses, the license texts of the crates and the licenses of fresnel. A text that several
//! crates carry is in it one time, and the same crates give the same bytes in any order.

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use super::crates::TARGETS;
use super::package::{Crate, identifiers, normal};

/// The width of a line of the file, so a text editor shows it with no wrap.
const WIDTH: usize = 100;

/// The license texts of the repository that the file carries.
pub(super) struct Ours<'a> {
    /// The SIL Open Font License of JetBrains Mono, with its copyright.
    pub(super) font: &'a str,
    pub(super) mit: &'a str,
    pub(super) apache: &'a str,
}

/// The text of the file, and the count of different license texts of the crates in it.
pub(super) struct Document {
    pub(super) text: String,
    pub(super) texts: usize,
}

/// A license text of the crates.
struct Text<'a> {
    body: &'a str,
    /// The file and the crate of its first use, as "LICENSE-MIT of serde 1.0.229".
    first: String,
    /// The count of crates that carry it.
    crates: usize,
}

/// The standard text of a license: the number of the text and the file that has it.
struct Standard {
    number: usize,
    file: String,
}

/// The file for the crates of others in the program. A crate with no license file whose
/// license has no standard text stops it: the file would miss the text of that license.
pub(super) fn write(crates: &[Crate], ours: &Ours<'_>) -> io::Result<Document> {
    let mut crates: Vec<&Crate> = crates.iter().collect();
    crates.sort_by(|one, other| (&one.name, &one.version).cmp(&(&other.name, &other.version)));
    let (texts, numbers) = number(&crates);
    let standards = standards(&crates, &numbers);

    let mut out = String::new();
    head(&mut out, crates.len(), texts.len());
    heading(&mut out, "Part 1: JetBrains Mono", '=');
    paragraph(&mut out, "The font JetBrains Mono, in the program unchanged, is under the SIL Open Font License 1.1:");
    body(&mut out, ours.font);
    heading(&mut out, "Part 2: Crates", '=');
    for (one, numbers) in crates.iter().zip(&numbers) {
        entry(&mut out, one, numbers, &standards)?;
    }
    heading(&mut out, "Part 3: Texts", '=');
    for (index, text) in texts.iter().enumerate() {
        let others = match text.crates.saturating_sub(1) {
            0 => String::new(),
            1 => " and 1 other crate".to_owned(),
            count => format!(" and {count} other crates"),
        };
        heading(&mut out, &format!("Text {}: {}{others}", index + 1, text.first), '-');
        body(&mut out, text.body);
    }
    heading(&mut out, "Part 4: Fresnel", '=');
    for (name, text) in [("LICENSE-MIT", ours.mit), ("LICENSE-APACHE", ours.apache)] {
        heading(&mut out, name, '-');
        body(&mut out, text);
    }
    let text = format!("{}\n", out.trim_end());
    Ok(Document { text, texts: texts.len() })
}

/// The texts of the crates, each one time in the order of its first use, and for each crate
/// the number of the text of each of its files, from 1.
fn number<'a>(crates: &[&'a Crate]) -> (Vec<Text<'a>>, Vec<Vec<usize>>) {
    let mut texts: Vec<Text<'a>> = Vec::new();
    let mut known: BTreeMap<&'a str, usize> = BTreeMap::new();
    let mut numbers = Vec::new();
    for one in crates {
        let mut of_crate = Vec::new();
        for file in &one.files {
            let number = *known.entry(file.text.as_str()).or_insert_with(|| {
                texts.push(Text { body: &file.text, first: format!("{} of {}", file.name, one.label()), crates: 0 });
                texts.len()
            });
            of_crate.push(number);
        }
        let carried: BTreeSet<usize> = of_crate.iter().copied().collect();
        for number in carried {
            if let Some(text) = texts.get_mut(number - 1) {
                text.crates += 1;
            }
        }
        numbers.push(of_crate);
    }
    (texts, numbers)
}

/// The standard text of each license: the first file, in the order of the crates, that is the
/// text of that license alone.
fn standards(crates: &[&Crate], numbers: &[Vec<usize>]) -> BTreeMap<String, Standard> {
    let mut standards = BTreeMap::new();
    for (one, numbers) in crates.iter().zip(numbers) {
        for (file, number) in one.files.iter().zip(numbers) {
            if let Some(license) = &file.license {
                let file = format!("{} of {}", file.name, one.label());
                standards.entry(license.clone()).or_insert(Standard { number: *number, file });
            }
        }
    }
    standards
}

/// The entry of a crate: its name and version, its license and the numbers of its texts. A
/// crate with no license file also gets the authors of its manifest: no file has its copyright.
fn entry(out: &mut String, one: &Crate, numbers: &[usize], standards: &BTreeMap<String, Standard>) -> io::Result<()> {
    out.push_str(&one.label());
    out.push('\n');
    wrap(out, "  License: ", "    ", &one.license);
    if !one.files.is_empty() {
        let files = one.files.iter().zip(numbers).map(|(file, number)| format!("{} (text {number})", file.name));
        wrap(out, "  Texts: ", "    ", &files.collect::<Vec<_>>().join(", "));
    }
    if one.bare {
        let covered: BTreeSet<&str> = one.files.iter().filter_map(|file| file.license.as_deref()).collect();
        let mut uses = Vec::new();
        for license in identifiers(&one.license).iter().filter(|license| !covered.contains(license.as_str())) {
            let Some(standard) = standards.get(license) else {
                let label = one.label();
                return Err(io::Error::other(format!("{label}: no license file and no standard text of {license}")));
            };
            uses.push(format!("{license} (text {}, {})", standard.number, standard.file));
        }
        if !uses.is_empty() {
            let note = format!("No license file at the root of its package; the standard texts: {}.", uses.join(", "));
            wrap(out, "  ", "    ", &note);
        }
        let authors = if one.authors.is_empty() { "none".to_owned() } else { one.authors.join(", ") };
        wrap(out, "  Authors in its manifest: ", "    ", &authors);
    }
    out.push('\n');
    Ok(())
}

/// The head: what the file has, and why.
fn head(out: &mut String, crates: usize, texts: usize) {
    heading(out, "The licenses of the parts of Fresnel", '=');
    let targets = TARGETS.join(" and ");
    let text = format!(
        "Fresnel is under the MIT license or the Apache License 2.0, at your option: part 4 has their \
         texts, as do LICENSE-MIT and LICENSE-APACHE next to this file. The program contains parts of \
         others, each one under its own license: the font JetBrains Mono (part 1) and {crates} crates \
         (part 2), its normal dependencies for {targets}, with no build or test dependency. Part 2 \
         gives the license of each crate and the numbers of the texts of its license files, and part 3 \
         has each of the {texts} texts one time. cargo xtask notices writes this file from cargo."
    );
    paragraph(out, &text);
}

/// A heading with a line of `mark` below it. A part (`=`) starts after two empty lines.
fn heading(out: &mut String, title: &str, mark: char) {
    if mark == '=' && !out.is_empty() {
        out.push('\n');
    }
    out.push_str(title);
    out.push('\n');
    out.extend(std::iter::repeat_n(mark, title.chars().count()));
    out.push_str("\n\n");
}

/// A text of a license file, and an empty line.
fn body(out: &mut String, text: &str) {
    out.push_str(&normal(text));
    out.push_str("\n\n");
}

/// A paragraph wrapped at `WIDTH`, and an empty line.
fn paragraph(out: &mut String, text: &str) {
    wrap(out, "", "", text);
    out.push('\n');
}

/// The words of `text` in lines of `WIDTH` characters at most, unless a word is longer: the first
/// line starts with `first`, each other one with `indent`.
fn wrap(out: &mut String, first: &str, indent: &str, text: &str) {
    let mut line = first.to_owned();
    let mut empty = true;
    for word in text.split(' ').filter(|word| !word.is_empty()) {
        if !empty && line.chars().count() + 1 + word.chars().count() > WIDTH {
            out.push_str(&line);
            out.push('\n');
            line = indent.to_owned();
            empty = true;
        }
        if !empty {
            line.push(' ');
        }
        line.push_str(word);
        empty = false;
    }
    out.push_str(&line);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::super::package::File;
    use super::*;

    const APACHE: &str = "Apache License\nVersion 2.0, January 2004\n";
    const OURS: Ours<'static> =
        Ours { font: "Copyright 2020 JetBrains\n\nOFL 1.1", mit: "MIT of fresnel", apache: "Apache of fresnel" };

    fn file(name: &str, text: &str, license: Option<&str>) -> File {
        File { name: name.to_owned(), text: normal(text), license: license.map(str::to_owned) }
    }

    fn one(label: &str, license: &str, files: Vec<File>) -> Crate {
        let (name, version) = label.split_once(' ').unwrap_or((label, "1.0.0"));
        let bare = files.iter().all(|file| file.name.contains('/'));
        let authors = vec![format!("The authors of {name}")];
        Crate { name: name.to_owned(), version: version.to_owned(), license: license.to_owned(), authors, files, bare }
    }

    /// Two crates with the same Apache text, a crate with one license and one file, a crate with
    /// no license file and the fonts of egui, whose notices are in a folder.
    fn crates() -> Vec<Crate> {
        vec![
            one(
                "serde 1.0.229",
                "MIT OR Apache-2.0",
                vec![file("LICENSE-APACHE", APACHE, None), file("LICENSE-MIT", "MIT of serde", None)],
            ),
            one("egui 0.36.2", "MIT OR Apache-2.0", Vec::new()),
            one("archery 1.2.3", "MIT", vec![file("LICENSE.md", "MIT of archery\r\n", Some("MIT"))]),
            one(
                "adler2 2.0.1",
                "0BSD OR Apache-2.0",
                vec![file("LICENSE-0BSD", "0BSD", None), file("LICENSE-APACHE", APACHE, None)],
            ),
            one("kube 4.2.0", "Apache-2.0", Vec::new()),
            one("codespan 0.13.1", "Apache-2.0", vec![file("LICENSE", APACHE, Some("Apache-2.0"))]),
            one(
                "epaint_default_fonts 0.36.2",
                "(MIT OR Apache-2.0) AND OFL-1.1",
                vec![file("fonts/Hack.txt", "Hack", None), file("fonts/OFL.txt", "OFL of Noto Emoji", Some("OFL-1.1"))],
            ),
        ]
    }

    fn entry_of<'a>(text: &'a str, label: &str) -> &'a str {
        let start = text.find(&format!("\n{label}\n")).expect("the entry of the crate") + 1;
        let end = text[start..].find("\n\n").expect("the end of the entry");
        &text[start..start + end]
    }

    #[test]
    fn a_text_that_several_crates_carry_is_in_the_file_one_time() {
        let document = write(&crates(), &OURS).expect("the file");
        let text = &document.text;
        assert_eq!(
            entry_of(text, "adler2 2.0.1"),
            "adler2 2.0.1\n  License: 0BSD OR Apache-2.0\n  Texts: LICENSE-0BSD (text 1), LICENSE-APACHE (text 2)"
        );
        assert!(
            entry_of(text, "serde 1.0.229").ends_with("  Texts: LICENSE-APACHE (text 2), LICENSE-MIT (text 6)"),
            "{text}"
        );
        assert!(text.contains("\nText 2: LICENSE-APACHE of adler2 2.0.1 and 2 other crates\n---"), "{text}");
        assert!(text.contains("\nText 3: LICENSE.md of archery 1.2.3\n------"), "{text}");
        assert_eq!(text.matches("Version 2.0, January 2004").count(), 1, "{text}");
        assert_eq!(document.texts, 6);
        assert!(text.contains("\n\nMIT of archery\n\n"), "the text has its LF form: {text}");
    }

    #[test]
    fn a_crate_with_no_license_file_gets_the_standard_text_of_each_license_and_its_authors() {
        let text = write(&crates(), &OURS).expect("the file").text;
        assert_eq!(
            entry_of(&text, "egui 0.36.2"),
            "egui 0.36.2\n  License: MIT OR Apache-2.0\n  No license file at the root of its package; the standard \
             texts: MIT (text 3, LICENSE.md of archery\n    1.2.3), Apache-2.0 (text 2, LICENSE of codespan 0.13.1).\n  \
             Authors in its manifest: The authors of egui"
        );
        // The notices of the fonts are the texts of their licenses, and the code has the standard texts.
        assert_eq!(
            entry_of(&text, "epaint_default_fonts 0.36.2"),
            "epaint_default_fonts 0.36.2\n  License: (MIT OR Apache-2.0) AND OFL-1.1\n  Texts: fonts/Hack.txt (text 4), \
             fonts/OFL.txt (text 5)\n  No license file at the root of its package; the standard texts: MIT (text 3, \
             LICENSE.md of archery\n    1.2.3), Apache-2.0 (text 2, LICENSE of codespan 0.13.1).\n  Authors in its \
             manifest: The authors of epaint_default_fonts"
        );
        assert!(!entry_of(&text, "serde 1.0.229").contains("Authors"), "a crate with license files: {text}");
    }

    #[test]
    fn a_license_with_no_standard_text_stops_the_file() {
        let mut crates = crates();
        crates.push(one("clipboard-win 5.4.1", "BSL-1.0", Vec::new()));
        let error = write(&crates, &OURS).map(|_| ()).expect_err("no text of BSL-1.0");
        assert_eq!(error.to_string(), "clipboard-win 5.4.1: no license file and no standard text of BSL-1.0");
    }

    #[test]
    fn the_file_has_its_parts_and_the_same_crates_in_an_other_order_give_the_same_bytes() {
        let text = write(&crates(), &OURS).expect("the file").text;
        let mut reversed = crates();
        reversed.reverse();
        assert_eq!(write(&reversed, &OURS).expect("the file").text, text);

        let lines: Vec<&str> = text.lines().collect();
        let parts: Vec<&str> = lines
            .windows(2)
            .filter(|pair| pair[1].starts_with("==") && pair[1].chars().all(|mark| mark == '='))
            .map(|pair| pair[0])
            .collect();
        let names = ["The licenses of the parts of Fresnel", "Part 1: JetBrains Mono", "Part 2: Crates"];
        assert_eq!(parts, [names.as_slice(), &["Part 3: Texts", "Part 4: Fresnel"]].concat());
        let head = text.replace('\n', " ");
        assert!(head.contains(
            "7 crates (part 2), its normal dependencies for aarch64-apple-darwin and x86_64-pc-windows-msvc"
        ));
        assert!(text.contains("\n\nCopyright 2020 JetBrains\n\nOFL 1.1\n\n\nPart 2: Crates\n"), "{text}");
        assert!(text.ends_with(
            "\nLICENSE-MIT\n-----------\n\nMIT of fresnel\n\nLICENSE-APACHE\n--------------\n\nApache of fresnel\n"
        ));
        assert!(text.lines().take_while(|line| !line.starts_with("Part 3")).all(|line| line.chars().count() <= WIDTH));
    }

    #[test]
    fn a_line_wraps_at_the_width_with_its_indent_and_keeps_a_long_word() {
        let mut out = String::new();
        let words = format!("{} {}", "word ".repeat(30), "x".repeat(120));
        wrap(&mut out, "  Texts: ", "    ", &words);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3, "{out}");
        assert!(lines[0].starts_with("  Texts: word ") && lines[0].len() <= WIDTH && lines[0].len() > WIDTH - 5);
        assert!(lines[1].starts_with("    word") && lines[1].ends_with("word"), "{out}");
        assert_eq!(lines[2], format!("    {}", "x".repeat(120)));
    }
}
