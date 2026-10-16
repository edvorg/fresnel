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

#[test]
fn a_binary_value_that_is_not_base64_is_an_error_and_base64_is_none() {
    let text = format!("{DEPLOYMENT}data:\n  good: !!binary iVBORw0KGgo=\n  bad: !!binary not*base64\n");
    assert_eq!(marks(&text), [(8, 17, Level::Error, "The value with the tag !!binary is not base64.".to_owned())]);
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
