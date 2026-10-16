//! Box drawing, block elements and braille as shapes, not glyphs of a font (plan section
//! 12.8). A glyph of the font was small in the spike and left a gap between two rows at one
//! pixel for each point; a shape fills its cell to the edge, so the lines of `htop` and the
//! graphs of `btop` join.

mod lines;

use egui::epaint::QuadraticBezierShape;
use egui::{Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};

/// The pen of one cell: its place, its color and the size of a pixel.
pub(super) struct Pen<'a> {
    pub(super) rect: Rect,
    pub(super) color: Color32,
    pub(super) pixel: f32,
    pub(super) shapes: &'a mut Vec<Shape>,
}

/// True for a character of the three blocks.
pub(super) fn is_shape(c: char) -> bool {
    matches!(u32::from(c), 0x2500..=0x259f | 0x2800..=0x28ff)
}

/// Draws a character of the three blocks as shapes. An other character draws nothing.
pub(super) fn draw(c: char, pen: &mut Pen<'_>) {
    let code = u32::from(c);
    match code {
        0x256d..=0x2570 => arc(code, pen),
        0x2571..=0x2573 => diagonal(code, pen),
        0x2500..=0x257f => lines::draw(code, pen),
        0x2580..=0x259f => block(code, pen),
        0x2800..=0x28ff => braille(code, pen),
        _ => {}
    }
}

impl Pen<'_> {
    /// A length in whole pixels, one pixel at least.
    fn snap(&self, length: f32) -> f32 {
        (length / self.pixel).round().max(1.0) * self.pixel
    }

    /// A place on the border of a pixel.
    fn on_pixel(&self, at: f32) -> f32 {
        (at / self.pixel).round() * self.pixel
    }

    /// The width of a light line: an eighth of a cell, in whole pixels.
    fn light(&self) -> f32 {
        self.snap(self.rect.width() / 8.0)
    }

    /// The top or left edge of a stroke of `width` through the middle of `from..to`.
    fn middle(&self, from: f32, to: f32, width: f32) -> f32 {
        self.on_pixel((from + to - width) / 2.0)
    }

    fn fill(&mut self, rect: Rect) {
        self.shapes.push(Shape::rect_filled(rect, 0.0, self.color));
    }
}

/// The round corners: a curve from the middle of one edge to the middle of the other.
fn arc(code: u32, pen: &mut Pen<'_>) {
    let rect = pen.rect;
    let light = pen.light();
    let x = pen.middle(rect.left(), rect.right(), light) + light / 2.0;
    let y = pen.middle(rect.top(), rect.bottom(), light) + light / 2.0;
    let (vertical, horizontal) = match code {
        0x256d => (rect.bottom(), rect.right()),
        0x256e => (rect.bottom(), rect.left()),
        0x256f => (rect.top(), rect.left()),
        _ => (rect.top(), rect.right()),
    };
    let points = [pos2(x, vertical), pos2(x, y), pos2(horizontal, y)];
    let curve =
        QuadraticBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, Stroke::new(light, pen.color));
    pen.shapes.push(curve.into());
}

fn diagonal(code: u32, pen: &mut Pen<'_>) {
    let rect = pen.rect;
    let stroke = Stroke::new(pen.light(), pen.color);
    if code != 0x2572 {
        pen.shapes.push(Shape::line_segment([rect.right_top(), rect.left_bottom()], stroke));
    }
    if code != 0x2571 {
        pen.shapes.push(Shape::line_segment([rect.left_top(), rect.right_bottom()], stroke));
    }
}

/// The block elements: parts of the cell in eighths, quadrants and shades.
fn block(code: u32, pen: &mut Pen<'_>) {
    let eighths = |code: u32| code as f32 / 8.0;
    // Upper left, upper right, lower left and lower right of the quadrants.
    const QUADRANTS: [u8; 10] = [0b0100, 0b1000, 0b0001, 0b1101, 0b1001, 0b0111, 0b1011, 0b0010, 0b0110, 0b1110];
    let parts: Vec<[f32; 4]> = match code {
        0x2580 => vec![[0.0, 0.0, 1.0, 0.5]],
        0x2581..=0x2588 => vec![[0.0, 1.0 - eighths(code - 0x2580), 1.0, 1.0]],
        0x2589..=0x258f => vec![[0.0, 0.0, eighths(0x2590 - code), 1.0]],
        0x2590 => vec![[0.5, 0.0, 1.0, 1.0]],
        0x2591..=0x2593 => {
            let alpha = (code - 0x2590) as f32 / 4.0;
            pen.color = pen.color.gamma_multiply(alpha);
            vec![[0.0, 0.0, 1.0, 1.0]]
        }
        0x2594 => vec![[0.0, 0.0, 1.0, 0.125]],
        0x2595 => vec![[0.875, 0.0, 1.0, 1.0]],
        _ => {
            let bits = QUADRANTS.get((code.saturating_sub(0x2596)) as usize).copied().unwrap_or_default();
            [[0.0, 0.0, 0.5, 0.5], [0.5, 0.0, 1.0, 0.5], [0.0, 0.5, 0.5, 1.0], [0.5, 0.5, 1.0, 1.0]]
                .into_iter()
                .enumerate()
                .filter(|(index, _)| bits & (1 << index) != 0)
                .map(|(_, part)| part)
                .collect()
        }
    };
    let rect = pen.rect;
    for [left, top, right, bottom] in parts {
        let x = |part: f32| pen.on_pixel(rect.left() + part * rect.width());
        let y = |part: f32| pen.on_pixel(rect.top() + part * rect.height());
        let part = Rect::from_min_max(pos2(x(left), y(top)), pos2(x(right), y(bottom)));
        pen.fill(part);
    }
}

/// A braille pattern: eight dots in two columns of four, a bit for each dot.
fn braille(code: u32, pen: &mut Pen<'_>) {
    let bits = code - 0x2800;
    // The bit of each dot, by column and row: dots 1, 2, 3 and 7 on the left.
    const DOTS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    let rect = pen.rect;
    let size = pen.snap(rect.width().min(rect.height() / 2.0) / 3.0);
    for (column, dots) in DOTS.iter().enumerate() {
        for (row, bit) in dots.iter().enumerate() {
            if bits & bit != 0 {
                let center = Pos2::new(
                    rect.left() + rect.width() * (1.0 + 2.0 * column as f32) / 4.0,
                    rect.top() + rect.height() * (1.0 + 2.0 * row as f32) / 8.0,
                );
                let corner = pos2(pen.on_pixel(center.x - size / 2.0), pen.on_pixel(center.y - size / 2.0));
                pen.fill(Rect::from_min_size(corner, vec2(size, size)));
            }
        }
    }
}

#[cfg(test)]
mod tests;
