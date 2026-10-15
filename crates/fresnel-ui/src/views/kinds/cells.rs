//! The cells of the table of a kind: the text and the tone of each value of a column.

use std::borrow::Cow;

use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::{ObjectHealth, Value};

use crate::format::{self, Clock};
use crate::views::objects::RowScene;
use crate::widgets::table::cells::TEXT_LIMIT;
use crate::widgets::table::model::{Cell, Lead};
use crate::widgets::tone::Tone;

/// The text of a cell with no value.
const NONE: &str = "–";
/// The most bytes of a list that a cell joins. A character has 4 bytes at most, so the
/// text has more characters than the table lays out, and the table cuts it with its mark.
const LIST_BYTES: usize = 4 * TEXT_LIMIT;

/// The name of an object: the freshness mark of a row from a store that is not live, and
/// else a dot for an object that is not healthy. A healthy object has no dot, so the dots
/// stand out.
pub(super) fn name<'a>(name: &'a str, health: ObjectHealth, old: Option<&'a str>, scene: &RowScene) -> Cell<'a> {
    let lead = match old {
        Some(since) => Lead::Old(since),
        None if health == ObjectHealth::Healthy => Lead::None,
        None => Lead::Dot(Tone::of_object(health).color(scene.palette)),
    };
    Cell::new(name).lead(lead)
}

/// The cluster of a row, with its color.
pub(super) fn cluster<'a>(cluster: &'a ClusterId, scene: &RowScene) -> Cell<'a> {
    Cell::new(cluster.as_str()).lead(Lead::Dot(scene.color(cluster)))
}

/// The text of a value. A time shows as the time since it, as the age of `kubectl get`.
pub(super) fn value<'a>(value: &'a Value<'static>, namespace: bool, clock: &Clock) -> Cell<'a> {
    let (text, tone): (Cow<'a, str>, Tone) = match value {
        Value::None => (Cow::Borrowed(NONE), Tone::Faint),
        Value::Text(text) => (Cow::Borrowed(text), if namespace { Tone::Muted } else { Tone::Normal }),
        Value::Number(number) => (Cow::Owned(number.to_string()), Tone::Normal),
        Value::Ratio(part, whole) => (Cow::Owned(format!("{part}/{whole}")), Tone::Normal),
        Value::Time(time) => (Cow::Owned(format::span(clock.since(*time))), Tone::Muted),
        Value::Span(span) => (Cow::Owned(format::span(*span)), Tone::Muted),
        Value::Flag(flag) => (Cow::Borrowed(if *flag { "Yes" } else { "No" }), Tone::Normal),
        Value::List(texts) => (Cow::Owned(joined(texts)), Tone::Normal),
    };
    Cell::new(text).tone(tone)
}

/// The texts of a list with a comma between two of them, up to the place that the table
/// shows. A frame joins the list of each row on the screen, and a list can have thousands
/// of texts: the keys of a ConfigMap, or the addresses of an EndpointSlice.
fn joined(texts: &[Cow<'static, str>]) -> String {
    let mut text = String::new();
    for (place, item) in texts.iter().enumerate() {
        if text.len() > LIST_BYTES {
            break;
        }
        if place > 0 {
            text.push_str(", ");
        }
        text.push_str(item);
    }
    text
}

/// An old row has no color that tells a state of now.
pub(super) fn of_old_row<'a>(cell: Cell<'a>, old: Option<&str>) -> Cell<'a> {
    if old.is_some() { cell.faint() } else { cell }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::short;

    #[test]
    fn a_long_list_joins_only_the_texts_that_the_table_shows_and_keeps_its_cut_mark() {
        let short_list = [Cow::Borrowed("web"), Cow::Borrowed("db")];
        assert_eq!(joined(&short_list), "web, db");
        let keys: Vec<Cow<'static, str>> = (0..100_000).map(|key| Cow::Owned(format!("key-{key:06}"))).collect();
        let text = joined(&keys);
        assert!(text.starts_with("key-000000, key-000001") && text.len() <= LIST_BYTES + 12, "{}", text.len());
        assert!(short(&text, TEXT_LIMIT).ends_with('…'), "the table cuts the text with its mark");
        let wide: Vec<Cow<'static, str>> = (0..1000).map(|_| Cow::Borrowed("名前")).collect();
        assert!(joined(&wide).chars().count() > TEXT_LIMIT, "characters of 3 bytes still fill the cell");
    }
}
