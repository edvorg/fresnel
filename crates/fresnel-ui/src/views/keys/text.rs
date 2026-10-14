//! The texts of one value in the key view: its size, its marks, a short view of a binary
//! value, and the text that "Copy" gives. A text of this module that a note shows names a
//! value by its key and its size, never by its bytes.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_core::keys::{Form, Value};

use crate::format;
use crate::widgets::tone::Tone;

/// The most bytes of a binary value that its hex view shows.
const HEX_BYTES: usize = 16;

/// "0 bytes", "1 byte", "980 bytes", "1.2 KiB", "3.4 MiB": the size of a value.
pub(super) fn size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if value < KIB {
        format::count(bytes as u64, "byte")
    } else if value < KIB * KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{:.1} MiB", value / KIB / KIB)
    }
}

/// The count of lines of a text: a last line with no line feed counts too.
pub(super) fn lines(text: &str) -> usize {
    text.split_inclusive('\n').count()
}

/// The marks of a value, each with its tone: the type and the size of a binary value, and
/// the line ends of a text that a text editor can change (research 08 section 8.3).
pub(super) fn marks(value: &Value) -> Vec<(Tone, String)> {
    let mut marks = Vec::new();
    if let Form::Binary(kind) = value.form() {
        marks.push((Tone::Warn, format!("binary · {} · {}", kind.label(), size(value.len()))));
    }
    if value.ends_without_line_feed() {
        marks.push((Tone::Muted, "no line feed at the end".to_owned()));
    }
    if value.has_carriage_return() {
        marks.push((Tone::Muted, "carriage returns".to_owned()));
    }
    marks
}

/// The first bytes of a binary value as hex: "89 50 4e 47 0d 0a 1a 0a …".
pub(super) fn hex(bytes: &[u8]) -> String {
    let shown: Vec<String> = bytes.iter().take(HEX_BYTES).map(|byte| format!("{byte:02x}")).collect();
    let more = if bytes.len() > HEX_BYTES { " …" } else { "" };
    format!("{}{more}", shown.join(" "))
}

/// What "Copy" puts on the clipboard: the text of a text value, and the base64 of a
/// binary value, which no text field can hold as it is.
pub(super) fn copied(value: &Value) -> String {
    match value.text() {
        Some(text) => text.to_owned(),
        None => STANDARD.encode(value.bytes()),
    }
}

/// The words of a copy for a note: what went to the clipboard, with no byte of it.
pub(super) fn copy_note(key: &str, value: &Value) -> String {
    match value.form() {
        Form::Text => format!("Copied the value of `{key}`."),
        Form::Binary(_) => format!("Copied the value of `{key}` as base64."),
    }
}

#[cfg(test)]
mod tests {
    use fresnel_core::keys::BinaryType;

    use super::*;

    #[test]
    fn a_size_has_its_unit_and_a_text_counts_its_last_line_with_no_line_feed() {
        let sizes: Vec<String> = [0, 1, 980, 1536, 3 * 1024 * 1024 + 400 * 1024].map(size).into();
        assert_eq!(sizes, ["0 bytes", "1 byte", "980 bytes", "1.5 KiB", "3.4 MiB"]);
        assert_eq!((lines(""), lines("one"), lines("one\n"), lines("one\ntwo")), (0, 1, 1, 2));
    }

    #[test]
    fn a_value_has_the_marks_of_its_form_and_of_its_line_ends() {
        let texts = |bytes: &[u8]| -> Vec<String> {
            marks(&Value::new(bytes.to_vec())).into_iter().map(|(_, text)| text).collect()
        };
        assert_eq!(texts(b"line\n"), Vec::<String>::new());
        assert_eq!(texts(b"a\r\nb"), ["no line feed at the end", "carriage returns"]);
        let png = b"\x89PNG\r\n\x1a\n\x00\x00";
        assert_eq!(texts(png), [format!("binary · {} · 10 bytes", BinaryType::Png.label())]);
    }

    #[test]
    fn a_binary_value_shows_its_first_bytes_and_copies_as_base64() {
        let png = Value::new(b"\x89PNG\r\n\x1a\n\x00\x01\x02\x03\x04\x05\x06\x07\x08".to_vec());
        assert_eq!(hex(png.bytes()), "89 50 4e 47 0d 0a 1a 0a 00 01 02 03 04 05 06 07 …");
        assert_eq!(hex(&[0xff]), "ff");
        assert_eq!(copied(&png), "iVBORw0KGgoAAQIDBAUGBwg=");
        assert_eq!(copy_note("logo", &png), "Copied the value of `logo` as base64.");
        let text = Value::new(b"made-up".to_vec());
        assert_eq!((copied(&text).as_str(), copy_note("a", &text).as_str()), ("made-up", "Copied the value of `a`."));
    }
}
