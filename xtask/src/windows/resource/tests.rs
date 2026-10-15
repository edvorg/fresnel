//! A reader of the resource file reads each entry and the tree of the version back, as the
//! linker and `VerQueryValue` of Windows read them.

use super::*;

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The entries of a resource file after its empty one: type, name and data. Each header must
/// have the size 32 and each entry must start at a 4-byte border.
fn entries(file: &[u8]) -> Vec<(u16, u16, Vec<u8>)> {
    assert_eq!(file[..32], EMPTY_ENTRY, "a 32-bit resource file starts with an empty entry");
    let (mut entries, mut at) = (Vec::new(), 32);
    while at < file.len() {
        assert_eq!(at % 4, 0, "an entry at {at}");
        let (size, header) = (u32_at(file, at) as usize, u32_at(file, at + 4) as usize);
        assert_eq!((header, u16_at(file, at + 8), u16_at(file, at + 12)), (32, 0xFFFF, 0xFFFF));
        assert_eq!((u16_at(file, at + 22), u16_at(file, at + 20)), (LANGUAGE, MEMORY_FLAGS));
        let data = file[at + header..at + header + size].to_vec();
        entries.push((u16_at(file, at + 10), u16_at(file, at + 14), data));
        at = (at + header + size).next_multiple_of(4);
    }
    assert_eq!(at, file.len(), "the last entry ends at the end of the file");
    entries
}

/// A node of the version: key, type, value and children, read by their lengths alone.
struct Node {
    key: String,
    kind: u16,
    value: Vec<u8>,
    children: Vec<Node>,
}

fn text_at(bytes: &[u8], at: usize) -> (String, usize) {
    let units: Vec<u16> = (at..).step_by(2).map(|at| u16_at(bytes, at)).take_while(|unit| *unit != 0).collect();
    (String::from_utf16(&units).expect("UTF-16"), at + units.len() * 2 + 2)
}

fn read_node(bytes: &[u8]) -> Node {
    let (length, value_length, kind) = (u16_at(bytes, 0) as usize, u16_at(bytes, 2) as usize, u16_at(bytes, 4));
    assert!(length <= bytes.len(), "a node of {length} bytes in {}", bytes.len());
    let (key, at) = text_at(bytes, 6);
    let at = at.next_multiple_of(4);
    // The length of a text value counts units of 2 bytes.
    let value_end = at + if kind == 1 { value_length * 2 } else { value_length };
    let value = bytes[at..value_end].to_vec();
    let (mut children, mut at) = (Vec::new(), value_end.next_multiple_of(4));
    while at < length {
        let child = read_node(&bytes[at..length]);
        at = (at + u16_at(bytes, at) as usize).next_multiple_of(4);
        children.push(child);
    }
    Node { key, kind, value, children }
}

impl Node {
    fn child(&self, key: &str) -> &Node {
        self.children.iter().find(|child| child.key == key).unwrap_or_else(|| panic!("no {key} in {}", self.key))
    }

    fn text(&self) -> String {
        assert_eq!(self.kind, 1, "{} is a text", self.key);
        text_at(&self.value, 0).0
    }
}

#[test]
fn the_file_has_each_image_of_the_icon_its_list_and_the_version() {
    let version = Version { release: [0, 1, 0], build: Some(57) };
    let entries = entries(&file(&version).expect("the resource file"));
    let kinds: Vec<(u16, u16)> = entries.iter().map(|(kind, id, _)| (*kind, *id)).collect();
    assert_eq!(kinds, [(3, 1), (3, 2), (3, 3), (3, 4), (3, 5), (3, 6), (14, 1), (16, 1)]);

    // The list of the icon: reserved, type 1 (icon), count, then 14 bytes for each image.
    let group = &entries[6].2;
    assert_eq!((u16_at(group, 0), u16_at(group, 2), u16_at(group, 4)), (0, 1, 6));
    assert_eq!(group.len(), 6 + 6 * 14);
    for (index, size) in ICON_SIZES.into_iter().enumerate() {
        let (line, image) = (&group[6 + index * 14..6 + (index + 1) * 14], &entries[index].2);
        let side = if size == 256 { 0 } else { size as u8 };
        assert_eq!((line[0], line[1], u16_at(line, 4), u16_at(line, 6)), (side, side, 1, 32), "size {size}");
        assert_eq!((u32_at(line, 8) as usize, u16_at(line, 12)), (image.len(), index as u16 + 1), "size {size}");
        // A PNG file has its width and its height as the first fields of its first chunk.
        let field = |at: usize| u32::from_be_bytes([image[at], image[at + 1], image[at + 2], image[at + 3]]);
        assert_eq!((&image[1..4], field(16), field(20)), (&b"PNG"[..], size, size));
    }
}

#[test]
fn the_version_has_its_numbers_and_its_full_text() {
    // A text of 15 UTF-16 units with its zero: the node after it starts after 2 bytes of padding.
    let version = Version { release: [2, 10, 3], build: Some(7) };
    let entries = entries(&file(&version).expect("the resource file"));
    let data = &entries[7].2;
    let root = read_node(data);
    assert_eq!((root.key.as_str(), root.kind, u16_at(data, 0) as usize), ("VS_VERSION_INFO", 0, data.len()));

    // The signature, the version of the structure, then the file and the product version.
    let fixed: Vec<u32> = (0..13).map(|field| u32_at(&root.value, field * 4)).collect();
    assert_eq!(fixed[..6], [0xFEEF_04BD, 0x0001_0000, 2 << 16 | 10, 3 << 16 | 7, 2 << 16 | 10, 3 << 16 | 7]);
    assert_eq!((fixed[8], fixed[9]), (0x0004_0004, 1), "a program of Windows NT");

    let table = root.child("StringFileInfo").child("040904B0");
    let texts: Vec<(&str, String)> = table.children.iter().map(|text| (text.key.as_str(), text.text())).collect();
    assert_eq!(
        texts,
        [
            ("FileDescription", "Fresnel".to_owned()),
            ("FileVersion", "2.10.3+build.7".to_owned()),
            ("InternalName", "fresnel".to_owned()),
            ("OriginalFilename", "fresnel.exe".to_owned()),
            ("ProductName", "Fresnel".to_owned()),
            ("ProductVersion", "2.10.3+build.7".to_owned()),
        ]
    );
    let translation = root.child("VarFileInfo").child("Translation");
    assert_eq!((translation.kind, translation.value.as_slice()), (0, &[0x09, 0x04, 0xB0, 0x04][..]));
}

#[test]
fn a_build_outside_the_release_workflow_has_the_build_number_0() {
    let entries = entries(&file(&Version { release: [0, 1, 0], build: None }).expect("the resource file"));
    let root = read_node(&entries[7].2);
    assert_eq!((u32_at(&root.value, 8), u32_at(&root.value, 12)), (1, 0));
    assert_eq!(root.child("StringFileInfo").child("040904B0").child("ProductVersion").text(), "0.1.0");
}
