//! The lines of the box drawing block, U+2500 to U+257F: light, heavy, double and dashed.
//!
//! A line reaches past the middle of its cell over the strokes across it, so a corner has
//! no notch. A double line meets a line across it at its near or its far stroke, as the
//! frames of Midnight Commander need: each stroke of `╔` turns into its own corner.

use egui::{Pos2, Rect, pos2};

use super::Pen;

/// The segments of each character: two bits each for left, right, up and down (0 none,
/// 1 light, 2 heavy, 3 double), from the Unicode name of the character. The arcs and the
/// diagonals have their own code and no segment here.
const SEGMENTS: [u8; 128] = [
    0x05, 0x0a, 0x50, 0xa0, 0x05, 0x0a, 0x50, 0xa0, 0x05, 0x0a, 0x50, 0xa0, 0x44, 0x48, 0x84, 0x88, //
    0x41, 0x42, 0x81, 0x82, 0x14, 0x18, 0x24, 0x28, 0x11, 0x12, 0x21, 0x22, 0x54, 0x58, 0x64, 0x94, //
    0xa4, 0x68, 0x98, 0xa8, 0x51, 0x52, 0x61, 0x91, 0xa1, 0x62, 0x92, 0xa2, 0x45, 0x46, 0x49, 0x4a, //
    0x85, 0x86, 0x89, 0x8a, 0x15, 0x16, 0x19, 0x1a, 0x25, 0x26, 0x29, 0x2a, 0x55, 0x56, 0x59, 0x5a, //
    0x65, 0x95, 0xa5, 0x66, 0x69, 0x96, 0x99, 0x6a, 0x9a, 0xa6, 0xa9, 0xaa, 0x05, 0x0a, 0x50, 0xa0, //
    0x0f, 0xf0, 0x4c, 0xc4, 0xcc, 0x43, 0xc1, 0xc3, 0x1c, 0x34, 0x3c, 0x13, 0x31, 0x33, 0x5c, 0xf4, //
    0xfc, 0x53, 0xf1, 0xf3, 0x4f, 0xc5, 0xcf, 0x1f, 0x35, 0x3f, 0x5f, 0xf5, 0xff, 0x44, 0x41, 0x11, //
    0x14, 0x00, 0x00, 0x00, 0x01, 0x10, 0x04, 0x40, 0x02, 0x20, 0x08, 0x80, 0x09, 0x90, 0x06, 0x60, //
];

const HEAVY: u8 = 2;
const DOUBLE: u8 = 3;

/// One half of a line: from the middle of the cell to one edge.
struct Half {
    weight: u8,
    /// -1 to the left or up, 1 to the right or down.
    sign: f32,
    horizontal: bool,
    /// The segments across it, on the side of -1 and of 1: up and down for a horizontal half.
    across: [u8; 2],
    /// The other half on its axis.
    opposite: u8,
}

pub(super) fn draw(code: u32, pen: &mut Pen<'_>) {
    let segments = SEGMENTS.get((code - 0x2500) as usize).copied().unwrap_or_default();
    let weight = |shift: u8| (segments >> shift) & 3;
    let [left, right, up, down] = [weight(0), weight(2), weight(4), weight(6)];
    let light = pen.light();
    let rect = pen.rect;
    let middle = pos2(
        pen.middle(rect.left(), rect.right(), light) + light / 2.0,
        pen.middle(rect.top(), rect.bottom(), light) + light / 2.0,
    );
    if let Some(count) = dashes(code) {
        dashed(pen, middle, (left.max(up), left > 0), count);
        return;
    }
    let halves = [
        Half { weight: left, sign: -1.0, horizontal: true, across: [up, down], opposite: right },
        Half { weight: right, sign: 1.0, horizontal: true, across: [up, down], opposite: left },
        Half { weight: up, sign: -1.0, horizontal: false, across: [left, right], opposite: down },
        Half { weight: down, sign: 1.0, horizontal: false, across: [left, right], opposite: up },
    ];
    for half in halves.iter().filter(|half| half.weight != 0) {
        for (offset, stroke, from) in strokes(half, light) {
            let along = if half.horizontal { middle.x } else { middle.y };
            let edge = match (half.horizontal, half.sign < 0.0) {
                (true, true) => rect.left(),
                (true, false) => rect.right(),
                (false, true) => rect.top(),
                (false, false) => rect.bottom(),
            };
            let start = along + half.sign * from;
            let across = if half.horizontal { middle.y } else { middle.x };
            let low = pen.on_pixel(across + offset - stroke / 2.0);
            let band = (start.min(edge), start.max(edge), low, low + stroke);
            pen.fill(oriented(band, half.horizontal));
        }
    }
}

/// The strokes of a half: the offset of each across the half, its width, and its start from
/// the middle toward the edge (negative: past the middle).
fn strokes(half: &Half, light: f32) -> Vec<(f32, f32, f32)> {
    // Half the width of the stroke of a weight; a double line has two light strokes.
    let half_width = |weight: u8| if weight == HEAVY { light } else { light / 2.0 };
    // The offset of the outer stroke of a weight from the middle.
    let spread = |weight: u8| if weight == DOUBLE { light } else { 0.0 };
    let [before, after] = half.across;
    if half.weight == DOUBLE {
        return [(-1.0, before, after), (1.0, after, before)]
            .into_iter()
            .map(|(side, near, far)| {
                let from = if near != 0 {
                    // Meet the near stroke of the line on this side.
                    spread(near) - half_width(near)
                } else if far != 0 {
                    // Turn at the far stroke of the line on the other side.
                    -(spread(far) + half_width(far))
                } else {
                    0.0
                };
                (side * light, light, from)
            })
            .collect();
    }
    let widest = before.max(after);
    let from = if widest == DOUBLE && half.opposite == 0 {
        // A single line that ends at a double one meets its near stroke, not the gap.
        light / 2.0
    } else {
        -(spread(widest) + half_width(widest)).max(0.0)
    };
    vec![(0.0, 2.0 * half_width(half.weight), from)]
}

/// A rectangle from a band along an axis: its start, its end, and its two edges across.
fn oriented((start, end, low, high): (f32, f32, f32, f32), horizontal: bool) -> Rect {
    if horizontal {
        Rect::from_min_max(pos2(start, low), pos2(end, high))
    } else {
        Rect::from_min_max(pos2(low, start), pos2(high, end))
    }
}

/// The count of dashes of a dashed line.
fn dashes(code: u32) -> Option<u32> {
    match code {
        0x2504..=0x2507 => Some(3),
        0x2508..=0x250b => Some(4),
        0x254c..=0x254f => Some(2),
        _ => None,
    }
}

/// A dashed line over the full cell: `count` dashes with gaps between them.
fn dashed(pen: &mut Pen<'_>, middle: Pos2, (weight, horizontal): (u8, bool), count: u32) {
    let rect = pen.rect;
    let stroke = if weight == HEAVY { 2.0 * pen.light() } else { pen.light() };
    let (from, to) = if horizontal { (rect.left(), rect.right()) } else { (rect.top(), rect.bottom()) };
    let across = if horizontal { middle.y } else { middle.x };
    let low = pen.on_pixel(across - stroke / 2.0);
    let part = (to - from) / count as f32;
    for index in 0..count {
        let start = from + index as f32 * part;
        let band = (pen.on_pixel(start + part * 0.15), pen.on_pixel(start + part * 0.85), low, low + stroke);
        pen.fill(oriented(band, horizontal));
    }
}
