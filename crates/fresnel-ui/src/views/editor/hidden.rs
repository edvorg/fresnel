//! The values of a Secret in the diff of its save dialog: the editor shows them decoded, so
//! the dialog hides them until the user shows them, as the save dialog of the keys does (plan
//! section 11.2). A hidden row keeps the key of its value: the user still sees which keys
//! change. The copy of `kubectl apply` in the annotations holds each value too, so it hides.
//!
//! Both texts of the diff come from the YAML writer of the core, in block style: a field at
//! the start of a line, and each line of its value indented more. So the lines of a value are
//! the line of its field and the lines below it with more indent.

/// What a hidden value shows.
const DOTS: &str = "••••••••";
/// The fields of a Secret whose values the editor shows decoded.
const FIELDS: [&str; 2] = ["data", "stringData"];
/// The path of the copy of `kubectl apply`: the JSON of the object with each value.
const LAST_APPLIED: [&str; 3] = ["metadata", "annotations", "kubectl.kubernetes.io/last-applied-configuration"];

/// The text of each line with its value hidden, for a line of a value; `None` for each other
/// line. `lines` have no line feed.
pub(super) fn hidden(lines: &[&str]) -> Vec<Option<String>> {
    // The fields above the line, each with its indent.
    let mut path = Vec::new();
    lines.iter().map(|line| hide(line, &mut path)).collect()
}

/// The text of `line` with its value hidden, for a line of a value. `path` holds the fields
/// above it, and gets the field of the line.
fn hide<'a>(line: &'a str, path: &mut Vec<(usize, &'a str)>) -> Option<String> {
    let content = line.trim_start_matches(' ');
    let indent = line.len() - content.len();
    if content.is_empty() {
        return None;
    }
    while path.last().is_some_and(|(at, _)| *at >= indent) {
        path.pop();
    }
    let spaces = " ".repeat(indent);
    // A line of a value of more than one line.
    if (1..=path.len()).any(|length| is_value(path.get(..length).unwrap_or_default())) {
        return Some(format!("{spaces}{DOTS}"));
    }
    let (name, rest) = content.split_once(':')?;
    path.push((indent, name));
    // A value in the line of its field, or the values of `data` in that line, as `{}`.
    let inline = matches!(path.as_slice(), [(_, field)] if FIELDS.contains(field)) && !rest.trim().is_empty();
    (is_value(path) || inline).then(|| format!("{spaces}{name}: {DOTS}"))
}

/// True for the path of a field whose value hides: a key of `data` or `stringData`, or the
/// copy of `kubectl apply`.
fn is_value(path: &[(usize, &str)]) -> bool {
    match path {
        [(_, field), _] => FIELDS.contains(field),
        _ => path.iter().map(|(_, name)| *name).eq(LAST_APPLIED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_values_of_data_and_string_data_hide_and_their_keys_and_the_other_fields_show() {
        let lines = [
            "apiVersion: v1",
            "data:",
            "  password: hunter2",
            "  raw: !!binary /wA=",
            "  tls.crt: |",
            "    -----BEGIN CERTIFICATE-----",
            "    user: admin",
            "kind: Secret",
            "metadata:",
            "  name: db",
            "stringData: {}",
            "type: Opaque",
        ];
        let shown: Vec<Option<&str>> = vec![
            None,
            None,
            Some("  password: ••••••••"),
            Some("  raw: ••••••••"),
            Some("  tls.crt: ••••••••"),
            Some("    ••••••••"),
            Some("    ••••••••"),
            None,
            None,
            None,
            Some("stringData: ••••••••"),
            None,
        ];
        assert_eq!(hidden(&lines).iter().map(Option::as_deref).collect::<Vec<_>>(), shown);
    }

    /// `kubectl apply` keeps a copy of the object with each value in an annotation, also of
    /// `stringData` as plain text, in one line or in more.
    #[test]
    fn the_copy_of_kubectl_apply_hides_and_the_other_annotations_show() {
        let copy = r#"{"apiVersion":"v1","kind":"Secret","stringData":{"password":"hunter2"}}"#;
        let one_line = format!("    kubectl.kubernetes.io/last-applied-configuration: '{copy}'");
        let lines = [
            "metadata:",
            "  annotations:",
            "    kubectl.kubernetes.io/last-applied-configuration: |",
            &format!("      {copy}"),
            "    team: shop",
            "  labels:",
            "    kubectl.kubernetes.io/last-applied-configuration: no copy",
            "  name: db",
        ];
        let shown: Vec<Option<&str>> = vec![
            None,
            None,
            Some("    kubectl.kubernetes.io/last-applied-configuration: ••••••••"),
            Some("      ••••••••"),
            None,
            None,
            None,
            None,
        ];
        assert_eq!(hidden(&lines).iter().map(Option::as_deref).collect::<Vec<_>>(), shown);
        let lines = ["metadata:", "  annotations:", &one_line];
        let copy = "    kubectl.kubernetes.io/last-applied-configuration: ••••••••".to_owned();
        assert_eq!(hidden(&lines), [None, None, Some(copy)], "a copy in one line");
    }
}
