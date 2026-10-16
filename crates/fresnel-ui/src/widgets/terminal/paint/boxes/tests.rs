use super::*;

/// A cell of 8 x 15.5 points at two pixels for each point, as the spike measured.
fn shapes(c: char) -> (bool, Vec<Shape>) {
    let mut shapes = Vec::new();
    let rect = Rect::from_min_size(pos2(80.0, 31.0), vec2(8.0, 15.5));
    let mut pen = Pen { rect, color: Color32::WHITE, pixel: 0.5, shapes: &mut shapes };
    draw(c, &mut pen);
    (is_shape(c), shapes)
}

/// The rectangles of the shapes.
fn rects(c: char) -> Vec<Rect> {
    shapes(c).1.iter().filter_map(|shape| if let Shape::Rect(rect) = shape { Some(rect.rect) } else { None }).collect()
}

fn cover(rects: &[Rect]) -> Rect {
    rects.iter().fold(Rect::NOTHING, |all, rect| all.union(*rect))
}

#[test]
fn each_character_of_the_three_blocks_is_shapes_and_an_other_one_is_not() {
    for code in (0x2500..=0x259f).chain(0x2801..=0x28ff) {
        let c = char::from_u32(code).expect("a character");
        let (drawn, shapes) = shapes(c);
        assert!(drawn && !shapes.is_empty(), "{c} U+{code:04X}");
    }
    assert_eq!(shapes('\u{2800}'), (true, Vec::new()), "the empty braille pattern");
    assert_eq!(shapes('a'), (false, Vec::new()));
    assert!(!is_shape('中') && !is_shape('\u{25a0}'));
}

#[test]
fn a_line_reaches_the_edges_of_its_cell_so_two_rows_and_two_columns_join() {
    let horizontal = cover(&rects('─'));
    assert_eq!((horizontal.left(), horizontal.right()), (80.0, 88.0));
    assert_eq!(horizontal.height(), 1.0, "one point: two pixels");
    let vertical = cover(&rects('│'));
    assert_eq!((vertical.top(), vertical.bottom()), (31.0, 46.5));
    assert_eq!(cover(&rects('┃')).width(), 2.0, "heavy is twice as wide");
    let mut double: Vec<f32> = rects('║').iter().map(|rect| rect.left()).collect();
    double.sort_by(f32::total_cmp);
    double.dedup();
    assert_eq!(double.len(), 2, "double is two lines, up and down");
    assert_eq!(rects('┄').len(), 3, "three dashes");
    let corner = cover(&rects('┌'));
    assert_eq!((corner.right(), corner.bottom()), (88.0, 46.5));
    assert!(corner.left() > 80.0 && corner.top() > 31.0);
    assert_eq!(rects('┼').len(), 4);
    // Each stroke of a double corner turns into its own corner: no stroke reaches past the
    // outer one, and the inner one starts inside the cell.
    let corner = rects('╔');
    let outer = cover(&corner);
    assert_eq!((outer.right(), outer.bottom()), (88.0, 46.5));
    let inner_left = corner.iter().map(|rect| rect.left()).fold(f32::MIN, f32::max);
    assert!(inner_left > outer.left(), "{corner:?}");
    assert_eq!(corner.len(), 4);
}

#[test]
fn a_block_fills_its_part_of_the_cell_on_whole_pixels() {
    assert_eq!(cover(&rects('█')), Rect::from_min_size(pos2(80.0, 31.0), vec2(8.0, 15.5)));
    let lower = cover(&rects('▄'));
    assert_eq!((lower.top(), lower.bottom()), (39.0, 46.5), "the half of 15.5 points, on a pixel");
    assert_eq!(cover(&rects('▐')).left(), 84.0);
    assert_eq!(rects('▚').len(), 2);
    assert_eq!(rects('⣿').len(), 8);
    assert_eq!(rects('⠁').len(), 1);
}
