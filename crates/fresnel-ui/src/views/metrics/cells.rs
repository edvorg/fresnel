//! The CPU and memory cells of a table (research 11 section 8.1): the value with its bar,
//! the age of its sample in the words of the cell, and the mark of an old value or of a
//! cluster whose list fails, with the reason. A missing value is a dash, not zero.

use std::cmp::Ordering;

use super::gauge::{Resource, Usage};
use crate::format::Clock;
use crate::widgets::table::model::{Bar, Cell, Lead};
use crate::widgets::tone::Tone;

/// The text of a cell with no value.
const NONE: &str = "–";
/// The reason of an old value of a list that works.
const NOT_IN_ANSWER: &str = "old: the last answer of metrics-server had no sample of this object";

/// The cell of one resource of a row. `problem` is the state of the list of the cluster
/// in words when it is not ok: each cell of the cluster then has the mark with it.
pub(crate) fn cell<'a>(usage: Option<&Usage>, resource: Resource, problem: Option<&'a str>, clock: &Clock) -> Cell<'a> {
    let Some(usage) = usage else {
        let cell = Cell::new(NONE).tone(Tone::Faint);
        return match problem {
            Some(problem) => cell.lead(Lead::Old(problem)).hint(problem),
            None => cell,
        };
    };
    let gauge = usage.of(resource);
    let mut cell = Cell::new(resource.text(gauge.usage));
    if let Some(part) = gauge.part() {
        cell = cell.bar(Bar { part: part as f32, mark: gauge.mark().map(|mark| mark as f32), tone: gauge.tone() });
    }
    let hint = usage.hint(resource, clock);
    match problem.or((!usage.current).then_some(NOT_IN_ANSWER)) {
        Some(reason) => cell.lead(Lead::Old(reason)).hint(format!("{reason}\n{hint}")).faint(),
        None => cell.hint(hint),
    }
}

/// The sequence of two rows by the value of one resource. A row with no value comes first,
/// as a value of no column does.
pub(crate) fn compare(a: Option<&Usage>, b: Option<&Usage>, resource: Resource) -> Ordering {
    let value = |usage: Option<&Usage>| usage.map(|usage| usage.of(resource).usage);
    value(a).cmp(&value(b))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use fresnel_core::metrics::Cover;

    use super::*;
    use crate::views::metrics::gauge::{Gauge, Whole};

    fn usage(current: bool) -> Usage {
        let gauge = Gauge { usage: 400_000_000, request: Some(500_000_000), most: Some(1_000_000_000) };
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 - 5);
        Usage { cpu: gauge, memory: Gauge::default(), whole: Whole::Limit, at, cover: Cover::ONE, current }
    }

    #[test]
    fn a_value_has_its_bar_and_an_old_value_or_a_failed_list_has_the_mark_with_the_reason() {
        let clock = Clock::fixed(SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000));
        let current = cell(Some(&usage(true)), Resource::Cpu, None, &clock);
        assert_eq!((current.text.as_ref(), current.lead, current.tone), ("400m", Lead::None, Tone::Normal));
        assert_eq!(current.bar, Some(Bar { part: 0.4, mark: Some(0.5), tone: Tone::Info }));
        assert!(current.hint.as_deref().is_some_and(|hint| hint.ends_with("sample of 07:59:55, 5 s ago")));

        let old = cell(Some(&usage(false)), Resource::Cpu, None, &clock);
        assert_eq!(
            (old.lead, old.tone, old.bar.map(|bar| bar.tone)),
            (Lead::Old(NOT_IN_ANSWER), Tone::Faint, Some(Tone::Faint))
        );
        let failed = cell(Some(&usage(true)), Resource::Cpu, Some("metrics-server not found"), &clock);
        assert_eq!(failed.lead, Lead::Old("metrics-server not found"));
        assert!(failed.hint.as_deref().is_some_and(|hint| hint.starts_with("metrics-server not found\nCPU 400m")));

        let none = cell(None, Resource::Memory, None, &clock);
        assert_eq!((none.text.as_ref(), none.lead, none.bar, none.hint), (NONE, Lead::None, None, None));
        let none = cell(None, Resource::Memory, Some("forbidden"), &clock);
        assert_eq!((none.lead, none.hint.as_deref()), (Lead::Old("forbidden"), Some("forbidden")));
        let no_base = Usage { memory: Gauge { usage: 1024, request: None, most: None }, ..usage(true) };
        assert_eq!(cell(Some(&no_base), Resource::Memory, None, &clock).bar, None, "no request and no limit");
    }

    #[test]
    fn a_row_with_no_value_comes_before_each_value() {
        let (small, large) = (usage(true), Usage { cpu: Gauge { usage: 1, ..usage(true).cpu }, ..usage(true) });
        assert_eq!(compare(None, Some(&small), Resource::Cpu), Ordering::Less);
        assert_eq!(compare(Some(&small), Some(&large), Resource::Cpu), Ordering::Greater);
        assert_eq!(compare(None, None, Resource::Memory), Ordering::Equal);
    }
}
