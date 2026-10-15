//! The icon of the app: a Fresnel lens from the front, drawn by this code.
//!
//! The repository holds no image file. The icon is some rings on a dark tile, so a function
//! draws it at each size that macOS and Windows ask for, with smooth edges at each one.

use std::io;
use std::path::Path;

/// The sizes of an icon set for `iconutil`: the name of the file and its width in pixels.
const ICON_SET: [(&str, u32); 10] = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

/// The tile of a macOS icon: its half width and its corner radius, as parts of the canvas.
const TILE_HALF: f64 = 0.402;
const TILE_CORNER: f64 = 0.181;
/// The colors of the tile from top to bottom, and the color of the rings.
const TILE_TOP: [f64; 3] = [0.165, 0.188, 0.239];
const TILE_BOTTOM: [f64; 3] = [0.067, 0.075, 0.094];
const RING: [f64; 3] = [0.345, 0.651, 1.0];
/// The count of rings, the radius of the last ring and the width of a ring line.
const RINGS: u32 = 6;
const LAST_RING: f64 = 0.33;
const LINE: f64 = 0.02;
const CENTER_DOT: f64 = 0.045;

/// Writes the PNG file of each size into the folder of an icon set.
pub(crate) fn write_icon_set(folder: &Path) -> io::Result<()> {
    for (file, size) in ICON_SET {
        std::fs::write(folder.join(file), png(size)?)?;
    }
    Ok(())
}

/// The icon as a PNG file with a width of `size` pixels.
pub(crate) fn png(size: u32) -> io::Result<Vec<u8>> {
    let mut file = Vec::new();
    let mut encoder = png::Encoder::new(&mut file, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(&rgba(size)).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)?;
    Ok(file)
}

/// The pixels of the icon, 4 bytes for each pixel: red, green, blue and alpha.
fn rgba(size: u32) -> Vec<u8> {
    let scale = f64::from(size);
    let mut pixels = Vec::with_capacity(size as usize * size as usize * 4);
    for row in 0..size {
        for column in 0..size {
            // The center of the pixel, with the center of the canvas at (0, 0).
            let (x, y) = ((f64::from(column) + 0.5) / scale - 0.5, (f64::from(row) + 0.5) / scale - 0.5);
            let (color, alpha) = pixel(x, y, scale);
            pixels.extend(color.map(byte));
            pixels.push(byte(alpha));
        }
    }
    pixels
}

fn byte(value: f64) -> u8 {
    // The value is in 0.0 to 1.0, so the product fits.
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The color and the alpha of one point. `scale` is the width of the canvas in pixels;
/// it makes each edge one pixel soft.
fn pixel(x: f64, y: f64, scale: f64) -> ([f64; 3], f64) {
    let cover = |distance: f64| (0.5 - distance * scale).clamp(0.0, 1.0);
    let mut color = mix(TILE_TOP, TILE_BOTTOM, (y / TILE_HALF + 1.0) / 2.0);
    let from_center = x.hypot(y);
    for ring in 1..=RINGS {
        // The rings of a Fresnel lens come closer to each other toward the edge.
        let radius = LAST_RING * (f64::from(ring) / f64::from(RINGS)).sqrt();
        let strength = 1.0 - 0.11 * f64::from(ring - 1);
        color = mix(color, RING, cover((from_center - radius).abs() - LINE / 2.0) * strength);
    }
    color = mix(color, RING, cover(from_center - CENTER_DOT));
    (color, cover(tile_distance(x, y)))
}

/// The distance from a point to the edge of the tile. It is negative inside the tile.
fn tile_distance(x: f64, y: f64) -> f64 {
    let inner = TILE_HALF - TILE_CORNER;
    let (beyond_x, beyond_y) = (x.abs() - inner, y.abs() - inner);
    let outside = beyond_x.max(0.0).hypot(beyond_y.max(0.0));
    outside + beyond_x.max(beyond_y).min(0.0) - TILE_CORNER
}

fn mix(from: [f64; 3], to: [f64; 3], part: f64) -> [f64; 3] {
    let part = part.clamp(0.0, 1.0);
    [0, 1, 2].map(|channel| from[channel] + (to[channel] - from[channel]) * part)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(pixels: &[u8], size: u32, column: u32, row: u32) -> [u8; 4] {
        let start = (row * size + column) as usize * 4;
        [pixels[start], pixels[start + 1], pixels[start + 2], pixels[start + 3]]
    }

    #[test]
    fn the_icon_is_a_tile_with_clear_corners_and_a_blue_center() {
        for size in [32, 64, 1024] {
            let pixels = rgba(size);
            assert_eq!(pixels.len(), (size * size * 4) as usize);
            let last = size - 1;
            for (column, row) in [(0, 0), (last, 0), (0, last), (last, last)] {
                assert_eq!(at(&pixels, size, column, row)[3], 0, "the corner {column}, {row} at size {size}");
            }
            let [red, green, blue, alpha] = at(&pixels, size, size / 2, size / 2);
            assert!(alpha == 255 && blue > 200 && blue > red && blue > green, "the center at size {size}");
            // The tile is dark between the center dot and the first ring.
            let [red, _, blue, alpha] = at(&pixels, size, size / 2, size / 2 + size / 11);
            assert!(alpha == 255 && red < 60 && blue < 80, "the tile at size {size}: {red} {blue}");
        }
    }

    #[test]
    fn the_icon_is_the_same_from_the_left_and_from_the_right() {
        let size = 128;
        let pixels = rgba(size);
        for row in 0..size {
            for column in 0..size / 2 {
                assert_eq!(at(&pixels, size, column, row), at(&pixels, size, size - 1 - column, row));
            }
        }
    }

    #[test]
    fn each_size_of_the_icon_set_is_a_png_file_of_that_size() {
        for (file, size) in ICON_SET {
            let png = png(size).expect("a PNG file");
            assert_eq!(png[..8], [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a], "{file}");
            // The width and the height are the first two fields of the first chunk.
            let field = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
            assert_eq!((field(16), field(20)), (size, size), "{file}");
        }
    }
}
