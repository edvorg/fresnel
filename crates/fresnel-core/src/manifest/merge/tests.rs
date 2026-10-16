use super::*;

const BASE: &str = "kind: Deployment\nmetadata:\n  name: web\n  resourceVersion: \"7\"\nspec:\n  replicas: 2\n  \
                    template:\n    spec:\n      containers:\n      - image: nginx:1.27\n        name: web\n";

/// The text of a merge with the side of the user in each conflict.
fn ours(merge: &Merge) -> String {
    merge.text(|_| Side::Ours)
}

#[test]
fn changes_of_other_lines_merge_and_the_new_version_of_the_server_comes_with_them() {
    let mine = BASE.replace("nginx:1.27", "nginx:1.28");
    let server = BASE.replace("\"7\"", "\"9\"").replace("replicas: 2", "replicas: 5");
    let merged = merge(BASE, &mine, &server);
    assert!(merged.is_clean(), "{merged:?}");
    assert_eq!(
        ours(&merged),
        BASE.replace("nginx:1.27", "nginx:1.28").replace("\"7\"", "\"9\"").replace("replicas: 2", "replicas: 5")
    );
    assert_eq!(merged.parts.len(), 1, "the merged lines are one part");
}

#[test]
fn changes_of_the_same_lines_are_a_conflict_with_the_three_versions_unless_they_are_equal() {
    let mine = BASE.replace("replicas: 2", "replicas: 3");
    let server = BASE.replace("replicas: 2", "replicas: 5").replace("\"7\"", "\"9\"");
    let merged = merge(BASE, &mine, &server);
    let conflicts: Vec<&Conflict> = merged.conflicts().collect();
    let three =
        Conflict { base: "  replicas: 2\n".into(), ours: "  replicas: 3\n".into(), theirs: "  replicas: 5\n".into() };
    assert_eq!(conflicts, [&three]);
    assert_eq!(merged.text(|_| Side::Theirs), server);
    assert_eq!(ours(&merged), mine.replace("\"7\"", "\"9\""), "the other change of the server comes in any case");

    let same = merge(BASE, &mine, &mine.replace("\"7\"", "\"9\""));
    assert!(same.is_clean(), "both sides made the same change");
    assert_eq!(ours(&same), mine.replace("\"7\"", "\"9\""));
}

#[test]
fn an_insertion_that_touches_a_change_is_a_conflict_and_changes_of_lines_side_by_side_are_not() {
    let base = "a\nb\nc\nd\n";
    let side_by_side = merge(base, "a\nB\nc\nd\n", "a\nb\nC\nd\n");
    assert_eq!((side_by_side.is_clean(), ours(&side_by_side)), (true, "a\nB\nC\nd\n".to_owned()));
    let touching = [
        (base, "a\nB\nc\nd\n", "a\nb\nx\nc\nd\n"),
        (base, "a\nx\nb\nc\nd\n", "a\nB\nc\nd\n"),
        ("a\nb\n", "a\nx\nb\n", "a\ny\nb\n"),
    ];
    for (base, mine, server) in touching {
        let touching = merge(base, mine, server);
        assert_eq!(touching.conflicts().count(), 1, "{mine:?} and {server:?}: {touching:?}");
    }
    let apart = merge(base, "x\na\nb\nc\nd\n", "a\nb\nc\nd\ny\n");
    assert_eq!((apart.is_clean(), ours(&apart)), (true, "x\na\nb\nc\nd\ny\n".to_owned()));
}

#[test]
fn a_side_with_no_change_takes_the_other_side() {
    let mine = BASE.replace("name: web\n", "name: web\n  labels:\n    tier: front\n");
    assert_eq!(ours(&merge(BASE, &mine, BASE)), mine);
    assert_eq!(ours(&merge(BASE, BASE, &mine)), mine);
    assert_eq!(ours(&merge(BASE, BASE, BASE)), BASE);
    let removed = merge(BASE, "", BASE);
    assert_eq!((removed.is_clean(), ours(&removed)), (true, String::new()), "the user removed each line");
}

/// A random edit of `lines`: a line changes, goes away or gets a new line before it. Each new
/// line is unique, so a line of the base keeps one place in each version.
fn edit(lines: &[String], random: &mut fastrand::Rng, side: &str) -> Vec<String> {
    let mut edited = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        match random.u8(0..4) {
            0 => {}
            1 => edited.push(format!("{side} changed {index}\n")),
            2 => edited.extend([format!("{side} new {index}\n"), line.clone()]),
            _ => edited.push(line.clone()),
        }
    }
    edited
}

/// Random edits of two parts of a text with an equal line between them merge with no
/// conflict into the text with both edits, in either order of the sides; a side with no
/// change gives the other side.
#[test]
fn edits_of_random_texts_in_parts_apart_merge_into_both_edits() {
    let mut random = fastrand::Rng::with_seed(5);
    for _ in 0..2_000 {
        let base: Vec<String> = (0..random.usize(3..30)).map(|line| format!("line {line}\n")).collect();
        let split = random.usize(0..base.len());
        let (before, after) = (&base[..split], &base[split + 1..]);
        let (head, tail) = (edit(before, &mut random, "ours"), edit(after, &mut random, "theirs"));
        let middle = std::slice::from_ref(&base[split]);
        let (ours, theirs) = ([&head, middle, after].concat(), [before, middle, &tail].concat());
        let both = [&head, middle, &tail].concat().concat();
        let (base, ours, theirs) = (base.concat(), ours.concat(), theirs.concat());
        let merged = merge(&base, &ours, &theirs);
        assert!(merged.is_clean(), "{base:?} {ours:?} {theirs:?}: {merged:?}");
        assert_eq!(merged.text(|_| Side::Ours), both, "{base:?} {ours:?} {theirs:?}");
        assert_eq!(merge(&base, &theirs, &ours).text(|_| Side::Theirs), both);
        assert_eq!(merge(&base, &ours, &base).text(|_| Side::Theirs), ours);
        assert_eq!(merge(&base, &base, &theirs).text(|_| Side::Ours), theirs);
    }
}
