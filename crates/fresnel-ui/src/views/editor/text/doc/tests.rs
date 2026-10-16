use super::*;

fn doc(text: &str) -> Doc {
    Doc::new(text, 10, true)
}

fn at(doc: &mut Doc, line: usize, byte: usize) {
    doc.place(Pos::new(line, byte), false);
}

fn run(doc: &mut Doc, acts: impl IntoIterator<Item = Act>) {
    for (index, act) in acts.into_iter().enumerate() {
        doc.act(act, index as f64 * 0.1, 3);
    }
}

#[test]
fn enter_keeps_the_indent_and_adds_a_step_after_a_colon_and_tab_gives_spaces() {
    let mut doc = doc("spec:\n  containers:\n  - name: web\n");
    at(&mut doc, 1, 13);
    run(&mut doc, [Act::Newline, Act::Type("- x".to_owned()), Act::Newline]);
    assert_eq!(doc.buffer.text(), "spec:\n  containers:\n    - x\n      \n  - name: web\n");
    at(&mut doc, 4, 13);
    run(&mut doc, [Act::Newline, Act::Indent, Act::Type("a".to_owned()), Act::Indent]);
    assert_eq!(doc.buffer.line(5), "      a ", "the indent of the item, a step, and a space to the even column");
    assert!(!doc.buffer.text().contains('\t'));
}

#[test]
fn tab_and_shift_tab_move_each_selected_line_and_the_selection_with_them() {
    let mut doc = doc("a:\n  b: 1\n  c: 2\nd: 3\n");
    doc.select(Selection { anchor: Pos::new(1, 3), head: Pos::new(3, 0) });
    run(&mut doc, [Act::Indent]);
    assert_eq!(doc.buffer.text(), "a:\n    b: 1\n    c: 2\nd: 3\n", "the line of an end at column 0 stays");
    assert_eq!(doc.selection, Selection { anchor: Pos::new(1, 5), head: Pos::new(3, 0) });
    run(&mut doc, [Act::Unindent, Act::Unindent, Act::Unindent]);
    assert_eq!(doc.buffer.text(), "a:\nb: 1\nc: 2\nd: 3\n");
    assert_eq!(doc.act(Act::Unindent, 1.0, 3), Outcome::None, "no indent to take");
}

#[test]
fn backspace_takes_a_character_a_step_of_indent_a_word_or_the_selection() {
    let mut doc = doc("    name: web-api\n");
    at(&mut doc, 0, 4);
    run(&mut doc, [Act::Delete { forward: false, unit: Unit::Character }]);
    assert_eq!(doc.buffer.line(0), "  name: web-api");
    at(&mut doc, 0, 15);
    run(&mut doc, [Act::Delete { forward: false, unit: Unit::Word }]);
    assert_eq!(doc.buffer.line(0), "  name: web-");
    run(
        &mut doc,
        [Act::Move { motion: Motion::LineStart, extend: true }, Act::Delete { forward: true, unit: Unit::Word }],
    );
    assert_eq!(doc.buffer.line(0), "  ", "the selection");
}

#[test]
fn undo_and_redo_give_each_text_and_selection_back() {
    let mut doc = doc("kind: Pod\n");
    at(&mut doc, 0, 9);
    run(&mut doc, [Act::Type("s".to_owned()), Act::Type("!".to_owned())]);
    run(&mut doc, [Act::Move { motion: Motion::Start, extend: false }, Act::Insert("# a\r\n".to_owned())]);
    assert_eq!(doc.buffer.text(), "# a\nkind: Pods!\n", "a paste has no carriage return");
    run(&mut doc, [Act::Undo]);
    assert_eq!((doc.buffer.text().as_str(), doc.selection), ("kind: Pods!\n", Selection::at(Pos::new(0, 0))));
    run(&mut doc, [Act::Undo]);
    assert_eq!((doc.buffer.text().as_str(), doc.selection.head), ("kind: Pod\n", Pos::new(0, 9)), "typing is one step");
    assert!(!doc.can_undo() && doc.can_redo());
    run(&mut doc, [Act::Redo, Act::Redo]);
    assert_eq!(doc.buffer.text(), "# a\nkind: Pods!\n");
    assert_eq!(doc.act(Act::Redo, 9.0, 3), Outcome::None);
}

