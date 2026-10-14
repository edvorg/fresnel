//! The columns of a kind and the value of one cell.
//!
//! The columns of a kind are the union of the list views of Headlamp and of Lens for that
//! kind (plan section 11.2). A column reads its value from the object alone, so a sort by
//! it never moves a row whose object did not change. Metrics are not columns of a kind:
//! they come from a poll store (M6).

use std::borrow::Cow;
use std::cmp::Ordering;
use std::time::{Duration, SystemTime};

use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

use super::{Describe, Scope};

/// One column of the table of a kind.
pub struct Column<K> {
    /// The title.
    pub title: &'static str,
    /// The width of the column before the user changes it, in points.
    pub width: f32,
    /// True for a column of numbers and times, which align to the right.
    pub numeric: bool,
    /// False for a column that the table hides until the user shows it.
    pub shown: bool,
    /// Reads the value of the cell of one object.
    pub read: for<'a> fn(&'a K) -> Value<'a>,
}

impl<K> Clone for Column<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for Column<K> {}

impl<K> std::fmt::Debug for Column<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Column").field("title", &self.title).finish_non_exhaustive()
    }
}

impl<K> Column<K> {
    /// A column of text.
    pub const fn text(title: &'static str, width: f32, read: for<'a> fn(&'a K) -> Value<'a>) -> Self {
        Self { title, width, numeric: false, shown: true, read }
    }

    /// A column of numbers or times.
    pub const fn number(title: &'static str, width: f32, read: for<'a> fn(&'a K) -> Value<'a>) -> Self {
        Self { title, width, numeric: true, shown: true, read }
    }

    /// The same column, hidden until the user shows it.
    pub const fn hidden(mut self) -> Self {
        self.shown = false;
        self
    }
}

/// Each column of a kind, in its sequence: the name, the namespace of a kind with
/// namespaces, the columns of the kind, the labels (hidden) and the age. A table adds the
/// column of the cluster after the namespace.
pub fn columns<K: Describe>() -> Vec<Column<K>> {
    let name = Column::text("Name", 220.0, |object: &K| Value::text(object.metadata().name.as_deref()));
    let namespace = Column::text("Namespace", 120.0, |object: &K| Value::text(object.metadata().namespace.as_deref()));
    let labels = Column::text("Labels", 200.0, |object: &K| {
        let labels = object.metadata().labels.iter().flatten();
        Value::texts(labels.map(|(key, value)| format!("{key}={value}")))
    });
    let age = Column::number("Age", 84.0, |object: &K| Value::time(object.metadata().creation_timestamp.as_ref()));
    let mut all = vec![name];
    if <K::Scope as Scope>::NAMESPACED {
        all.push(namespace);
    }
    all.extend_from_slice(K::COLUMNS);
    all.extend([labels.hidden(), age]);
    all
}

/// The value of one cell. The sequence of two values of one column is the sequence of a
/// sort by it, and "none" comes first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value<'a> {
    /// The object has no value for the column.
    None,
    /// A text.
    Text(Cow<'a, str>),
    /// A count or an other whole number.
    Number(i64),
    /// A part of a whole, for example 2 of 3 pods that are ready.
    Ratio(i64, i64),
    /// A time of the cluster. A view shows the time since it, so a sort by it puts the
    /// latest time first, as a sort by that age does.
    Time(SystemTime),
    /// A length of time that does not grow, for example the run time of a Job that ended.
    Span(Duration),
    /// Yes or no.
    Flag(bool),
    /// Some texts, for example the images of a pod.
    List(Vec<Cow<'a, str>>),
}

impl<'a> Value<'a> {
    /// The text, or none for no text and for an empty one.
    pub fn text(text: Option<&'a str>) -> Self {
        text.filter(|text| !text.is_empty()).map_or(Self::None, |text| Self::Text(Cow::Borrowed(text)))
    }

    /// A text that a column made, or none for an empty one: the status of a pod with no
    /// status is empty.
    pub fn owned(text: String) -> Self {
        if text.is_empty() { Self::None } else { Self::Text(Cow::Owned(text)) }
    }

    /// The number, or none.
    pub fn number(number: Option<impl Into<i64>>) -> Self {
        number.map_or(Self::None, |number| Self::Number(number.into()))
    }

    /// The time, or none.
    pub fn time(time: Option<&Time>) -> Self {
        time.map_or(Self::None, |time| Self::Time(SystemTime::from(time.0)))
    }

    /// The texts, or none when there is no text.
    pub fn list(texts: impl IntoIterator<Item = &'a str>) -> Self {
        let texts: Vec<Cow<'a, str>> = texts.into_iter().map(Cow::Borrowed).collect();
        if texts.is_empty() { Self::None } else { Self::List(texts) }
    }

    /// The texts that a column made, or none when there is no text.
    pub fn texts(texts: impl IntoIterator<Item = String>) -> Self {
        let texts: Vec<Cow<'a, str>> = texts.into_iter().map(Cow::Owned).collect();
        if texts.is_empty() { Self::None } else { Self::List(texts) }
    }

    /// The same value with no borrow of its object. A table computes the value of each
    /// cell one time, on its worker thread, and keeps it with the row.
    pub fn into_owned(self) -> Value<'static> {
        let own = |text: Cow<'_, str>| Cow::Owned(text.into_owned());
        match self {
            Self::None => Value::None,
            Self::Text(text) => Value::Text(own(text)),
            Self::Number(number) => Value::Number(number),
            Self::Ratio(part, whole) => Value::Ratio(part, whole),
            Self::Time(time) => Value::Time(time),
            Self::Span(span) => Value::Span(span),
            Self::Flag(flag) => Value::Flag(flag),
            Self::List(texts) => Value::List(texts.into_iter().map(own).collect()),
        }
    }

    /// The place of the variant in a sort of values of different variants.
    fn rank(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::Text(_) => 1,
            Self::Number(_) => 2,
            Self::Ratio(..) => 3,
            Self::Time(_) => 4,
            Self::Span(_) => 5,
            Self::Flag(_) => 6,
            Self::List(_) => 7,
        }
    }
}

impl Ord for Value<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::Ratio(a, of_a), Self::Ratio(b, of_b)) => (a, of_a).cmp(&(b, of_b)),
            (Self::Time(a), Self::Time(b)) => b.cmp(a),
            (Self::Span(a), Self::Span(b)) => a.cmp(b),
            (Self::Flag(a), Self::Flag(b)) => a.cmp(b),
            (Self::List(a), Self::List(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for Value<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::{ConfigMap, Node};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use super::*;

    fn titles<K: Describe>() -> Vec<&'static str> {
        columns::<K>().iter().map(|column| column.title).collect()
    }

    #[test]
    fn each_kind_has_the_name_the_labels_and_the_age_and_a_kind_with_namespaces_the_namespace() {
        assert_eq!(titles::<ConfigMap>(), ["Name", "Namespace", "Keys", "Labels", "Age"]);
        let node = titles::<Node>();
        assert_eq!((node.first(), node.get(1)), (Some(&"Name"), Some(&"Status")), "a node has no namespace");
        let config = ConfigMap {
            metadata: ObjectMeta {
                name: Some("web".into()),
                labels: Some([("b".into(), "2".into()), ("a".into(), "1".into())].into()),
                ..ObjectMeta::default()
            },
            ..ConfigMap::default()
        };
        let values: Vec<Value<'_>> = columns::<ConfigMap>().iter().map(|column| (column.read)(&config)).collect();
        let list = |texts: &[&'static str]| Value::List(texts.iter().map(|text| Cow::Borrowed(*text)).collect());
        assert_eq!(
            values,
            [Value::text(Some("web")), Value::None, Value::None, list(&["a=1", "b=2"]), Value::None],
            "an absent value and an empty one are none"
        );
        assert!(!columns::<ConfigMap>().iter().any(|column| column.title == "Labels" && column.shown));
        let empty = ConfigMap {
            metadata: ObjectMeta { labels: Some([].into()), ..ObjectMeta::default() },
            data: Some([].into()),
            ..ConfigMap::default()
        };
        let empty: Vec<Value<'_>> = columns::<ConfigMap>().iter().map(|column| (column.read)(&empty)).collect();
        assert_eq!(empty, [Value::None, Value::None, Value::None, Value::None, Value::None]);
    }

    #[test]
    fn none_sorts_first_and_a_later_time_sorts_before_an_earlier_one() {
        let early = SystemTime::UNIX_EPOCH;
        let late = early + Duration::from_secs(60);
        let mut values = [Value::Time(early), Value::None, Value::Time(late)];
        values.sort();
        assert_eq!(values, [Value::None, Value::Time(late), Value::Time(early)], "the youngest object first");
        assert!(Value::Ratio(1, 3) < Value::Ratio(2, 2) && Value::Ratio(2, 2) < Value::Ratio(2, 3));
        assert!(Value::text(Some("a")) < Value::Number(0), "the variants of one column have one sequence");
        assert_eq!((Value::text(Some("")), Value::list([])), (Value::None, Value::None));
    }

    #[test]
    fn an_owned_value_is_equal_to_the_value_that_borrows_its_object() {
        let others = [
            Value::Ratio(1, 2),
            Value::Time(SystemTime::UNIX_EPOCH),
            Value::Span(Duration::from_secs(5)),
            Value::Flag(true),
            Value::Number(-3),
            Value::None,
        ];
        let owned: Vec<Value<'static>> = {
            let text = String::from("web");
            let borrowed = [Value::text(Some(&text)), Value::list([text.as_str(), "db"])];
            borrowed.into_iter().chain(others.iter().cloned()).map(Value::into_owned).collect()
        };
        let list = Value::List(vec![Cow::Borrowed("web"), Cow::Borrowed("db")]);
        assert_eq!(owned[..2], [Value::text(Some("web")), list], "the object is gone, and the values stay");
        assert_eq!(owned[2..], others);
    }
}
