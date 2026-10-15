//! `cargo xtask screenshots`: the pictures of the README.
//!
//! The tests of `crates/fresnel-ui/tests/views/showcase` draw them on a made-up world, with
//! no window and no cluster. This task runs those tests, compresses each PNG file with no
//! loss and writes it into `docs/screenshots`. The tests run in `cargo test --workspace`
//! too, so a change of a view cannot break a picture unseen; only this task changes the
//! files of the repository.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

/// The folder of the pictures, below the root of the workspace.
pub const FOLDER: &str = "docs/screenshots";
/// The most bytes of one picture and of all of them: a reader of the README loads each one.
pub const FILE_LIMIT: usize = 450_000;
pub const TOTAL_LIMIT: usize = 3_500_000;
/// The filter of the tests that draw the pictures.
const SCENES: &str = "showcase::";

/// One picture: its file name and its PNG file after the compression.
pub struct Picture {
    /// The file name, for example `pods-three-clusters.png`.
    pub name: String,
    /// The bytes of the file.
    pub file: Vec<u8>,
}

/// Draws the pictures, compresses them and writes them into the folder of the repository,
/// which then holds no other PNG file. Nothing changes there when a picture is above its
/// limit. `root` is the root of the workspace.
pub fn run(root: &Path) -> io::Result<Vec<Picture>> {
    let stage = crate::target_folder(root).join("screenshots");
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    fs::create_dir_all(&stage)?;
    render(root, &stage)?;
    let mut pictures = Vec::new();
    for name in png_files(&stage)? {
        let file = compress(&fs::read(stage.join(&name))?)?;
        pictures.push(Picture { name, file });
    }
    if pictures.is_empty() {
        return Err(io::Error::other(format!("the tests {SCENES} made no picture")));
    }
    let sizes: Vec<(&str, usize)> =
        pictures.iter().map(|picture| (picture.name.as_str(), picture.file.len())).collect();
    let problems = problems(&sizes);
    if !problems.is_empty() {
        return Err(io::Error::other(problems.join("; ")));
    }
    let folder = root.join(FOLDER);
    fs::create_dir_all(&folder)?;
    for old in png_files(&folder)?.iter().filter(|old| pictures.iter().all(|picture| picture.name != **old)) {
        fs::remove_file(folder.join(old))?;
    }
    for picture in &pictures {
        fs::write(folder.join(&picture.name), &picture.file)?;
    }
    Ok(pictures)
}

/// Runs the tests of the scenes with the cargo that runs this task. They write their
/// pictures into `stage`.
fn render(root: &Path, stage: &Path) -> io::Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut tests = Command::new(cargo);
    tests.args(["test", "--locked", "--package", "fresnel-ui", "--test", "views", "--", SCENES]);
    let status = tests.env("FRESNEL_SNAPSHOT_DIR", stage).current_dir(root).status()?;
    if !status.success() {
        return Err(io::Error::other(format!("the tests of the pictures failed ({status})")));
    }
    Ok(())
}

/// The names of the PNG files of a folder, in their sequence.
fn png_files(folder: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(folder)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.ends_with(".png") {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// A PNG file with the same pixels in fewer bytes. Level 4 of oxipng tries each filter of
/// the rows: the higher levels gave these pictures no smaller file and took five times as
/// long. The compression runs in one thread, so two runs give the same bytes.
pub fn compress(png: &[u8]) -> io::Result<Vec<u8>> {
    oxipng::optimize_from_memory(png, &oxipng::Options::from_preset(4)).map_err(io::Error::other)
}

/// One line for each picture above `FILE_LIMIT`, and one when the sum is above
/// `TOTAL_LIMIT`.
pub fn problems(sizes: &[(&str, usize)]) -> Vec<String> {
    let mut problems: Vec<String> = sizes
        .iter()
        .filter(|(_, size)| *size > FILE_LIMIT)
        .map(|(name, size)| format!("{name} has {size} bytes, the limit is {FILE_LIMIT}"))
        .collect();
    let total: usize = sizes.iter().map(|(_, size)| size).sum();
    if total > TOTAL_LIMIT {
        problems.push(format!("the pictures have {total} bytes, the limit is {TOTAL_LIMIT}"));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An opaque picture with soft edges, as a frame of the app: no palette fits it.
    fn picture(width: u32, height: u32) -> (Vec<u8>, Vec<u8>) {
        let pixel = |x: u32, y: u32| [(x * 255 / width) as u8, (y * 255 / height) as u8, ((x ^ y) % 7 * 30) as u8, 255];
        let pixels: Vec<u8> = (0..height).flat_map(|y| (0..width).flat_map(move |x| pixel(x, y))).collect();
        let mut file = Vec::new();
        let mut encoder = png::Encoder::new(&mut file, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().expect("a header");
        writer.write_image_data(&pixels).expect("the pixels");
        writer.finish().expect("the end of the file");
        (file, pixels)
    }

    /// The pixels of a PNG file as RGBA, whichever color type the file has.
    fn rgba(file: &[u8]) -> Vec<u8> {
        let mut decoder = png::Decoder::new(io::Cursor::new(file));
        decoder.set_transformations(png::Transformations::ALPHA);
        let mut reader = decoder.read_info().expect("a PNG file");
        let mut pixels = vec![0; reader.output_buffer_size().expect("a size")];
        let frame = reader.next_frame(&mut pixels).expect("the pixels");
        pixels.truncate(frame.buffer_size());
        match frame.color_type {
            png::ColorType::Rgba => pixels,
            png::ColorType::Rgb => pixels.chunks(3).flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255]).collect(),
            other => panic!("a color type that the pictures do not have: {other:?}"),
        }
    }

    #[test]
    fn a_compressed_picture_has_the_same_pixels_in_fewer_bytes_and_the_same_bytes_each_time() {
        let (file, pixels) = picture(160, 90);
        let compressed = compress(&file).expect("a compression");
        assert!(compressed.len() < file.len(), "{} bytes, before {}", compressed.len(), file.len());
        assert!(rgba(&compressed) == pixels, "other pixels");
        assert_eq!(compress(&file).expect("a compression"), compressed);
    }

    #[test]
    fn a_picture_above_its_limit_and_pictures_above_the_sum_are_problems() {
        assert_eq!(problems(&[("a.png", FILE_LIMIT), ("b.png", 1)]), Vec::<String>::new());
        let large = problems(&[("a.png", FILE_LIMIT + 1)]);
        assert_eq!(large, [format!("a.png has {} bytes, the limit is {FILE_LIMIT}", FILE_LIMIT + 1)]);
        let many: Vec<(&str, usize)> = (0..8).map(|_| ("a.png", FILE_LIMIT)).collect();
        assert_eq!(
            problems(&many),
            [format!("the pictures have {} bytes, the limit is {TOTAL_LIMIT}", 8 * FILE_LIMIT)]
        );
    }
}
