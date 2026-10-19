use std::collections::BTreeSet;

use super::*;

/// Each round has each kind of fault once, so a short soak has each one; the sequence of a
/// round is random.
#[test]
fn each_round_has_each_kind_of_fault_once_in_a_random_sequence() {
    let (mut rounds, mut rng, now) = (Rounds::new(Duration::ZERO), fastrand::Rng::with_seed(1), Instant::now());
    let mut sequences = BTreeSet::new();
    for _ in 0..20 {
        let round: Vec<Fault> = (0..ROUND.len()).map(|_| rounds.next(&mut rng, now)).collect();
        let mut sorted = round.clone();
        sorted.sort();
        assert_eq!(sorted, ROUND, "each kind once: {round:?}");
        sequences.insert(round);
    }
    assert!(sequences.len() > 10, "the rounds have different sequences: {}", sequences.len());
}

/// A restart of k3s before the gap of the pace ends is a reset: restarts in a row gave the
/// environment more outages than minutes of the app.
#[test]
fn a_restart_before_the_gap_is_a_reset_and_one_after_it_is_a_restart() {
    let gap = Duration::from_secs(20 * 60);
    let (mut rounds, mut rng, start) = (Rounds::new(gap), fastrand::Rng::with_seed(2), Instant::now());
    let restarts = |rounds: &mut Rounds, rng: &mut fastrand::Rng, now: Instant| {
        (0..ROUND.len()).filter(|_| rounds.next(rng, now) == Fault::Restart).count()
    };
    assert_eq!(restarts(&mut rounds, &mut rng, start), 1, "the first restart");
    let soon = start + Duration::from_secs(5 * 60);
    assert_eq!(restarts(&mut rounds, &mut rng, soon), 0, "5 minutes later: a reset in its place");
    assert_eq!(restarts(&mut rounds, &mut rng, start + gap), 1, "after the gap");
}
