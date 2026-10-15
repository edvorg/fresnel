use fresnel_core::logs::LineKind;

use super::*;

fn line(text: &str) -> LogLine {
    LogLine { time: 0, source: 0, text: text.into(), spans: None, kind: LineKind::Log, late: false, cut: false }
}

fn lines(texts: &[&str]) -> Lines {
    Lines::new(10, texts.iter().map(|text| line(text)))
}

#[test]
fn the_shown_lines_are_each_line_or_the_kept_ones_by_number() {
    let all = lines(&["a", "b", "c", "d"]);
    let shown = Shown { lines: &all, kept: None };
    assert_eq!((shown.len(), shown.number(0), shown.number(3), shown.number(4)), (4, Some(10), Some(13), None));
    assert_eq!((shown.index_from(0), shown.index_from(12), shown.index_from(99)), (0, 2, 4));
    let kept = VecDeque::from([11, 13]);
    let shown = Shown { lines: &all, kept: Some(&kept) };
    assert_eq!((shown.len(), shown.number(1), shown.index_from(12), shown.index_from(14)), (2, Some(13), 1, 2));
    assert_eq!(shown.line(0).map(|(number, line)| (number, &*line.text)), Some((11, "b")));
}

#[test]
fn a_row_has_the_columns_of_the_pane_and_a_character_is_never_cut() {
    let wrap = Cut { columns: Some(4) };
    let rows = |text: &str| wrap.rows(&line(text)).into_iter().map(|row| row.bytes).collect::<Vec<_>>();
    assert_eq!(rows("abcdefghij"), [0..4, 4..8, 8..10]);
    assert_eq!(rows("abcd"), vec![0..4]);
    assert_eq!(rows(""), vec![0..0]);
    assert_eq!(rows("größerer"), [0..6, 6..10], "four characters in each row, of one or two bytes");
    assert_eq!(Cut { columns: None }.rows(&line("abcdefghij")), [Row { bytes: 0..10, more: false }]);
    for text in ["abcdefghij", "abcd", "", "größerer"] {
        assert_eq!(wrap.count(&line(text)), wrap.rows(&line(text)).len(), "{text}");
    }
}

#[test]
fn a_long_line_shows_its_laid_out_part_and_a_row_for_the_full_line() {
    let long = line(&"x".repeat(LAID_OUT + 500));
    let wrap = Cut { columns: Some(100) };
    let rows = wrap.rows(&long);
    assert_eq!(rows.len(), LAID_OUT / 100 + 1);
    assert_eq!(rows.last(), Some(&Row { bytes: LAID_OUT..LAID_OUT, more: true }));
    assert_eq!(wrap.count(&long), rows.len());
    let one = Cut { columns: None };
    assert_eq!(
        one.rows(&long),
        [Row { bytes: 0..LAID_OUT, more: false }, Row { bytes: LAID_OUT..LAID_OUT, more: true }]
    );
    let cut = LogLine { cut: true, ..line("short") };
    assert_eq!((one.count(&cut), wrap.count(&cut)), (2, 2), "the core cut the line: the row tells it");
}

#[test]
fn a_walk_crosses_the_rows_of_each_line_and_stops_at_the_ends() {
    // Rows: 10 has 3, 11 has 1, 12 has 2.
    let all = lines(&["abcdefghij", "ab", "abcdef"]);
    let (shown, cut) = (Shown { lines: &all, kept: None }, Cut { columns: Some(4) });
    let at = |line, row| Position { line, row };
    assert_eq!(walk(shown, cut, at(10, 0), 2), at(10, 2));
    assert_eq!(walk(shown, cut, at(10, 2), 1), at(11, 0));
    assert_eq!(walk(shown, cut, at(10, 1), 3), at(12, 0));
    assert_eq!(walk(shown, cut, at(10, 0), 100), at(12, 1), "the last row");
    assert_eq!(walk(shown, cut, at(12, 1), -3), at(10, 2));
    assert_eq!(walk(shown, cut, at(12, 0), -100), at(10, 0), "the first row");
    assert_eq!(walk(shown, cut, at(3, 0), 0), at(10, 0), "a line that left the ring gives the first one");
    assert_eq!(end(shown, cut), Some(at(12, 1)));

    let kept = VecDeque::from([10, 12]);
    let filtered = Shown { lines: &all, kept: Some(&kept) };
    assert_eq!(walk(filtered, cut, at(10, 2), 1), at(12, 0), "a line that the filter hides has no row");
    assert_eq!(walk(filtered, cut, at(11, 0), 0), at(12, 0));
    assert_eq!(end(Shown { lines: &all, kept: Some(&VecDeque::new()) }, cut), None);
}
