//! The values of a Secret in the text of its editor (plan section 12.9, research 05 section
//! 6.11).
//!
//! The server gives each value of `data` as base64. The editor shows a value that is text by
//! the rule of the key view (`keys::Value`) decoded, so the user reads and edits it, and each
//! other value as its base64 with the YAML tag `!!binary`: the mark of a value that a text
//! editor cannot hold. The YAML writer writes that tag itself for bytes, so the app writes
//! no YAML of its own. The save encodes each text value again (`encode`).

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::{Map, Value};
use serde_saphyr::granit_parser::{Event, Parser};

use super::Decoded;
use super::check::is_binary;
use super::yaml::{self, Json};
use crate::error::CoreError;
use crate::keys::{Form, Value as KeyValue};

/// The YAML of a Secret with its values decoded.
pub(super) fn yaml(object: &Value) -> Result<(String, Decoded), CoreError> {
    let empty = Map::new();
    let fields = object.as_object().unwrap_or(&empty);
    let mut decoded = Decoded::default();
    let mut values = Vec::new();
    for (key, value) in fields.get("data").and_then(Value::as_object).unwrap_or(&empty) {
        let shown = match value.as_str() {
            Some(base64) => {
                let bytes =
                    STANDARD.decode(base64).map_err(|_| CoreError::invalid("a value of the Secret is not base64"))?;
                let value = KeyValue::new(bytes);
                match value.form() {
                    Form::Text => decoded.texts += 1,
                    Form::Binary(_) => decoded.binary += 1,
                }
                Shown::Decoded(value)
            }
            None => Shown::Other(value),
        };
        values.push((key.as_str(), shown));
    }
    let text = yaml::write(&Secret { fields, values: &values })?;
    Ok((text, decoded))
}

/// A Secret with the values of its `data` in place of the base64 of the server.
struct Secret<'a> {
    fields: &'a Map<String, Value>,
    values: &'a [(&'a str, Shown<'a>)],
}

/// One value of `data` in the text.
enum Shown<'a> {
    Decoded(KeyValue),
    /// A value that is not a text of base64. The server sends none such; it stays as it is.
    Other(&'a Value),
}

impl Serialize for Secret<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.fields.len()))?;
        for (key, value) in self.fields {
            if key == "data" && value.is_object() {
                map.serialize_entry(key, &Data(self.values))?;
            } else {
                map.serialize_entry(key, &Json(value))?;
            }
        }
        map.end()
    }
}

struct Data<'a>(&'a [(&'a str, Shown<'a>)]);

impl Serialize for Data<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl Serialize for Shown<'_> {
    /// A text as a string; other bytes as bytes, which the YAML writer writes as base64 with
    /// the tag `!!binary`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Decoded(value) => match value.text() {
                Some(text) => yaml::text(text, serializer),
                None => serializer.serialize_bytes(value.bytes()),
            },
            Self::Other(value) => Json(value).serialize(serializer),
        }
    }
}

/// Encodes the values of `data` of a Secret from the text of its editor again: a text as the
/// base64 of its bytes, and a value with the tag `!!binary`, which the parser reads as its
/// base64, as it is with no space. Each other value stays as the text has it: the server
/// refuses a number there, and the user sees why.
pub(super) fn encode(text: &str, object: &mut Value) {
    let binary = binary_keys(text);
    let Some(data) = object.get_mut("data").and_then(Value::as_object_mut) else {
        return;
    };
    for (key, value) in data.iter_mut() {
        if let Value::String(value) = value {
            *value = if binary.contains(key) {
                value.split_whitespace().collect()
            } else {
                STANDARD.encode(value.as_bytes())
            };
        }
    }
}

