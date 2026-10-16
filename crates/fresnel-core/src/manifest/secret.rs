//! The values of a Secret in the text of its editor (plan section 12.9, research 05 section
//! 6.11).
//!
//! The server gives each value of `data` as base64. The editor shows a value that is text by
//! the rule of the key view (`keys::Value`) decoded, so the user reads and edits it, and each
//! other value as its base64 with the YAML tag `!!binary`: the mark of a value that a text
//! editor cannot hold. The YAML writer writes that tag itself for bytes, so the app writes
//! no YAML of its own. The save encodes each text value again.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::{Map, Value};

use super::Decoded;
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
    let secret = Secret { fields, values: &values };
    let text = serde_saphyr::to_string(&secret).map_err(|_| CoreError::invalid("the object has no YAML form"))?;
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
                map.serialize_entry(key, value)?;
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
                Some(text) => serializer.serialize_str(text),
                None => serializer.serialize_bytes(value.bytes()),
            },
            Self::Other(value) => value.serialize(serializer),
        }
    }
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
        let values = [b"a\r\nb".to_vec(), b"  lead and tail  \n\n".to_vec(), b"yes".to_vec(), vec![0, 1, 2, 255]];
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

    #[test]
    fn a_value_that_is_not_base64_fails_with_no_value_in_its_text() {
        let error = yaml(&json!({"kind": "Secret", "data": {"key": "not base64!"}})).expect_err("no base64");
        assert_eq!(error.to_string(), "a value of the Secret is not base64");
    }
}
