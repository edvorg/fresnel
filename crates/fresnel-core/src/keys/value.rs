//! The decode of one value (plan section 12.2, research 08 section 8.2).
//!
//! A text editor changes what it cannot show: a byte that is not UTF-8, a control
//! character. So a value goes into a text editor only when it is text, and each other
//! value shows as its size and its type.

use std::fmt;
use std::sync::Arc;

/// One value of a key: its bytes, and the form that a view shows. Its debug text has the
/// size only, never the bytes: a value of a Secret must not go to a log.
#[derive(Clone, PartialEq, Eq)]
pub struct Value {
    bytes: Arc<[u8]>,
    form: Form,
}

/// What a view shows of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// UTF-8 with no control character but tab, line feed and carriage return: a view
    /// shows it in a text editor.
    Text,
    /// Each other value: a view shows its size and its type.
    Binary(BinaryType),
}

impl Value {
    /// The value of these bytes, with its form.
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        let bytes = bytes.into();
        let form = match std::str::from_utf8(&bytes) {
            Ok(text) if !text.chars().any(is_hidden) => Form::Text,
            Ok(_) => Form::Binary(BinaryType::of(&bytes).unwrap_or(BinaryType::ControlText)),
            Err(_) => Form::Binary(BinaryType::of(&bytes).unwrap_or(BinaryType::Unknown)),
        };
        Self { bytes, form }
    }

    /// The bytes, as the server stores them.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The size in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// True for a value with no byte. It is text.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// What a view shows of the value.
    pub fn form(&self) -> Form {
        self.form
    }

    /// The text of a text value. `None` for a binary value.
    pub fn text(&self) -> Option<&str> {
        match self.form {
            Form::Text => std::str::from_utf8(&self.bytes).ok(),
            Form::Binary(_) => None,
        }
    }

    /// True for a text with no line feed at its end. A text editor can add one, and the
    /// save then changes the value: the view marks such a value, so the user sees why.
    pub fn ends_without_line_feed(&self) -> bool {
        self.form == Form::Text && self.bytes.last().is_some_and(|last| *last != b'\n')
    }

    /// True for a text with a carriage return. A text editor can change line ends, and a
    /// view marks such a value for the same reason.
    pub fn has_carriage_return(&self) -> bool {
        self.form == Form::Text && self.bytes.contains(&b'\r')
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Value").field("bytes", &self.bytes.len()).field("form", &self.form).finish()
    }
}

/// A control character that a text editor does not show. Tab, line feed and carriage return
/// are a part of each text file.
fn is_hidden(character: char) -> bool {
    character.is_control() && !matches!(character, '\t' | '\n' | '\r')
}

/// The type of a binary value, from its first bytes, as `file` tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryType {
    Gzip,
    Zip,
    Bzip2,
    Xz,
    Zstd,
    Png,
    Jpeg,
    Gif,
    Pdf,
    /// A Java keystore of the format JKS.
    JavaKeystore,
    Elf,
    /// UTF-16 with a byte order mark. It is not UTF-8.
    Utf16,
    /// One DER structure: a certificate, a key or a PKCS #12 file in binary form.
    Der,
    /// UTF-8 with a control character that a text editor does not show.
    ControlText,
    /// No known type.
    Unknown,
}

/// The first bytes of each type with a fixed start.
const MAGIC: [(&[u8], BinaryType); 14] = [
    (b"\x1f\x8b", BinaryType::Gzip),
    (b"PK\x03\x04", BinaryType::Zip),
    (b"BZh", BinaryType::Bzip2),
    (b"\xfd7zXZ\x00", BinaryType::Xz),
    (b"\x28\xb5\x2f\xfd", BinaryType::Zstd),
    (b"\x89PNG\r\n\x1a\n", BinaryType::Png),
    (b"\xff\xd8\xff", BinaryType::Jpeg),
    (b"GIF87a", BinaryType::Gif),
    (b"GIF89a", BinaryType::Gif),
    (b"%PDF-", BinaryType::Pdf),
    (b"\xfe\xed\xfe\xed", BinaryType::JavaKeystore),
    (b"\x7fELF", BinaryType::Elf),
    (b"\xff\xfe", BinaryType::Utf16),
    (b"\xfe\xff", BinaryType::Utf16),
];

impl BinaryType {
    /// The type of these bytes. `None` when no known type fits.
    fn of(bytes: &[u8]) -> Option<Self> {
        let magic = MAGIC.iter().find(|(start, _)| bytes.starts_with(start)).map(|(_, kind)| *kind);
        magic.or_else(|| is_der_sequence(bytes).then_some(Self::Der))
    }

    /// The words of a view.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gzip => "gzip data",
            Self::Zip => "ZIP archive",
            Self::Bzip2 => "bzip2 data",
            Self::Xz => "xz data",
            Self::Zstd => "zstd data",
            Self::Png => "PNG image",
            Self::Jpeg => "JPEG image",
            Self::Gif => "GIF image",
            Self::Pdf => "PDF document",
            Self::JavaKeystore => "Java keystore",
            Self::Elf => "ELF program",
            Self::Utf16 => "UTF-16 text",
            Self::Der => "DER data (certificate or key)",
            Self::ControlText => "text with control characters",
            Self::Unknown => "binary data",
        }
    }
}

