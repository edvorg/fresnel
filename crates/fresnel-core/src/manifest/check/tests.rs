use serde_json::json;

use super::*;
use crate::manifest::edit::text_of;

fn web() -> Identity {
    Identity { kind: Kind::Deployment, key: ObjectKey::new("shop", "web") }
}

/// The place, the level and the start of the text of each mark.
fn marks(text: &str) -> Vec<(usize, usize, Level, String)> {
    let check = check(text, &web());
    check.marks.into_iter().map(|mark| (mark.line, mark.column, mark.level, mark.text)).collect()
}

const DEPLOYMENT: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: shop\n";

#[test]
fn the_text_of_an_object_as_the_editor_writes_it_has_no_mark() {
    let deployment = json!({"apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "shop", "labels": {"on": "yes", "mode": "0644"}},
        "spec": {"replicas": 3, "template": {"spec": {"containers": [{"name": "web", "image": "nginx",
            "env": [{"name": "A", "value": "1_000"}, {"name": "B", "value": "no"}, {"name": "C", "value": "y"}]}]}}}});
    let text = text_of(deployment, Kind::Deployment).expect("a text");
    assert_eq!(check(&text.text, &web()), Check::default(), "the writer quotes each such text:\n{}", text.text);
}

#[test]
fn an_error_of_the_parser_is_one_mark_with_its_line_and_column() {
    let duplicate = format!("{DEPLOYMENT}  name: web\n");
    assert_eq!(marks(&duplicate), [(6, 3, Level::Error, "Duplicate mapping key: name not allowed here.".to_owned())]);
    let indent = format!("{DEPLOYMENT} labels: {{}}\n");
    let found = marks(&indent);
    assert!(matches!(found.as_slice(), [(6, 2, Level::Error, _)]), "{found:?}");
    let bracket = format!("{DEPLOYMENT}spec:\n  ports: [1, 2\n");
    assert_eq!(marks(&bracket), [(7, 10, Level::Error, "Unclosed bracket '['.".to_owned())]);
    let tab = "kind: Deployment\nmetadata:\n\tname: web\n";
    assert!(check(tab, &web()).has_error(), "a tab in the indent");
}

/// Plan section 15, M11: valid YAML is never refused by a hint.
#[test]
fn a_value_that_reads_as_a_boolean_or_a_number_is_a_hint_and_never_an_error() {
    let text = format!(
        "{DEPLOYMENT}  labels:\n    on: x\n    tier: yes\n    mode: 0644\n    count: 1_000\n    fine: '0644'\n    \
         plain: 644\n    words: yes please\n    False: Off\n"
    );
    let check = check(&text, &web());
    assert!(!check.has_error(), "{check:?}");
    let found: Vec<(usize, usize, String)> =
        check.marks.into_iter().map(|mark| (mark.line, mark.column, mark.text)).collect();
    assert_eq!(
        found,
        [
            (7, 5, "kubectl reads the key on as true. Quote it, so that each tool reads on.".to_owned()),
            (8, 11, "yes is the boolean true here, not a text. Quote it for a text.".to_owned()),
            (9, 11, "0644 is the octal number 420 here, not a text. Quote it for a text.".to_owned()),
            (10, 12, "1_000 is the number 1000 here, not a text. Quote it for a text.".to_owned()),
            (14, 12, "Off is the boolean false here, not a text. Quote it for a text.".to_owned()),
        ]
    );
}

#[test]
fn an_other_kind_name_or_namespace_is_a_hint_at_its_place() {
    let text =
        "apiVersion: apps/v1\nkind: StatefulSet\nmetadata:\n  name: api\n  namespace: shop\n  labels:\n    name: x\n";
    assert_eq!(
        marks(text),
        [
            (
                2,
                7,
                Level::Hint,
                "The editor edits the object with the kind Deployment. The server refuses an other kind.".to_owned()
            ),
            (
                4,
                9,
                Level::Hint,
                "The editor edits the object with the name web. The server refuses an other name.".to_owned()
            ),
        ],
        "a label `name` is no name of the object"
    );
}

#[test]
fn an_empty_text_and_a_text_that_is_not_a_map_are_hints() {
    let empty = "The text is empty. The text of an object is a map with its kind and metadata.";
    assert_eq!(marks(""), [(1, 1, Level::Hint, empty.to_owned())]);
    assert_eq!(marks("# only a comment\n"), [(1, 1, Level::Hint, empty.to_owned())]);
    let list = "The text of an object is a map with its kind and metadata.";
    assert_eq!(marks("- kind: Deployment\n"), [(1, 1, Level::Hint, list.to_owned())]);
}

