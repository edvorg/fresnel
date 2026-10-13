//! The removal of metadata that no view of a list shows, before an object goes into a store.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

/// `kubectl apply` keeps a full copy of the object in this annotation. For a Secret, the
/// copy holds each value, and the server sends annotations also with the metadata form.
const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

/// Removes `managedFields` and the copy of `kubectl apply`. In the store prototype,
/// `managedFields` were 56 percent of a pod in memory. The manifest view reads the object
/// again from the server.
pub(super) fn strip(meta: &mut ObjectMeta) {
    meta.managed_fields = None;
    if let Some(annotations) = meta.annotations.as_mut() {
        annotations.remove(LAST_APPLIED);
        if annotations.is_empty() {
            meta.annotations = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ManagedFieldsEntry;

    use super::*;

    #[test]
    fn the_managed_fields_and_the_copy_of_kubectl_apply_go_and_each_other_annotation_stays() {
        let annotations = |entries: &[(&str, &str)]| {
            Some(entries.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect())
        };
        let mut meta = ObjectMeta {
            managed_fields: Some(vec![ManagedFieldsEntry::default()]),
            annotations: annotations(&[(LAST_APPLIED, r#"{"data":{"password":"c2VjcmV0"}}"#), ("team", "shop")]),
            ..ObjectMeta::default()
        };
        strip(&mut meta);
        assert_eq!((meta.managed_fields.as_ref(), &meta.annotations), (None, &annotations(&[("team", "shop")])));

        let mut only_copy = ObjectMeta { annotations: annotations(&[(LAST_APPLIED, "{}")]), ..ObjectMeta::default() };
        strip(&mut only_copy);
        assert_eq!(only_copy, ObjectMeta::default(), "an empty map of annotations goes too");
    }
}
