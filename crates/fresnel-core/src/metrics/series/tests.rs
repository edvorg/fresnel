use super::*;

/// The tiers of the settings: 15 s steps for one hour, 1 minute steps for 24 hours.
const TIERS: Tiers = Tiers { fine: 3_600, step: 60, coarse: 86_400 };
/// A time of the cluster at the start of a minute.
const START: u32 = 1_792_058_400;

fn sample(at: u32, cpu: u64, memory: u64) -> Sample {
    Sample { at, cover: Cover::ONE, usage: Amount { cpu, memory } }
}

fn times(series: &Series) -> Vec<u32> {
    series.samples().map(|sample| sample.at - START).collect()
}

#[test]
fn a_sample_that_is_not_later_than_the_last_one_is_not_added() {
    let mut series = Series::default();
    assert!(series.push(sample(START + 15, 1, 1), &TIERS));
    assert!(!series.push(sample(START + 15, 2, 2), &TIERS), "the same sample in a second answer");
    assert!(!series.push(sample(START + 5, 3, 3), &TIERS), "an older one of a second replica");
    assert!(series.push(sample(START + 30, 4, 4), &TIERS));
    assert_eq!(times(&series), [15, 30]);
    assert_eq!(series.latest(), Some(sample(START + 30, 4, 4)));
}

#[test]
fn the_fine_tier_keeps_one_hour_of_samples_and_the_coarse_tier_one_per_minute() {
    let mut series = Series::default();
    // Two hours of samples each 15 s, with the CPU of the second of each minute.
    for step in 0..480u32 {
        let second = step * 15;
        assert!(series.push(sample(START + second, u64::from(second % 60), u64::from(second)), &TIERS));
    }
    assert_eq!(series.fine.len(), 240, "15 s steps for one hour");
    assert_eq!(series.oldest(Tier::Fine), Some(START + 3_600));
    // 119 minutes ended; the last one is still open.
    assert_eq!(series.coarse.len(), 119);
    let first_minute = series.coarse.front().copied();
    let mean = (15 + 30 + 45) / 4;
    assert_eq!(
        first_minute,
        Some(Sample { at: START + 45, cover: Cover::ONE, usage: Amount { cpu: mean, memory: 45 } })
    );
    // A chart has the minutes before the fine tier, then each fine sample, in time.
    let all = times(&series);
    assert_eq!(all.len(), 60 + 240);
    assert!(all.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(all.first(), Some(&45));
    assert_eq!(all.get(60), Some(&3_600));
    assert_eq!(series.len(), 240 + 119);
    assert_eq!((series.fine_from(), series.step()), (Some(START + 3_600), 60), "the tiers for the gaps of a chart");
}

#[test]
fn the_coarse_tier_keeps_24_hours() {
    let mut series = Series::default();
    for minute in 0..(25 * 60u32) {
        series.push(sample(START + minute * 60, 1, 1), &TIERS);
    }
    let newest = START + (25 * 60 - 1) * 60;
    assert_eq!(series.oldest(Tier::Coarse), Some(newest - 86_400 + 60));
    assert_eq!(series.coarse.len(), 24 * 60 - 1);
    assert_eq!(series.fine.len(), 60);
}

#[test]
fn a_step_keeps_the_smallest_cover_and_a_gap_starts_a_new_step() {
    let mut series = Series::default();
    let partial = Sample { at: START + 15, cover: Cover::new(2, 3), usage: Amount::default() };
    series.push(Sample { at: START, cover: Cover::new(3, 3), usage: Amount::default() }, &TIERS);
    series.push(partial, &TIERS);
    // No sample for ten minutes: the next one closes the step, and no step fills the gap.
    series.push(sample(START + 600, 1, 1), &TIERS);
    let steps: Vec<_> = series.coarse.iter().map(|step| (step.at - START, step.cover)).collect();
    assert_eq!(steps, [(15, Cover { sampled: 2, of: 3 })]);
}

#[test]
fn a_drop_removes_the_samples_of_a_tier_up_to_a_time() {
    let mut series = Series::default();
    for second in [0, 15, 30, 45, 60, 75] {
        series.push(sample(START + second, 1, 1), &TIERS);
    }
    assert_eq!(series.drop_through(Tier::Fine, START + 30), 3);
    assert_eq!((series.oldest(Tier::Fine), series.drop_through(Tier::Coarse, START + 45)), (Some(START + 45), 1));
    assert_eq!((series.oldest(Tier::Coarse), series.len()), (None, 3));
}

#[test]
fn a_cover_never_expects_fewer_objects_than_it_has_and_stops_at_its_largest_count() {
    assert_eq!(Cover::new(3, 4), Cover { sampled: 3, of: 4 });
    assert_eq!(Cover::new(5, 2), Cover { sampled: 5, of: 5 });
    assert_eq!(Cover::new(70_000, 80_000), Cover { sampled: u16::MAX, of: u16::MAX });
    assert_eq!(sample(START, 0, 0).time(), SystemTime::UNIX_EPOCH + Duration::from_secs(START.into()));
}