/// True for bytes that are one DER sequence: the tag 0x30 and a length that ends at the last
/// byte. Random bytes fit this rule only by chance.
fn is_der_sequence(bytes: &[u8]) -> bool {
    let [0x30, first, rest @ ..] = bytes else {
        return false;
    };
    // 0x80 is the length form "until an end mark", which DER does not use.
    let (length, body) = match *first {
        0x80 => return false,
        short if short < 0x80 => (usize::from(short), rest),
        long @ 0x81..=0x84 => {
            let count = usize::from(long - 0x80);
            let Some((digits, body)) = rest.split_at_checked(count) else {
                return false;
            };
            (digits.iter().fold(0, |length, digit| (length << 8) | usize::from(*digit)), body)
        }
        _ => return false,
    };
    !body.is_empty() && body.len() == length
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(bytes: &[u8]) -> Form {
        Value::new(bytes.to_vec()).form()
    }

    #[test]
    fn utf8_with_tabs_and_line_ends_is_text_and_each_other_control_character_is_not() {
        assert_eq!(form(b"user=admin\n\tport=5432\r\n"), Form::Text);
        assert_eq!(form("Grüße, 東京 🙂".as_bytes()), Form::Text);
        assert_eq!(form(b""), Form::Text, "an empty value is an empty text");
        assert_eq!(form(b"\x1b[31mred\x1b[0m"), Form::Binary(BinaryType::ControlText));
        assert_eq!(form(b"a\0b"), Form::Binary(BinaryType::ControlText));
        assert_eq!(form("a\u{7f}".as_bytes()), Form::Binary(BinaryType::ControlText), "DEL");
        assert_eq!(form("a\u{85}b".as_bytes()), Form::Binary(BinaryType::ControlText), "a C1 control character");
        assert_eq!(form(b"caf\xe9"), Form::Binary(BinaryType::Unknown), "Latin-1 is not UTF-8");
        assert_eq!(form(b"\xc3"), Form::Binary(BinaryType::Unknown), "a cut UTF-8 sequence");

        let text = Value::new(b"one\ntwo".to_vec());
        assert_eq!((text.text(), text.len(), text.is_empty()), (Some("one\ntwo"), 7, false));
        assert_eq!(Value::new(vec![0xff]).text(), None, "a binary value has no text");
    }

    #[test]
    fn a_binary_value_gets_the_type_of_its_first_bytes() {
        let cases: [(&[u8], BinaryType); 12] = [
            (b"\x1f\x8b\x08\x00\x00\x00", BinaryType::Gzip),
            (b"PK\x03\x04\x14\x00", BinaryType::Zip),
            (b"BZh91AY&SY\xff", BinaryType::Bzip2),
            (b"\xfd7zXZ\x00\x00\x04", BinaryType::Xz),
            (b"\x28\xb5\x2f\xfd\x04\x58", BinaryType::Zstd),
            (b"\x89PNG\r\n\x1a\n\x00\x00", BinaryType::Png),
            (b"\xff\xd8\xff\xe0\x00\x10", BinaryType::Jpeg),
            (b"GIF89a\x01\x00\x01\x00\x80", BinaryType::Gif),
            (b"%PDF-1.7\n\xe2\xe3\xcf\xd3", BinaryType::Pdf),
            (b"\xfe\xed\xfe\xed\x00\x00\x00\x02", BinaryType::JavaKeystore),
            (b"\x7fELF\x02\x01\x01", BinaryType::Elf),
            (b"\xff\xfeh\x00i\x00", BinaryType::Utf16),
        ];
        for (bytes, kind) in cases {
            assert_eq!(form(bytes), Form::Binary(kind), "{kind:?}");
        }
        assert_eq!(form(b"%PDF-1.4 all text"), Form::Text, "a type with a text start that is all text stays text");
        assert_eq!(BinaryType::Unknown.label(), "binary data");
    }

    #[test]
    fn one_der_sequence_is_der_and_a_length_that_does_not_fit_is_not() {
        let long: Vec<u8> = [&[0x30, 0x82, 0x01, 0x00][..], &[0xa5; 256]].concat();
        assert_eq!(form(&long), Form::Binary(BinaryType::Der), "a long form length");
        assert_eq!(form(&[0x30, 0x03, 0x02, 0x01, 0x80]), Form::Binary(BinaryType::Der), "a short form length");
        assert_eq!(form(&long[..200]), Form::Binary(BinaryType::Unknown), "a cut structure");
        assert_eq!(form(&[0x30, 0x03, 0x02, 0x01, 0x80, 0x00]), Form::Binary(BinaryType::Unknown), "one byte more");
        assert_eq!(form(&[0x30, 0x03, 0x02, 0x01, 0x05]), Form::Binary(BinaryType::Der), "DER that is UTF-8 too");
        assert_eq!(form(&[0x30, 0x85, 1, 2, 3, 4, 5, 0x80]), Form::Binary(BinaryType::Unknown), "too long");
        assert_eq!(form(&[0x30, 0x81]), Form::Binary(BinaryType::Unknown), "no length digit");
    }

    #[test]
    fn a_text_tells_its_line_ends_and_a_debug_text_has_no_byte_of_the_value() {
        let marks = |bytes: &[u8]| {
            let value = Value::new(bytes.to_vec());
            (value.ends_without_line_feed(), value.has_carriage_return())
        };
        assert_eq!(marks(b"hunter2"), (true, false));
        assert_eq!(marks(b"line\n"), (false, false));
        assert_eq!(marks(b"line\r\n"), (false, true));
        assert_eq!(marks(b""), (false, false), "an empty text has no end to mark");
        assert_eq!(marks(b"\x00\r"), (false, false), "a binary value has no marks of a text");

        let debug = format!("{:?}", Value::new(b"hunter2".to_vec()));
        assert_eq!(debug, "Value { bytes: 7, form: Text }");
    }
}