/// The parser reads a value with the tag `!!binary` as its text, so a value that is not base64
/// is valid YAML: a hint, which refuses nothing.
#[test]
fn a_binary_value_that_is_not_base64_is_a_hint_and_base64_is_none() {
    let text = format!("{DEPLOYMENT}data:\n  good: !!binary iVBORw0KGgo=\n  bad: !!binary not*base64\n");
    let hint = "The value with the tag !!binary is not base64. A Secret takes no such value.";
    assert_eq!(marks(&text), [(8, 17, Level::Hint, hint.to_owned())]);
}

#[test]
fn a_check_keeps_500_marks_and_counts_the_others() {
    let lines: String = (0..600).map(|number| format!("  k{number}: yes\n")).collect();
    let check = check(&format!("{DEPLOYMENT}  labels:\n{lines}"), &web());
    assert_eq!((check.marks.len(), check.more), (500, 100));
    assert_eq!(check.marks.first().map(|mark| mark.line), Some(7), "the first places");
}

/// A hint never hides that the parser refuses a text: an error after more hints than the
/// limit stays in the check, and the hints give it their room.
#[test]
fn an_error_after_more_hints_than_the_limit_stays_in_the_check() {
    let lines: String = (0..600).map(|number| format!("  k{number}: yes\n")).collect();
    let check = check(&format!("{DEPLOYMENT}  labels:\n{lines}  name: api\n"), &web());
    assert!(check.has_error(), "the second name of the metadata");
    assert_eq!((check.marks.len(), check.more), (500, 102), "600 hints, the hint of the name and the error");
    let last = check.marks.last().map(|mark| (mark.line, mark.column, mark.level));
    assert_eq!(last, Some((607, 3, Level::Error)), "the marks stay in the sequence of their places");
}

/// The server drops the namespace of an object of a kind with none, and refuses no text for it.
#[test]
fn a_namespace_of_an_object_of_a_kind_with_none_is_a_hint_that_the_server_drops_it() {
    let node = Identity { kind: Kind::Node, key: ObjectKey::new("", "worker") };
    let check = check("apiVersion: v1\nkind: Node\nmetadata:\n  name: worker\n  namespace: shop\n", &node);
    let found: Vec<(usize, usize, Level, String)> =
        check.marks.into_iter().map(|mark| (mark.line, mark.column, mark.level, mark.text)).collect();
    assert_eq!(found, [(5, 14, Level::Hint, "A Node has no namespace: the server drops this one.".to_owned())]);
}

/// The column of a mark counts characters, as the editor does: a letter of two bytes is one.
#[test]
fn the_column_of_a_mark_counts_characters() {
    let text = format!("{DEPLOYMENT}  labels:\n    größe: yes\n    ü: größe: x\n");
    let found: Vec<(usize, usize, Level)> =
        marks(&text).into_iter().map(|(line, column, level, _)| (line, column, level)).collect();
    assert_eq!(found, [(7, 12, Level::Hint), (8, 13, Level::Error)], "bytes would give 14 and 16");
}

/// Plan section 15, M11: the forms of YAML that kubectl reads give no error: a document
/// start, comments, an anchor with a merge key, flow maps, a tag, plain and folded texts on
/// more lines, and a base64 value on lines of its own.
#[test]
fn the_forms_of_valid_yaml_give_no_error() {
    let text = "--- # the object\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: shop\n  \
                labels: &labels {app: web, \"tier\": front}\n  annotations:\n    mode: !!str 0644\n    note: a text \
                on\n      two lines\n    folded: >\n      one\n      line\nspec:\n  selector:\n    matchLabels:\n      \
                <<: *labels\n  data: !!binary |\n    aGVs\n    bG8=\n";
    let check = check(text, &web());
    assert!(!check.has_error() && check.marks.is_empty(), "{check:?}");
}

/// A text of documents for a create: the hints of each document, none of an identity, none
/// for an empty document, and the error of the parser in any document.
#[test]
fn each_document_of_a_text_for_a_create_gets_its_hints_and_an_error_its_place() {
    let text = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\ndata:\n  on: yes\n---\n---\napiVersion: v1\n\
                kind: Secret\nmetadata:\n  name: b\n  namespace: other\n---\n- a list\n";
    let found: Vec<(usize, Level, String)> =
        check_documents(text).marks.into_iter().map(|mark| (mark.line, mark.level, mark.text)).collect();
    let words = |line: usize, text: &str| (line, Level::Hint, text.to_owned());
    assert_eq!(
        found,
        [
            words(6, "kubectl reads the key on as true. Quote it, so that each tool reads on."),
            words(6, "yes is the boolean true here, not a text. Quote it for a text."),
            words(15, "The text of an object is a map with its kind and metadata."),
        ]
    );
    let error = check_documents("kind: a\n---\nkind: [b\n");
    assert!(error.has_error() && error.marks.iter().any(|mark| mark.line == 3), "{error:?}");
    assert_eq!(check_documents("---\n---\n").marks.len(), 1, "an empty text");
    assert!(check("kind: a\n---\nkind: b\n", &web()).has_error(), "an editor of one object takes one document");
}
