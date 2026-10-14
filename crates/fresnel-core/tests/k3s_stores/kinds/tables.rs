//! The tables that the server prints for kubectl (`as=Table`), compared with the columns of
//! the app, and their copy for the unit tests in `fixtures/kinds`.
//!
//! The rows and the objects of a table come from one answer, so they show one moment.

#[path = "../../common/columns.rs"]
mod columns;

use std::fmt::Write as _;

use fresnel_core::cluster::Gateway;
use fresnel_core::kinds::{self, Kind};
use http::header::ACCEPT;
use serde_json::Value;

use super::scene::{LABEL, NAMESPACE};
use crate::bed::Cluster;
use columns::{Table, compare, fixture};

/// The form of an answer that kubectl asks for.
const TABLE: &str = "application/json;as=Table;v=v1;g=meta.k8s.io";
/// The test writes the fixture only when this variable of the environment is set.
const VARIABLE: &str = "FRESNEL_WRITE_FIXTURE";

/// The table of each object of one kind, with each object in full.
async fn table(api: &Gateway, kind: Kind) -> Table {
    let path = format!("{}?includeObject=Object", kind.info().path());
    let request = http::Request::get(path).header(ACCEPT, TABLE).body(Vec::new()).expect("a request");
    api.list_page(request).await.unwrap_or_else(|error| panic!("the table of {kind:?}: {error}"))
}

/// Each column of the app that shows what a column of kubectl shows has the text of the
/// server, for each object of each kind.
pub(crate) async fn each_column_has_the_text_of_the_server(cluster: &Cluster) {
    let (mut agree, mut differ) = (0, Vec::new());
    for kind in Kind::ALL.iter().copied() {
        let table = table(&cluster.api.gateway, kind).await;
        assert!(!table.rows.is_empty(), "the cluster has an object of the kind {kind:?}");
        let (same, other) = compare(kind, &table);
        agree += same;
        differ.extend(other);
    }
    assert_eq!(differ, Vec::<String>::new(), "each column of the app has the text of the server");
    println!("{agree} cells of the {} kinds have the text of the server", Kind::ALL.len());
}

/// True for a row of the scene: an object in its namespace or with its label, and the one
/// node of the cluster.
fn of_scene(kind: Kind, object: &Value) -> bool {
    let text = |pointer: &str| object.pointer(pointer).and_then(Value::as_str);
    kind == Kind::Node
        || text("/metadata/namespace") == Some(NAMESPACE)
        || object.pointer("/metadata/labels").and_then(|labels| labels.get(LABEL.0)).and_then(Value::as_str)
            == Some(LABEL.1)
}

/// Writes the fixture when the environment asks for it: the table of each kind with the
/// rows of the scene, with no `managedFields`, and one row for each line.
pub(crate) async fn write_if_asked(cluster: &Cluster) {
    if std::env::var_os(VARIABLE).is_none() {
        return;
    }
    for kind in Kind::ALL.iter().copied() {
        let mut table = table(&cluster.api.gateway, kind).await;
        table.rows.retain(|row| of_scene(kind, &row.object));
        for row in &mut table.rows {
            if let Some(metadata) = row.object.get_mut("metadata").and_then(Value::as_object_mut) {
                metadata.remove("managedFields");
            }
        }
        let definitions = serde_json::to_string(&table.column_definitions).expect("JSON");
        let mut text = format!("{{\"columnDefinitions\":{definitions},\"rows\":[\n");
        for (number, row) in table.rows.iter().enumerate() {
            let end = if number + 1 == table.rows.len() { "" } else { "," };
            writeln!(text, "{}{end}", serde_json::to_string(row).expect("JSON")).expect("a write to a text");
        }
        text.push_str("]}\n");
        let path = fixture(kind);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("the folder of the fixture");
        std::fs::write(&path, text).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    }
    println!("the fixture of the kinds is new");
}
