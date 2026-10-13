//! What a store reads of an object: its metadata.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// An object that a store can hold. The store reads the name, the namespace, the UID and
/// the resource version, and it removes the fields that no view shows.
pub trait Object: DeserializeOwned + Send + Sync + 'static {
    /// The metadata of the object.
    fn meta(&self) -> &ObjectMeta;

    /// The metadata of the object, for a change.
    fn meta_mut(&mut self) -> &mut ObjectMeta;
}

impl<K> Object for K
where
    K: k8s_openapi::Metadata<Ty = ObjectMeta> + DeserializeOwned + Send + Sync + 'static,
{
    fn meta(&self) -> &ObjectMeta {
        self.metadata()
    }

    fn meta_mut(&mut self) -> &mut ObjectMeta {
        self.metadata_mut()
    }
}

/// The metadata of one object, and nothing else. A tier B store holds this type: the app
/// asks the server for the metadata form of the kind, so the data of a Secret or of a
/// ConfigMap never arrives.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Meta {
    /// The metadata of the object.
    pub metadata: ObjectMeta,
}

impl Object for Meta {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }

    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The metadata form of the server has a kind of its own and no field of the object.
    #[test]
    fn the_metadata_form_of_the_server_gives_the_metadata() {
        let text = r#"{"kind":"PartialObjectMetadata","apiVersion":"meta.k8s.io/v1",
            "metadata":{"name":"db","namespace":"shop","uid":"u-1","resourceVersion":"42"}}"#;
        let meta: Meta = serde_json::from_str(text).expect("the metadata form");
        let read = meta.meta();
        assert_eq!((read.name.as_deref(), read.namespace.as_deref()), (Some("db"), Some("shop")));
        assert_eq!((read.uid.as_deref(), read.resource_version.as_deref()), (Some("u-1"), Some("42")));
    }
}
