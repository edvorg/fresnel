//! The text of the editor of one object (plan section 12.9, research 05 section 6.1).
//!
//! The text is the YAML of one read of the server with no `managedFields`, in the form of
//! the Manifest tab: the keys of each map in the sequence of the alphabet, as kubectl
//! writes them. A save with no `managedFields` keeps them on the server. A Secret shows each
//! value that is text decoded (`secret`).

use std::sync::Arc;

use serde_json::Value;

use super::{read, remove_managed_fields, secret, version, yaml};
use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// The most lines of a text that the editor edits (plan section 12.9). Each change gets a
/// check of the full text, which takes some milliseconds for 8,000 lines, and few objects
/// have more. A larger manifest opens read-only.
pub const LINE_LIMIT: usize = 8_000;

/// The text of the editor of one object at one moment.
#[derive(Clone, PartialEq, Eq)]
pub struct EditText {
    /// The YAML of the object.
    pub text: Arc<str>,
    /// The resource version of the object that the text shows.
    pub version: Option<String>,
    /// The count of the lines of the text.
    pub lines: usize,
    /// What the text shows of the values of a Secret. `None` for each other kind.
    pub decoded: Option<Decoded>,
}

/// The values of a Secret in the text of its editor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    /// The count of values that show as text.
    pub texts: usize,
    /// The count of values that stay base64, with the tag `!!binary`.
    pub binary: usize,
}

impl std::fmt::Debug for EditText {
    /// The size of the text, never the text: the text of a Secret has its values, and a
    /// debug text can go to a log (plan section 12.6).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditText")
            .field("bytes", &self.text.len())
            .field("version", &self.version)
            .field("lines", &self.lines)
            .field("decoded", &self.decoded)
            .finish()
    }
}

/// Reads one object from the server for its editor, with the time limit of a GET. `uid` is
/// the UID of the object that the user edits: a read that finds an other object fails.
pub async fn read_for_edit(gateway: &Gateway, kind: Kind, key: &ObjectKey, uid: &str) -> Result<EditText, CoreError> {
    text_of(read(gateway, kind, key, uid).await?, kind)
}

/// The text of the editor of an object as the server gave it.
pub(super) fn text_of(mut object: Value, kind: Kind) -> Result<EditText, CoreError> {
    remove_managed_fields(&mut object);
    let version = version(&object);
    let (text, decoded) = if kind == Kind::Secret {
        let (text, decoded) = secret::yaml(&object)?;
        (text, Some(decoded))
    } else {
        (yaml::write(&yaml::Json(&object))?, None)
    };
    Ok(EditText { lines: text.lines().count(), text: text.into(), version, decoded })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_text_has_no_managed_fields_and_keeps_each_other_field_and_its_version() {
        let pod = json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "web", "namespace": "shop",
            "resourceVersion": "42", "managedFields": [{"manager": "kubectl"}]},
            "spec": {"containers": [{"name": "web", "image": "nginx:1.27", "env": [{"name": "A", "value": "yes"}]}]}});
        let text = text_of(pod, Kind::Pod).expect("a text");
        assert!(!text.text.contains("managedFields") && text.text.contains("resourceVersion: \"42\""));
        assert!(
            text.text.contains("value: 'yes'") || text.text.contains("value: \"yes\""),
            "a text that kubectl reads as a text"
        );
        assert_eq!((text.version.as_deref(), text.lines, text.decoded), (Some("42"), 13, None));
        let debug = format!("{text:?}");
        assert!(debug.contains("bytes") && !debug.contains("nginx"), "no value of the object: {debug}");
    }
}
