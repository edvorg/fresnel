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
