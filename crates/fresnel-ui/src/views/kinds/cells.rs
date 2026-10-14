//! The cells of the table of a kind: the text and the tone of each value of a column.

use std::borrow::Cow;

use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::{ObjectHealth, Value};

use crate::format::{self, Clock};
use crate::views::objects::RowScene;
use crate::widgets::table::model::{Cell, Lead};
use crate::widgets::tone::Tone;

/// The text of a cell with no value.
const NONE: &str = "–";

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
        Value::List(texts) => (Cow::Owned(texts.join(", ")), Tone::Normal),
    };
    Cell::new(text).tone(tone)
}

/// An old row has no color that tells a state of now.
pub(super) fn of_old_row<'a>(cell: Cell<'a>, old: Option<&str>) -> Cell<'a> {
    if old.is_some() { cell.tone(Tone::Faint) } else { cell }
}