#[test]
fn a_change_past_the_line_limit_and_a_change_of_a_read_only_text_are_refused() {
    let mut doc = doc("a\n");
    run(&mut doc, [Act::Insert("1\n2\n3\n4\n5\n6\n7\n8\n".to_owned())]);
    assert_eq!(doc.buffer.text_lines(), 9);
    let refused = doc.act(Act::Insert("x\ny\n".to_owned()), 1.0, 3);
    assert_eq!(refused, Outcome::Refused("The editor holds 10 lines at most, so the text did not change.".to_owned()));
    assert_eq!(doc.buffer.text_lines(), 9, "the text as before");
    assert_eq!(doc.act(Act::Insert("x\n".to_owned()), 2.0, 3), Outcome::Changed, "the tenth line");

    let mut read_only = Doc::new("a: 1\n", 10, false);
    assert!(matches!(read_only.act(Act::Type("x".to_owned()), 0.0, 3), Outcome::Refused(_)));
    assert_eq!(read_only.act(Act::SelectAll, 0.0, 3), Outcome::Moved);
    assert_eq!(read_only.selected(), "a: 1\n");

    // A save on its way holds the text, and a move still works.
    let mut held = Doc::new("a: 1\n", 10, true);
    held.held = true;
    let waits = Outcome::Refused("The text waits for the answer of its save.".to_owned());
    assert_eq!(held.act(Act::Type("x".to_owned()), 0.0, 3), waits);
    assert_eq!(held.act(Act::Undo, 0.0, 3), waits);
    assert_eq!(held.act(Act::SelectAll, 0.0, 3), Outcome::Moved);
    held.held = false;
    assert_eq!(held.act(Act::Type("x".to_owned()), 0.0, 3), Outcome::Changed);
}

#[test]
fn up_and_down_keep_the_column_and_left_ends_a_selection_at_its_start() {
    let mut doc = doc("abcdef\nab\nabcdef\n");
    at(&mut doc, 0, 5);
    run(
        &mut doc,
        [Act::Move { motion: Motion::Down, extend: false }, Act::Move { motion: Motion::Down, extend: false }],
    );
    assert_eq!(doc.selection.head, Pos::new(2, 5));
    run(
        &mut doc,
        [Act::Move { motion: Motion::WordLeft, extend: true }, Act::Move { motion: Motion::Left, extend: false }],
    );
    assert_eq!(doc.selection, Selection::at(Pos::new(2, 0)));
}

/// A paste of a line of megabytes passes the line limit: a change that would make the text
/// larger than 8 MiB does not go in, and a larger text can get smaller.
#[test]
fn a_change_past_8_mib_is_refused_and_a_larger_text_can_get_smaller() {
    let mut doc = doc("a: 1\n");
    let refused = doc.act(Act::Insert("x".repeat(MOST_BYTES)), 0.0, 3);
    assert_eq!(
        refused,
        Outcome::Refused("The editor holds 8 MiB of text at most, so the text did not change.".to_owned())
    );
    assert_eq!(doc.buffer.text(), "a: 1\n");
    assert_eq!(doc.act(Act::Insert("x".repeat(MOST_BYTES - 5)), 1.0, 3), Outcome::Changed, "8 MiB");

    let mut large = Doc::new(&"x".repeat(MOST_BYTES + 10), 10, true);
    assert_eq!(large.act(Act::Delete { forward: true, unit: Unit::Character }, 0.0, 3), Outcome::Changed);
    assert_eq!(large.buffer.bytes(), MOST_BYTES + 9);
}
