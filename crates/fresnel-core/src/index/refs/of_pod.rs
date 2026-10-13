//! The objects that one pod uses, as its spec names them.

use std::collections::BTreeSet;

use k8s_openapi::api::core::v1::{EnvFromSource, EnvVar, Pod, Volume};

use super::Target;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// Collects the references of one pod. Each object is in the namespace of the pod, but
/// the Node.
struct Found<'a> {
    namespace: &'a str,
    targets: BTreeSet<Target>,
}

impl Found<'_> {
    fn add(&mut self, kind: Kind, name: &str) {
        let namespace = if kind == Kind::Node { "" } else { self.namespace };
        if !name.is_empty() {
            self.targets.insert(Target { kind, key: ObjectKey::new(namespace, name) });
        }
    }

    fn volume(&mut self, volume: &Volume) {
        if let Some(config_map) = &volume.config_map {
            self.add(Kind::ConfigMap, &config_map.name);
        }
        if let Some(name) = volume.secret.as_ref().and_then(|secret| secret.secret_name.as_deref()) {
            self.add(Kind::Secret, name);
        }
        for source in volume.projected.iter().flat_map(|projected| projected.sources.iter().flatten()) {
            if let Some(config_map) = &source.config_map {
                self.add(Kind::ConfigMap, &config_map.name);
            }
            if let Some(secret) = &source.secret {
                self.add(Kind::Secret, &secret.name);
            }
        }
    }

    /// The environment of one container: single values, and all keys of an object.
    fn environment(&mut self, values: Option<&Vec<EnvVar>>, sets: Option<&Vec<EnvFromSource>>) {
        for source in values.into_iter().flatten().filter_map(|value| value.value_from.as_ref()) {
            if let Some(key) = &source.config_map_key_ref {
                self.add(Kind::ConfigMap, &key.name);
            }
            if let Some(key) = &source.secret_key_ref {
                self.add(Kind::Secret, &key.name);
            }
        }
        for set in sets.into_iter().flatten() {
            if let Some(config_map) = &set.config_map_ref {
                self.add(Kind::ConfigMap, &config_map.name);
            }
            if let Some(secret) = &set.secret_ref {
                self.add(Kind::Secret, &secret.name);
            }
        }
    }
}

/// The ConfigMaps, the Secrets, the ServiceAccount and the Node of one pod: its volumes
/// with their projected sources, the environment of each container, its image pull
/// Secrets, its account and the node that runs it. A detail view of a pod links to them.
///
/// A Secret that only a volume driver knows, for example of a CSI volume, is not in the
/// result: the storage kinds are out of the scope of the app.
pub fn references_of(pod: &Pod) -> BTreeSet<Target> {
    let mut found =
        Found { namespace: pod.metadata.namespace.as_deref().unwrap_or_default(), targets: BTreeSet::new() };
    let Some(spec) = pod.spec.as_ref() else {
        return found.targets;
    };
    found.add(Kind::Node, spec.node_name.as_deref().unwrap_or_default());
    found.add(Kind::ServiceAccount, spec.service_account_name.as_deref().unwrap_or_default());
    for secret in spec.image_pull_secrets.iter().flatten() {
        found.add(Kind::Secret, &secret.name);
    }
    spec.volumes.iter().flatten().for_each(|volume| found.volume(volume));
    for container in spec.containers.iter().chain(spec.init_containers.iter().flatten()) {
        found.environment(container.env.as_ref(), container.env_from.as_ref());
    }
    for container in spec.ephemeral_containers.iter().flatten() {
        found.environment(container.env.as_ref(), container.env_from.as_ref());
    }
    found.targets
}
