//! The size limits of the documents and the source files.
//!
//! A short file holds one task. A short document gets read. A limit that no tool checks does
//! not hold, thus CI runs this check.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The documents with a limit for their count of lines.
const DOCUMENT_LINES: [(&str, usize); 3] = [("README.md", 80), ("docs/architecture.md", 200), ("CONTRIBUTING.md", 150)];
/// The document with one short paragraph below each `##` heading.
const DECISIONS: &str = "docs/decisions.md";
const PARAGRAPH_WORDS: usize = 80;
/// The folders that hold source files, and the limit for the lines of one file.
const SOURCE_FOLDERS: [&str; 2] = ["crates", "xtask"];
const SOURCE_LINES: usize = 400;

/// The result of the check: the count of files that it read, and one line for each problem.
#[derive(Debug)]
pub struct Sizes {
    /// The count of files that the check read.
    pub files: usize,
    /// One line for each problem.
    pub problems: Vec<String>,
}

/// Checks each limit below `root`. A file that the check cannot read is a problem.
pub fn check(root: &Path) -> Sizes {
    let mut sizes = Sizes { files: 0, problems: Vec::new() };
    for (document, limit) in DOCUMENT_LINES {
        sizes.read(root, document, |text| line_problem(document, text, limit).into_iter().collect());
    }
    sizes.read(root, DECISIONS, decision_problems);
    for folder in SOURCE_FOLDERS {
        let mut files = Vec::new();
        if let Err(error) = source_files(&root.join(folder), &mut files) {
            sizes.problems.push(format!("{folder}: cannot read the folder: {error}"));
        }
        files.sort();
        for file in files {
            let name = file.strip_prefix(root).unwrap_or(&file).display().to_string();
            sizes.read(root, &name, |text| line_problem(&name, text, SOURCE_LINES).into_iter().collect());
        }
    }
    sizes
}

impl Sizes {
    fn read(&mut self, root: &Path, file: &str, problems: impl FnOnce(&str) -> Vec<String>) {
        self.files += 1;
        match fs::read_to_string(root.join(file)) {
            Ok(text) => self.problems.extend(problems(&text)),
            Err(error) => self.problems.push(format!("{file}: cannot read the file: {error}")),
        }
    }
}

fn line_problem(file: &str, text: &str, limit: usize) -> Option<String> {
    let lines = text.lines().count();
    (lines > limit).then(|| format!("{file}: {lines} lines, the limit is {limit}"))
}

/// The rule of the decisions document: each `##` section is one paragraph of
/// `PARAGRAPH_WORDS` words or fewer.
fn decision_problems(text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let text = format!("\n{text}");
    for section in text.split("\n## ").skip(1) {
        let (title, body) = section.split_once('\n').unwrap_or((section, ""));
        let paragraphs = paragraph_words(body);
        if paragraphs.len() != 1 {
            problems.push(format!("{DECISIONS}: \"{title}\" has {} paragraphs, the rule is 1", paragraphs.len()));
        }
        if let Some(words) = paragraphs.iter().find(|words| **words > PARAGRAPH_WORDS) {
            problems.push(format!("{DECISIONS}: \"{title}\" has {words} words, the limit is {PARAGRAPH_WORDS}"));
        }
    }
    problems
}

/// The count of words of each paragraph. An empty line ends a paragraph.
fn paragraph_words(body: &str) -> Vec<usize> {
    let mut paragraphs = Vec::new();
    let mut words = 0;
    for line in body.lines().chain([""]) {
        let line_words = line.split_whitespace().count();
        if line_words > 0 {
            words += line_words;
        } else if words > 0 {
            paragraphs.push(words);
            words = 0;
        }
    }
    paragraphs
}

/// Collects the `.rs` files below `folder`. Build output is not source.
fn source_files(folder: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if entry.file_name() != "target" {
                source_files(&path, files)?;
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_at_the_limit_passes_and_one_more_line_fails() {
        assert_eq!(line_problem("a.rs", &"line\n".repeat(400), 400), None);
        let problem = line_problem("a.rs", &"line\n".repeat(401), 400);
        assert_eq!(problem.as_deref(), Some("a.rs: 401 lines, the limit is 400"));
    }

    #[test]
    fn a_decision_is_one_short_paragraph() {
        let good = format!(
            "# Decisions\n\nAn intro.\n\nMore intro.\n\n## A\n\n{}\n\n## B\n\nTwo\nlines.\n",
            "word ".repeat(80)
        );
        assert_eq!(decision_problems(&good), Vec::<String>::new());
    }

    #[test]
    fn a_long_or_split_or_empty_decision_fails() {
        let long = format!("## Long\n\n{}\n", "word ".repeat(81));
        assert_eq!(decision_problems(&long), ["docs/decisions.md: \"Long\" has 81 words, the limit is 80"]);
        let split = decision_problems("# T\n\n## Split\n\nOne.\n\nTwo.\n");
        assert_eq!(split, ["docs/decisions.md: \"Split\" has 2 paragraphs, the rule is 1"]);
        let empty = decision_problems("## Empty\n\n## Next\n\nText.\n");
        assert_eq!(empty, ["docs/decisions.md: \"Empty\" has 0 paragraphs, the rule is 1"]);
    }

    #[test]
    fn the_walk_finds_source_files_and_not_other_files() {
        let mut files = Vec::new();
        source_files(Path::new(env!("CARGO_MANIFEST_DIR")), &mut files).expect("the folder of this crate");
        assert!(files.iter().any(|file| file.ends_with("src/sizes.rs")), "{files:?}");
        assert!(files.iter().all(|file| file.extension().is_some_and(|extension| extension == "rs")), "{files:?}");
    }

    #[test]
    fn a_missing_document_is_a_problem() {
        let sizes = check(Path::new(env!("CARGO_MANIFEST_DIR")));
        assert!(sizes.problems.iter().any(|problem| problem.starts_with("README.md: cannot read the file")));
        assert!(sizes.problems.iter().any(|problem| problem.starts_with("crates: cannot read the folder")));
    }
}
