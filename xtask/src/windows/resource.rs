//! The resource file (`.res`) of the Windows program: its icon and its version.
//!
//! The linker of Windows takes a compiled resource file as one more input, so the program needs
//! no resource compiler, no build script and no crate for this. The format is the 32-bit resource
//! file of the Win32 SDK: an empty entry, then entries of a header and data at 4-byte borders.

use std::io;

use crate::icon;
use crate::version::Version;

/// The types of the entries: one image of the icon, the icon with the list of its images, and
/// the version (`RT_ICON`, `RT_GROUP_ICON`, `RT_VERSION`).
const ICON_IMAGE: u16 = 3;
const ICON: u16 = 14;
const VERSION: u16 = 16;
/// The sizes of the icon in pixels: Explorer, the taskbar and the title bar each take one. Each
/// image is a PNG file, which Windows reads since Vista.
const ICON_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];
/// English (United States), and Unicode for the texts of the version (the code page 1200).
const LANGUAGE: u16 = 0x0409;
const CODE_PAGE: u16 = 1200;
/// "Moveable, pure, discardable", as from the resource compiler. Windows NT reads none of them.
const MEMORY_FLAGS: u16 = 0x1030;
/// The first entry of a 32-bit resource file: empty, with type 0 and name 0. A 16-bit resource
/// file has no such entry, so the linker tells the two formats by it.
const EMPTY_ENTRY: [u8; 32] =
    [0, 0, 0, 0, 32, 0, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
/// The texts of the version that Explorer shows; Task Manager names the process by the description.
const NAME: &str = "Fresnel";
const FILE: &str = "fresnel.exe";

/// The resource file of the program of `version`.
pub(super) fn file(version: &Version) -> io::Result<Vec<u8>> {
    let mut file = EMPTY_ENTRY.to_vec();
    // The list of the icon: 0, the type 1 (an icon) and the count of its images; then for each
    // image its width, height, colors, 0, planes, bits for each pixel, size and the name of its entry.
    let mut group = Vec::new();
    for value in [0, 1, ICON_SIZES.len()] {
        put16(&mut group, fit(value)?);
    }
    for (id, size) in (1..).zip(ICON_SIZES) {
        let image = icon::png(size)?;
        // One byte for the width and for the height, where 0 means 256.
        let side = u8::try_from(size).unwrap_or(0);
        group.extend_from_slice(&[side, side, 0, 0]);
        put16(&mut group, 1);
        put16(&mut group, 32);
        put32(&mut group, fit(image.len())?);
        put16(&mut group, id);
        entry(&mut file, ICON_IMAGE, id, &image)?;
    }
    entry(&mut file, ICON, 1, &group)?;
    entry(&mut file, VERSION, 1, &version_info(version)?)?;
    Ok(file)
}

/// Adds one entry with a number as its type and its name, and its data up to a 4-byte border.
fn entry(file: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) -> io::Result<()> {
    put32(file, fit(data.len())?);
    put32(file, 32);
    for value in [0xFFFF, kind, 0xFFFF, id] {
        put16(file, value);
    }
    put32(file, 0);
    put16(file, MEMORY_FLAGS);
    put16(file, LANGUAGE);
    put32(file, 0);
    put32(file, 0);
    file.extend_from_slice(data);
    pad(file);
    Ok(())
}

/// The version (`VS_VERSIONINFO`): the numbers, which Explorer shows as the file version, and
/// the texts, among them the full text of the version as the product version.
fn version_info(version: &Version) -> io::Result<Vec<u8>> {
    let text = version.to_string();
    let texts = [
        ("FileDescription", NAME),
        ("FileVersion", text.as_str()),
        ("InternalName", "fresnel"),
        ("OriginalFilename", FILE),
        ("ProductName", NAME),
        ("ProductVersion", text.as_str()),
    ];
    let texts =
        texts.into_iter().map(|(key, value)| node(key, Value::Text(value), &[])).collect::<io::Result<Vec<_>>>()?;
    let table = node(&format!("{LANGUAGE:04X}{CODE_PAGE:04X}"), Value::None, &texts)?;
    let translation = [LANGUAGE.to_le_bytes(), CODE_PAGE.to_le_bytes()].concat();
    let translation = node("Translation", Value::Binary(&translation), &[])?;
    let children = [node("StringFileInfo", Value::None, &[table])?, node("VarFileInfo", Value::None, &[translation])?];
    node("VS_VERSION_INFO", Value::Binary(&fixed_info(version)), &children)
}

/// The numbers of the version (`VS_FIXEDFILEINFO`) of a program (`VFT_APP`) for 32-bit and
/// 64-bit Windows (`VOS_NT_WINDOWS32`).
fn fixed_info(version: &Version) -> Vec<u8> {
    let [major, minor, patch, build] = version.numbers().map(u32::from);
    let (most, least) = ((major << 16) | minor, (patch << 16) | build);
    let fields = [0xFEEF_04BD, 0x0001_0000, most, least, most, least, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0];
    fields.iter().flat_map(|field: &u32| field.to_le_bytes()).collect()
}

/// The value of a node of the version.
enum Value<'a> {
    None,
    Binary(&'a [u8]),
    Text(&'a str),
}

/// A node of the version: its length, the length of its value, its type (0 binary, 1 text),
/// its key, its value and its children. The value and each child start at a 4-byte border.
fn node(key: &str, value: Value<'_>, children: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    let mut node = vec![0; 6];
    put_text(&mut node, key);
    pad(&mut node);
    let (value_length, kind) = match value {
        Value::None => (0, 1),
        Value::Binary(bytes) => {
            node.extend_from_slice(bytes);
            (bytes.len(), 0)
        }
        // The length of a text counts its UTF-16 units with the zero at its end.
        Value::Text(text) => (put_text(&mut node, text), 1),
    };
    for child in children {
        pad(&mut node);
        node.extend_from_slice(child);
    }
    let head = [fit(node.len())?, fit(value_length)?, kind].map(u16::to_le_bytes).concat();
    node.splice(..6, head);
    Ok(node)
}

/// A count as a field of 16 or 32 bits; a count that does not fit is an error, never a cut.
fn fit<T: TryFrom<usize>>(count: usize) -> io::Result<T> {
    T::try_from(count).map_err(|_| io::Error::other(format!("{count} does not fit into a field of the resource file")))
}

/// Puts a text in UTF-16 with a zero at its end. Returns the count of its units.
fn put_text(bytes: &mut Vec<u8>, text: &str) -> usize {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    units.iter().for_each(|unit| put16(bytes, *unit));
    units.len()
}

fn put16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Fills with zeros up to the next 4-byte border.
fn pad(bytes: &mut Vec<u8>) {
    bytes.resize(bytes.len().next_multiple_of(4), 0);
}

#[cfg(test)]
mod tests;