/// The keys of `data` in the first document whose value has the tag `!!binary`, also through
/// an alias of such a value. The parser keeps no tag in the value that it gives.
fn binary_keys(text: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    // An open map with the key of the value that comes now (`None` while a key comes), or a list.
    let mut stack: Vec<Option<Option<String>>> = Vec::new();
    let mut anchors = BTreeSet::new();
    for event in Parser::new_from_str(text) {
        let Ok((event, _)) = event else {
            break;
        };
        let binary = match event {
            Event::DocumentEnd => break,
            Event::Scalar(value, _, anchor, tag) => {
                let binary = tag.as_deref().is_some_and(is_binary);
                if binary && anchor != 0 {
                    anchors.insert(anchor);
                }
                Some((binary, value.into_owned()))
            }
            Event::Alias(anchor) => Some((anchors.contains(&anchor), String::new())),
            Event::MappingStart(..) => {
                stack.push(Some(None));
                None
            }
            Event::SequenceStart(..) => {
                stack.push(None);
                None
            }
            Event::MappingEnd | Event::SequenceEnd => {
                stack.pop();
                Some((false, String::new()))
            }
            _ => None,
        };
        let Some((binary, text)) = binary else {
            continue;
        };
        if let [Some(Some(data)), Some(Some(key))] = stack.as_slice()
            && data == "data"
            && binary
        {
            keys.insert(key.clone());
        }
        // The end of a node: a key, or the value of a key.
        if let Some(Some(current)) = stack.last_mut() {
            *current = if current.is_none() { Some(text) } else { None };
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn base64(bytes: &[u8]) -> String {
        STANDARD.encode(bytes)
    }

    #[test]
    fn a_text_value_shows_decoded_and_each_other_value_stays_base64_with_its_mark() {
        let binary = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0xff];
        let secret = json!({"apiVersion": "v1", "kind": "Secret", "type": "Opaque",
            "metadata": {"name": "db", "namespace": "shop"},
            "data": {"password": base64(b"hunter2"), "image.png": base64(&binary),
                "ca.crt": base64(b"-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n"),
                "bell": base64(b"ding\x07"), "empty": ""}});
        let (text, decoded) = yaml(&secret).expect("a text");
        let expected = format!(
            "apiVersion: v1\ndata:\n  bell: !!binary {}\n  ca.crt: |\n    -----BEGIN CERTIFICATE-----\n    MIIB\n    \
             -----END CERTIFICATE-----\n  empty: \"\"\n  image.png: !!binary {}\n  password: hunter2\nkind: Secret\n\
             metadata:\n  name: db\n  namespace: shop\ntype: Opaque\n",
            base64(b"ding\x07"),
            base64(&binary)
        );
        assert_eq!(text, expected, "a control character is no text for the editor either");
        assert_eq!(decoded, Decoded { texts: 3, binary: 2 });
    }

    /// The text of the editor parses back to each value: a text as itself, a binary value as
    /// the base64 that the save sends as it is.
    #[test]
    fn the_parser_reads_a_decoded_text_as_the_text_and_a_binary_value_as_its_base64() {
        let values = [
            b"a\r\nb".to_vec(),
            b"  lead and tail  \n\n".to_vec(),
            b"yes".to_vec(),
            b"\n".to_vec(),
            vec![0, 1, 2, 255],
        ];
        let data: Map<String, Value> =
            values.iter().enumerate().map(|(index, bytes)| (format!("k{index}"), base64(bytes).into())).collect();
        let (text, _) = yaml(&json!({"kind": "Secret", "data": data})).expect("a text");
        let options = serde_saphyr::options! { ignore_binary_tag_for_string: true };
        let parsed: Value = serde_saphyr::from_str_with_options(&text, options).expect("valid YAML");
        for (index, bytes) in values.iter().enumerate() {
            let read = parsed.pointer(&format!("/data/k{index}")).and_then(Value::as_str).expect("a value");
            match std::str::from_utf8(bytes) {
                Ok(text) => assert_eq!(read, text, "{text:?}"),
                Err(_) => assert_eq!(read, base64(bytes)),
            }
        }
    }

    /// The text of the editor of a Secret encodes back to the base64 of the server, each value
    /// byte for byte; a changed text value gets its new base64.
    #[test]
    fn the_save_encodes_each_value_again_and_a_binary_value_stays_as_it_is() {
        let binary = [0x89, b'P', b'N', b'G', 0xff, 0x00];
        let data = json!({"password": base64(b"hunter2"), "image.png": base64(&binary),
            "crlf": base64(b"a\r\nb"), "only": base64(b"\n"), "empty": ""});
        let secret = json!({"apiVersion": "v1", "kind": "Secret", "metadata": {"name": "db"}, "data": data});
        let (text, _) = yaml(&secret).expect("a text");
        let mut object = crate::manifest::check::parse(&text).expect("valid YAML");
        encode(&text, &mut object);
        assert_eq!(object, secret);

        let edited = text.replace("password: hunter2", "password: hunter3");
        let mut object = crate::manifest::check::parse(&edited).expect("valid YAML");
        encode(&edited, &mut object);
        assert_eq!(object.pointer("/data/password"), Some(&json!(base64(b"hunter3"))));
        assert_eq!(object.pointer("/data/image.png"), Some(&json!(base64(&binary))));
    }

    /// A !!binary value through an alias stays base64, a !!binary value outside `data` and
    /// a binary value split on lines are no exception, and a number stays a number.
    #[test]
    fn the_tag_counts_only_on_a_value_of_data_and_through_an_alias() {
        let text = "data:\n  a: !!binary &bin AAEC\n  b: *bin\n  c: !!binary |\n    AAEC\n    AwQ=\n  d: 1234\n  \
                    e: AAEC\nmetadata:\n  note: !!binary AAEC\n";
        let mut object = crate::manifest::check::parse(text).expect("valid YAML");
        encode(text, &mut object);
        let data = json!({"a": "AAEC", "b": "AAEC", "c": "AAECAwQ=", "d": 1234, "e": base64(b"AAEC")});
        assert_eq!(object, json!({"data": data, "metadata": {"note": "AAEC"}}));
    }

    /// Random values of the characters that YAML reads in a special way: the text of the
    /// editor saves each one back to its bytes, so a save changes no value that the user did
    /// not edit.
    #[test]
    fn each_random_value_saves_back_to_its_bytes() {
        let characters = [
            " ", "\n", "\r", "\t", ":", "#", "-", "\"", "'", "|", ">", "!", "&", "*", "%", "@", "`", ",", "[", "]",
            "{", "}", "?", "a", "y", "s", "0", "7", ".", "n", "~", "\u{2028}", "\u{85}", "\u{feff}", "é", "\u{a0}",
            "\\", "=", "<<", "null", "true", "0x1F", "1e3", "---", "...", "\u{1}",
        ];
        let mut random = fastrand::Rng::with_seed(11);
        for _ in 0..3_000 {
            let value = |random: &mut fastrand::Rng| -> String {
                (0..random.usize(0..8)).map(|_| characters[random.usize(0..characters.len())]).collect()
            };
            let data: Map<String, Value> =
                (0..4).map(|index| (format!("k{index}"), base64(value(&mut random).as_bytes()).into())).collect();
            let secret = json!({"kind": "Secret", "data": data});
            let (text, _) = yaml(&secret).expect("a text");
            let mut object = crate::manifest::check::parse(&text).expect("valid YAML");
            encode(&text, &mut object);
            assert_eq!(object, secret, "{text}");
        }
    }

    #[test]
    fn a_value_that_is_not_base64_fails_with_no_value_in_its_text() {
        let error = yaml(&json!({"kind": "Secret", "data": {"key": "not base64!"}})).expect_err("no base64");
        assert_eq!(error.to_string(), "a value of the Secret is not base64");
    }
}
