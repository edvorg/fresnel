//! The YAML writer of the manifests: serde-saphyr with two corrections.
//!
//! serde-saphyr 0.0.29 writes a text of only line feeds as a block scalar that reads as a
//! shorter text: "\n" as `|` with an empty line, which reads as "". Its parser drops a space
//! that is not ASCII, such as U+00A0, at the start and the end of a plain text before it reads
//! a number, so "12\u{a0}" reads as the number 12, while its writer writes the text plain.
//! The Manifest tab, a download and the text of an editor would then give a value that the
//! object does not have, and a save would send it. Such texts go in double quotes; each other
//! text keeps the form of the writer.

use serde::ser::{Serialize, Serializer};
use serde_json::Value;
use serde_saphyr::DoubleQuoted;

use crate::error::CoreError;

/// Writes a value as YAML.
pub(super) fn write(value: &impl Serialize) -> Result<String, CoreError> {
    // The text of a YAML error can hold a value of the object, so the error has none.
    serde_saphyr::to_string(value).map_err(|_| CoreError::invalid("the object has no YAML form"))
}

/// A JSON value whose texts the writer writes by `text`.
pub(super) struct Json<'a>(pub(super) &'a Value);

impl Serialize for Json<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::String(value) => text(value, serializer),
            Value::Array(items) => serializer.collect_seq(items.iter().map(Json)),
            Value::Object(fields) => serializer.collect_map(fields.iter().map(|(key, value)| (key, Json(value)))),
            other => other.serialize(serializer),
        }
    }
}

/// Writes a text, in double quotes when it has only line feeds, or a space that is not ASCII
/// at its start or its end.
pub(super) fn text<S: Serializer>(value: &str, serializer: S) -> Result<S::Ok, S::Error> {
    let wide = |edge: Option<char>| edge.is_some_and(|edge| edge.is_whitespace() && !edge.is_ascii());
    let feeds = !value.is_empty() && value.bytes().all(|byte| byte == b'\n');
    if feeds || wide(value.chars().next()) || wide(value.chars().next_back()) {
        DoubleQuoted(value).serialize(serializer)
    } else {
        serializer.serialize_str(value)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, json};

    use super::*;
    use crate::manifest::check;

    /// Each text reads back as itself, also a text of only line feeds and a number with a
    /// space that is not ASCII at an edge; a text with other characters keeps the block of
    /// the writer.
    #[test]
    fn each_text_reads_back_as_itself_and_a_text_of_only_line_feeds_has_quotes() {
        let texts = [
            "\n",
            "\n\n",
            "\n\n\n",
            "",
            " ",
            "a\n",
            "a\n\n",
            "\na",
            " a\n b\n",
            "a\n \n",
            "x\u{2028}y",
            "a\r",
            "tab\there",
            "yes",
            "0644",
            "- a",
            "#x",
            "a: b",
            "~",
            "12\u{a0}",
            "\u{3000}0x1F",
            "\u{2009}.inf",
            "\u{a0}",
        ];
        let data: Map<String, Value> =
            texts.iter().enumerate().map(|(index, text)| (format!("k{index:02}"), Value::from(*text))).collect();
        let object = json!({"kind": "ConfigMap", "data": data, "list": texts});
        let written = write(&Json(&object)).expect("a text");
        assert_eq!(check::parse(&written).expect("valid YAML"), object, "{written}");
        assert!(written.contains("  k00: \"\\n\"\n  k01: \"\\n\\n\"\n"), "{written}");
        let certificate = write(&Json(&json!({"ca.crt": "-----BEGIN-----\nMIIB\n"}))).expect("a text");
        assert_eq!(certificate, "ca.crt: |\n  -----BEGIN-----\n  MIIB\n");
    }
}
