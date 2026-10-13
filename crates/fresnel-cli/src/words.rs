//! Small pieces of text that each command uses.

/// A count with its noun: "1 context", "3 contexts".
pub(crate) fn count(number: impl Into<u64>, noun: &str) -> String {
    match number.into() {
        1 => format!("1 {noun}"),
        number => format!("{number} {noun}s"),
    }
}

/// The same for the length of a list.
pub(crate) fn size(length: usize, noun: &str) -> String {
    count(u64::try_from(length).unwrap_or(u64::MAX), noun)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_has_the_form_of_its_noun_for_one_and_for_many() {
        assert_eq!(
            (size(0, "store"), size(1, "store"), count(2_u32, "store")),
            ("0 stores".into(), "1 store".into(), "2 stores".into())
        );
    }
}
