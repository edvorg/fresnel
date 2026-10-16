use super::*;

/// The text and the kind of each token of a line.
fn parts(line: &str, state: State) -> Vec<(&str, Kind)> {
    tokens(line, state).into_iter().map(|token| (line.get(token.bytes).unwrap_or("?"), token.kind)).collect()
}

#[test]
fn a_line_has_its_key_its_marks_its_value_and_its_comment() {
    use Kind::*;
    let plain = State::Plain;
    assert_eq!(
        parts("  name: web # the app", plain),
        [("name", Key), (":", Mark), ("web", Text), ("# the app", Comment)]
    );
    assert_eq!(
        parts("- - image: nginx:1.27", plain),
        [("-", Mark), ("-", Mark), ("image", Key), (":", Mark), ("nginx:1.27", Text)]
    );
    assert_eq!(parts("replicas: 3", plain), [("replicas", Key), (":", Mark), ("3", Number)]);
    assert_eq!(parts("ready: true", plain)[2], ("true", Word));
    assert_eq!(parts("on: yes", plain), [("on", Key), (":", Mark), ("yes", Word)]);
    assert_eq!(
        parts("url: http://x:80/a#b", plain)[2],
        ("http://x:80/a#b", Text),
        "no key end and no comment in a word"
    );
    assert_eq!(
        parts("'a: b': \"x \\\" y\" # c", plain),
        [("'a: b'", Key), (":", Mark), ("\"x \\\" y\"", Quoted), ("# c", Comment)]
    );
    assert_eq!(parts("data: !!binary iVBO", plain), [("data", Key), (":", Mark), ("!!binary", Tag), ("iVBO", Text)]);
    assert_eq!(parts("ports: [80, {a: 1}]", plain).iter().filter(|part| part.1 == Number).count(), 2);
    assert_eq!(parts("--- # one", plain), [("---", Document), ("# one", Comment)]);
    assert_eq!(parts("   ", plain), []);
}

#[test]
fn the_body_of_a_block_scalar_is_text_until_a_line_with_less_indent() {
    let text = "data:\n  ca.crt: |-\n    -----BEGIN-----\n\n    key: no key\n  next: 1\nlist:\n- |\n  body\nend: 1\n";
    let buffer = Buffer::new(text);
    let states = states(&buffer);
    assert_eq!(states[2], State::Block(2), "the node of the block is the key");
    let kinds: Vec<Vec<Kind>> = (0..buffer.count())
        .map(|line| tokens(buffer.line(line), states[line]).into_iter().map(|token| token.kind).collect())
        .collect();
    assert_eq!(kinds[4], [Kind::Block], "a colon in the body is text");
    assert_eq!(kinds[5], [Kind::Key, Kind::Mark, Kind::Number], "the end of the body");
    assert_eq!(
        (states[8], kinds[8].as_slice()),
        (State::Block(0), [Kind::Block].as_slice()),
        "the node of a list entry"
    );
    assert_eq!(kinds[9], [Kind::Key, Kind::Mark, Kind::Number], "a line with no indent ends it");
}

#[test]
fn a_long_line_is_read_in_its_first_part_and_its_last_token_takes_the_rest() {
    let line = format!("raw: !!binary {}", "A".repeat(100_000));
    let found = tokens(&line, State::Plain);
    assert_eq!(found.last().map(|token| (token.bytes.end, token.kind)), Some((line.len(), Kind::Text)));
    let wide = format!("k: {}", "ü".repeat(READ));
    assert_eq!(
        tokens(&wide, State::Plain).last().map(|token| token.bytes.end),
        Some(wide.len()),
        "a cut at a character"
    );
}
