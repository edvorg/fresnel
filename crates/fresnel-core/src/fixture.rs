//! The fixtures of the unit tests: copies of two scenes on k3s.
//!
//! The cluster test `tests/k3s_stores/indexes` creates the scene of the indexes and writes
//! the files of `fixtures/k3s`: each object of each tier A kind as the API gave it, and the
//! output of `kubectl get pods` for the same moment. The cluster test `tests/k3s_stores/kinds`
//! writes `fixtures/kinds`: the tables of the server for an object of each kind. No object
//! of a real cluster is in the repository.

use fresnel_testkit::PodLine;
use serde::Serialize;
use serde_json::{Value, json};

use crate::kinds::{Describe, Kind, Object};

/// The namespace of the scene.
pub(crate) const SCENE: &str = "scene";

macro_rules! file {
    ($name:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/k3s/", $name))
    };
}

fn text(kind: Kind) -> &'static str {
    match kind {
        Kind::Namespace => file!("namespaces.json"),
        Kind::Node => file!("nodes.json"),
        Kind::Pod => file!("pods.json"),
        Kind::Deployment => file!("deployments.json"),
        Kind::StatefulSet => file!("statefulsets.json"),
        Kind::DaemonSet => file!("daemonsets.json"),
        Kind::ReplicaSet => file!("replicasets.json"),
        Kind::Job => file!("jobs.json"),
        Kind::CronJob => file!("cronjobs.json"),
        Kind::Service => file!("services.json"),
        Kind::EndpointSlice => file!("endpointslices.json"),
        Kind::Event => file!("events.json"),
        other => panic!("the fixture has no objects of the kind {other:?}"),
    }
}

/// Each object of one kind, in the sequence of the API: by namespace and name.
pub(crate) fn all<K: Describe>() -> Vec<K> {
    serde_json::from_str(text(K::ID)).expect("a file of the fixture")
}

/// The object of the scene with this name, or with this start of its name when the
/// name ends with `-`: the cluster gives the pod of a workload the rest of its name.
pub(crate) fn one<K: Describe>(name: &str) -> K {
    let fits = |object: &K| {
        let meta = object.meta();
        let known = meta.name.as_deref().unwrap_or_default();
        let in_scene = matches!(meta.namespace.as_deref(), None | Some(SCENE));
        in_scene && if name.ends_with('-') { known.starts_with(name) } else { known == name }
    };
    all::<K>().into_iter().find(fits).unwrap_or_else(|| panic!("the scene has no {:?} {name}", K::ID))
}

/// An object of the scene after a change of its JSON, for a state that the scene does not
/// hold: one that passes in a moment, or one that needs a fault of the cluster.
pub(crate) fn changed<K: Describe + Serialize>(name: &str, change: impl FnOnce(&mut Value)) -> K {
    let mut object = serde_json::to_value(one::<K>(name)).expect("JSON");
    change(&mut object);
    serde_json::from_value(object).expect("an object of the kind")
}

/// The object of the scene of the kinds test with this name, after a change of its JSON.
pub(crate) fn of_kinds<K: Describe>(name: &str, change: impl FnOnce(&mut Value)) -> K {
    let path = format!("{}/fixtures/kinds/{}.json", env!("CARGO_MANIFEST_DIR"), K::ID.info().plural);
    let table: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("a file")).expect("JSON");
    let rows = table["rows"].as_array().expect("the rows of a table");
    let fits = |object: &&Value| object.pointer("/metadata/name").and_then(Value::as_str) == Some(name);
    let mut object = rows.iter().map(|row| &row["object"]).find(fits).expect("an object of the scene").clone();
    change(&mut object);
    serde_json::from_value(object).expect("an object of the kind")
}

/// The condition with this name in the JSON of an object. A new one has the status `True`.
pub(crate) fn condition<'a>(object: &'a mut Value, name: &str) -> &'a mut Value {
    let status = object.as_object_mut().expect("an object").entry("status").or_insert_with(|| json!({}));
    let conditions = status.as_object_mut().expect("a status").entry("conditions").or_insert_with(|| json!([]));
    let conditions = conditions.as_array_mut().expect("a list of conditions");
    let place = conditions.iter().position(|condition| condition["type"] == name).unwrap_or_else(|| {
        conditions.push(json!({ "type": name, "status": "True" }));
        conditions.len() - 1
    });
    &mut conditions[place]
}

/// What kubectl showed for each pod of the fixture.
pub(crate) fn pod_lines() -> Vec<PodLine> {
    PodLine::parse_all(file!("pods.kubectl.txt")).expect("the output of kubectl")
}
