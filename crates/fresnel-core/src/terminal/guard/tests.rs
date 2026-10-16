use super::*;

/// The bytes that pass, fed in pieces of `step` bytes: a sequence can end in a later chunk.
fn passed(input: &[u8], step: usize) -> Vec<u8> {
    let mut guard = Guard::default();
    let mut output = Vec::new();
    for chunk in input.chunks(step) {
        guard.filter(chunk, &mut output);
    }
    output
}

#[test]
fn text_and_sequences_under_the_limits_pass_whole() {
    let input = "ls \x1b[1;34mbin\x1b[m é 中\x1b]0;title\x07\x1b(B\x1bP1$r\x1b\\ok".as_bytes();
    for step in [1, 2, 3, 1000] {
        assert_eq!(passed(input, step), input, "in pieces of {step}");
    }
}

#[test]
fn an_osc_sequence_with_no_end_passes_its_first_4_kib_and_its_end() {
    let mut input = b"\x1b]0;".to_vec();
    input.extend(std::iter::repeat_n(b'x', 10 * 1024 * 1024));
    input.extend(b"\x07after");
    for step in [7, 4096] {
        let output = passed(&input, step);
        // `ESC ]`, the limit, BEL and the text after it.
        assert_eq!(output.len(), 2 + OSC_LIMIT + 6, "in pieces of {step}");
        assert!(output.ends_with(b"x\x07after"));
    }
    // ESC ends a sequence too, and the next one has its own limit.
    let twice = [b"\x1b]2;".as_slice(), &[b'y'; 5000], b"\x1b\\\x1b]2;", &[b'z'; 10], b"\x1b\\"].concat();
    assert_eq!(passed(&twice, 100).len(), 2 + OSC_LIMIT + 2 + 4 + 10 + 2);
}

#[test]
fn sixteen_marks_of_zero_width_in_a_row_pass_and_the_rest_waits_for_a_character_with_a_width() {
    let accent = "\u{301}";
    let many = format!("e{}x{}", accent.repeat(40), accent.repeat(3));
    let output = String::from_utf8(passed(many.as_bytes(), 5)).expect("text");
    assert_eq!(output, format!("e{}x{}", accent.repeat(MARKS_LIMIT), accent.repeat(3)));
    // A sequence of the cursor between the marks does not start the count again.
    let moved = format!("e{}", format!("{accent}\x1b[1C\x1b[1D").repeat(40));
    let output = String::from_utf8(passed(moved.as_bytes(), 3)).expect("text");
    assert_eq!(output.matches(accent).count(), MARKS_LIMIT);
    assert_eq!(output.matches("\x1b[1C").count(), 40, "each sequence passes");
}

#[test]
fn a_cut_character_passes_as_it_came() {
    assert_eq!(passed(b"a\xe4\xb8\x1b[m\xffz", 1), b"a\xe4\xb8\x1b[m\xffz");
}

#[test]
fn a_control_of_8_bits_or_the_end_of_a_string_does_not_start_the_count_of_marks_again() {
    let accents = |output: Vec<u8>| output.windows(2).filter(|pair| *pair == "\u{301}".as_bytes()).count();
    // The parser runs a control of 8 bits, such as 0x80, and prints nothing: each mark after
    // it goes on the same character.
    let mut controls = b"e".to_vec();
    for _ in 0..10 {
        controls.push(0x80);
        controls.extend("\u{301}".repeat(16).as_bytes());
    }
    assert_eq!(accents(passed(&controls, 5)), MARKS_LIMIT);
    // The parser ends DCS data at the end of a string of 8 bits, 0x9c, and prints after it.
    let after_dcs = [b"\x1bPq\x9ce".as_slice(), "\u{301}".repeat(40).as_bytes()].concat();
    assert_eq!(accents(passed(&after_dcs, 3)), MARKS_LIMIT);
}

#[test]
fn a_repeat_has_a_limit_and_a_mark_of_zero_width_is_not_repeated() {
    // The parser drops the sequence at CAN; a new one with the limit follows.
    let limit = format!("\x1b[{REPEAT_LIMIT}b");
    for step in [1, 4, 1000] {
        assert_eq!(passed(b"x\x1b[65535b", step), [b"x\x1b[65535\x18".as_slice(), limit.as_bytes()].concat());
        assert_eq!(passed(b"ab\x1b[3;7b", step), b"ab\x1b[3;7\x18\x1b[3b", "the first parameter counts");
        assert_eq!(passed(b"a\x1b[b\x1b[0b", step), b"a\x1b[\x18\x1b[1b\x1b[0\x18\x1b[1b", "one repeat");
        // xterm repeats no mark of zero width: one mark repeated was 65,535 marks on a cell.
        assert_eq!(passed("e\u{301}\x1b[1C\x1b[9b".as_bytes(), step), "e\u{301}\x1b[1C\x1b[9\x18".as_bytes());
    }
    // A sequence with a private mark or an intermediate byte is no repeat; ESC ends one.
    for other in [b"x\x1b[?5b".as_slice(), b"x\x1b[5 b"] {
        assert_eq!(passed(other, 2), other);
    }
    assert_eq!(passed(b"x\x1b[5\x1b[2b", 2), b"x\x1b[5\x1b[2\x18\x1b[2b");
}

#[test]
fn a_link_passes_its_number_only_and_the_other_osc_sequences_their_text() {
    let link = b"\x1b]8;id=1;https://example.test/a\x07link\x1b]8;;\x1b\\";
    for step in [1, 3, 1000] {
        assert_eq!(passed(link, step), b"\x1b]8;\x07link\x1b]8;\x1b\\");
        assert_eq!(passed(b"\x1b]0;8;x\x07\x1b]88;y\x07", step), b"\x1b]0;8;x\x07\x1b]88;y\x07");
    }
}
