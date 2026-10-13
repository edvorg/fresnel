//! The fixture of the unit tests: a copy of the scene, as the API server and kubectl give
//! it. No object of a real cluster goes into the repository (plan section 14).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use fresnel_core::cluster::Gateway;
use fresnel_core::kinds::{Kind, Tier};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ListMeta;
use kube::core::Request;
use kube::core::params::ListParams;
use serde::Deserialize;
use serde_json::Value;

use super::scene;
use crate::bed::Cluster;

/// The test writes the fixture only when this variable of the environment is set.
const VARIABLE: &str = "FRESNEL_WRITE_FIXTURE";
/// The most reads of the pods for one copy in which kubectl and the API show the same pods.
const ATTEMPTS: usize = 40;

#[derive(Deserialize)]
struct Page {
    metadata: ListMeta,
    items: Option<Vec<Value>>,
}

/// Each object of one kind, as the API gives it, with no `managedFields`: no store keeps them.
async fn objects(api: &Gateway, kind: Kind) -> Vec<Value> {
    let request = Request::new(kind.info().path());
    let (mut objects, mut params) = (Vec::new(), ListParams::default().limit(500));
    loop {
        let page: Page = api.list_page(request.list(&params).expect("a request")).await.expect("a list page");
        objects.extend(page.items.unwrap_or_default());
        match page.metadata.continue_.filter(|token| !token.is_empty()) {
            Some(token) => params = params.continue_token(&token),
            None => break,
        }
    }
    for object in &mut objects {
        if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
            metadata.remove("managedFields");
        }
    }
    objects
}

/// The pods and the text of kubectl for the same moment: no pod changed between two reads
/// of the API with the read of kubectl in the middle.
async fn pods_with_kubectl(cluster: &Cluster) -> (Vec<Value>, String) {
    let api = &cluster.api.gateway;
    let mut before = objects(api, Kind::Pod).await;
    for _ in 0..ATTEMPTS {
        let text = scene::pod_lines(cluster).await.0;
        let after = objects(api, Kind::Pod).await;
        if before == after {
            return (after, text);
        }
        before = after;
    }
    panic!("the pods changed in each of {ATTEMPTS} reads");
}

/// One object for each line, so that a new copy of the scene changes few lines.
fn lines(objects: &[Value]) -> String {
    let mut text = String::from("[\n");
    for (number, object) in objects.iter().enumerate() {
        let end = if number + 1 == objects.len() { "" } else { "," };
        writeln!(text, "{object}{end}").expect("a write to a text");
    }
    text + "]\n"
}

fn folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/k3s")
}

/// Writes the fixture when the environment asks for it: one file for each tier A kind,
/// and the output of kubectl for the pods.
pub(crate) async fn write_if_asked(cluster: &Cluster) {
    if std::env::var_os(VARIABLE).is_none() {
        return;
    }
    let folder = folder();
    std::fs::create_dir_all(&folder).expect("the folder of the fixture");
    let write = |file: String, text: String| {
        std::fs::write(folder.join(&file), text).unwrap_or_else(|error| panic!("write {file}: {error}"));
    };
    let (pods, text) = pods_with_kubectl(cluster).await;
    write("pods.json".to_owned(), lines(&pods));
    write("pods.kubectl.txt".to_owned(), text);
    for kind in Kind::ALL.iter().copied().filter(|kind| kind.info().tier == Tier::A && *kind != Kind::Pod) {
        let objects = objects(&cluster.api.gateway, kind).await;
        write(format!("{}.json", kind.info().plural), lines(&objects));
    }
    println!("the fixture of the unit tests is new: {}", folder.display());
}
