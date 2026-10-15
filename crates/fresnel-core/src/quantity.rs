//! The number of a resource quantity of the API: `100m` CPU, `8Gi` of memory, `20` pods.
//!
//! A view compares two quantities, for example the use of a ResourceQuota with its hard
//! limit. The text of a quantity is the text of the server, so a view shows that text and
//! uses the number only for the comparison.

/// The number of a quantity, with the suffixes of the API: the binary ones (`Ki` to `Ei`),
/// the decimal ones (`n` to `E`) and an exponent (`1e3`). `None` for a text that is not a
/// quantity.
pub fn number(text: &str) -> Option<f64> {
    let text = text.trim();
    let end = text.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '+' | '-'))).unwrap_or(text.len());
    let (number, suffix) = text.split_at_checked(end)?;
    let number: f64 = number.parse().ok()?;
    let factor = match suffix {
        "" => 1.0,
        "n" => 1e-9,
        "u" => 1e-6,
        "m" => 1e-3,
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        "Ki" => 1024_f64,
        "Mi" => 1024_f64.powi(2),
        "Gi" => 1024_f64.powi(3),
        "Ti" => 1024_f64.powi(4),
        "Pi" => 1024_f64.powi(5),
        "Ei" => 1024_f64.powi(6),
        exponent => 10_f64.powi(exponent.strip_prefix(['e', 'E'])?.parse().ok()?),
    };
    Some(number * factor).filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_form_of_a_quantity_gives_its_number_and_another_text_gives_none() {
        let read = |text: &str| number(text);
        assert_eq!(read("20"), Some(20.0));
        assert_eq!(read("100m"), Some(0.1));
        assert_eq!(read("1.5"), Some(1.5));
        assert_eq!(read("64Mi"), Some(64.0 * 1024.0 * 1024.0));
        assert_eq!(read("8Gi"), Some(8.0 * 1024.0 * 1024.0 * 1024.0));
        assert_eq!(read("2k"), Some(2000.0));
        assert_eq!(read("1e3"), Some(1000.0));
        assert_eq!(read("1E3"), Some(1000.0), "an exponent, not exa");
        assert_eq!(read("1E"), Some(1e18), "exa");
        assert_eq!(read("250000n"), Some(0.00025));
        assert_eq!(read("-1"), Some(-1.0));
        assert_eq!(read(" 3 "), Some(3.0));
        for text in ["", "Gi", "1Xi", "1e", "1e1.5", "abc", "1..2", "1e999"] {
            assert_eq!(read(text), None, "{text}");
        }
    }
}
