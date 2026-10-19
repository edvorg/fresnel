//! The values of a Secret in the diff of its save dialog: the editor shows them decoded, so
//! the dialog hides them until the user shows them, as the save dialog of the keys does (plan
//! section 11.2). A hidden row keeps the key of its value: the user still sees which keys
//! change.
//!
//! Both texts of the diff come from the YAML writer of the core, in block style: a field at
//! the start of a line, and each line of its value indented. So the lines of `data` and
//! `stringData` are those indented lines after the field.

/// What a hidden value shows.
const DOTS: &str = "••••••••";
/// The fields of a Secret whose values the editor shows decoded.
const FIELDS: [&str; 2] = ["data:", "stringData:"];

/// The text of each line with its value hidden, for a line of a value; `None` for each other
/// line. `lines` have no line feed.
pub(super) fn hidden(lines: &[&str]) -> Vec<Option<String>> {
    // In a field of the values: the indent of its keys, from its first line.
    let mut values: Option<Option<usize>> = None;
    let mut hide = |line: &str| {
        let content = line.trim_start_matches(' ');
        let indent = line.len() - content.len();
        if content.is_empty() {
            return None;
        }
        if indent == 0 {
            let field = FIELDS.into_iter().find_map(|field| Some((field, content.strip_prefix(field)?)));
            values = field.map(|_| None);
            // A value in the line of its field, such as `{}`.
            return field.filter(|(_, rest)| !rest.trim().is_empty()).map(|(field, _)| format!("{field} {DOTS}"));
        }
        let keys = *values.as_mut()?.get_or_insert(indent);
        let spaces = " ".repeat(indent);
        match content.split_once(':') {
            Some((key, _)) if indent == keys => Some(format!("{spaces}{key}: {DOTS}")),
            // A line of a value of more than one line.
            _ => Some(format!("{spaces}{DOTS}")),
        }
    };
    lines.iter().map(|line| hide(line)).collect()
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
}
