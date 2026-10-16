use super::*;

/// The text of `old` with each hunk applied.
fn apply(old: &[&str], new: &[&str], hunks: &[Hunk]) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    for hunk in hunks {
        out.extend(old[at..hunk.old.start].iter().map(|line| (*line).to_owned()));
        out.extend(new[hunk.new.clone()].iter().map(|line| (*line).to_owned()));
        at = hunk.old.end;
    }
    out.extend(old[at..].iter().map(|line| (*line).to_owned()));
    out
}

/// The fewest changed lines from `old` to `new`, by the longest common subsequence.
fn fewest(old: &[&str], new: &[&str]) -> usize {
    let mut table = vec![vec![0_usize; new.len() + 1]; old.len() + 1];
    for (i, a) in old.iter().enumerate() {
        for (j, b) in new.iter().enumerate() {
            table[i + 1][j + 1] = if a == b { table[i][j] + 1 } else { table[i][j + 1].max(table[i + 1][j]) };
        }
    }
    old.len() + new.len() - 2 * table[old.len()][new.len()]
}

#[test]
fn a_change_far_from_an_other_one_is_a_hunk_of_its_own() {
    let old = lines("a\nb\nc\nd\ne\nf\ng\n");
    let new = lines("a\nB\nc\nd\ne\nf\nG\nh\n");
    assert_eq!(hunks(&old, &new), [Hunk { old: 1..2, new: 1..2 }, Hunk { old: 6..7, new: 6..8 }]);
    assert_eq!(hunks(&old, &old), [], "equal texts");
    assert_eq!(hunks(&lines("a\nb"), &lines("a\nb\n")), [Hunk { old: 1..2, new: 1..2 }], "a line feed at the end");
    assert_eq!(hunks(&[], &lines("x\n")), [Hunk { old: 0..0, new: 0..1 }]);
    assert_eq!(hunks(&lines("x\n"), &[]), [Hunk { old: 0..1, new: 0..0 }]);
}

/// Random texts of few distinct lines: the hunks give the new text, have the fewest
/// changed lines, and an equal line between each two of them.
#[test]
fn the_hunks_of_random_texts_are_the_shortest_edit() {
    let mut random = fastrand::Rng::with_seed(12);
    let words = ["a\n", "b\n", "c\n", "d\n"];
    for _ in 0..2_000 {
        let text = |random: &mut fastrand::Rng| -> Vec<&str> {
            (0..random.usize(0..12)).map(|_| words[random.usize(0..words.len())]).collect()
        };
        let (old, new) = (text(&mut random), text(&mut random));
        let found = hunks(&old, &new);
        assert_eq!(apply(&old, &new, &found), new, "{old:?} to {new:?}: {found:?}");
        let changed: usize = found.iter().map(|hunk| hunk.old.len() + hunk.new.len()).sum();
        assert_eq!(changed, fewest(&old, &new), "{old:?} to {new:?}: {found:?}");
        assert!(
            found.windows(2).all(|pair| pair[0].old.end < pair[1].old.start && pair[0].new.end < pair[1].new.start)
        );
    }
}

#[test]
fn more_changes_than_the_limit_are_one_hunk_from_the_first_to_the_last_change() {
    let old: Vec<String> = (0..3_000).map(|line| format!("{line}\n")).collect();
    let mut new = old.clone();
    for line in (10..2_990).step_by(2) {
        new[line] = format!("new {line}\n");
    }
    let (old, new): (Vec<&str>, Vec<&str>) =
        (old.iter().map(String::as_str).collect(), new.iter().map(String::as_str).collect());
    let found = hunks(&old, &new);
    assert_eq!(found, [Hunk { old: 10..2_989, new: 10..2_989 }]);
    assert_eq!(apply(&old, &new, &found), new);
}
