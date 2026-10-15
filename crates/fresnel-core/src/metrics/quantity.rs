//! The reader of a Kubernetes quantity, for example `250m`, `1.5Gi` or `129e6`.
//!
//! An own reader, not a crate (research 11 section 7.3): `kube_quantity` 0.9.0 reads `5E`
//! and `1Ei` as numbers with no exponent, and `k8s-metrics` reads only `Ki`, `Mi` and `Gi`.
//! The reader computes with whole numbers, so `6082165n` stays exact.
//!
//! The grammar of the API (`k8s.io/apimachinery/pkg/api/resource`):
//! `<number><suffix>`, where the suffix is a binary one (`Ki` to `Ei`), a decimal one (`n`,
//! `u`, `m`, none, `k`, `M` to `E`) or an exponent (`e3`, `E-3`).

/// The most significant digits of a number: the reader computes in `u128`.
const MAX_DIGITS: usize = 38;

/// CPU in nanocores: `1` is 1,000,000,000, `250m` is 250,000,000.
pub(crate) fn nanocores(text: &str) -> Result<u64, &'static str> {
    scaled(text, -9)
}

/// Memory in bytes: `1Ki` is 1,024, `1k` is 1,000.
pub(crate) fn bytes(text: &str) -> Result<u64, &'static str> {
    scaled(text, 0)
}

/// A quantity as an exact number: `digits * 10^exponent * 2^binary`.
#[derive(Debug, PartialEq, Eq)]
struct Exact {
    digits: u128,
    exponent: i32,
    binary: u32,
}

/// The quantity in units of `10^unit`, rounded up as the API does it (`Quantity.Value`).
fn scaled(text: &str, unit: i32) -> Result<u64, &'static str> {
    let Exact { digits, exponent, binary } = exact(text)?;
    let value = digits.checked_mul(1u128.checked_shl(binary).ok_or("too large")?).ok_or("too large")?;
    let shift = exponent - unit;
    let ten = |power: i32| 10u128.checked_pow(power.unsigned_abs());
    let units = if shift >= 0 {
        ten(shift).and_then(|factor| value.checked_mul(factor))
    } else {
        // A divisor larger than `u128` gives a value below one unit: one unit, rounded up.
        Some(ten(shift).map_or(u128::from(value > 0), |divisor| value.div_ceil(divisor)))
    };
    units.and_then(|units| u64::try_from(units).ok()).ok_or("too large")
}

/// Reads the text of a quantity. A sign `-` is an error: no usage, request or limit is
/// below zero.
fn exact(text: &str) -> Result<Exact, &'static str> {
    let unsigned = text.strip_prefix('+').unwrap_or(text);
    if unsigned.starts_with('-') {
        return Err("below zero");
    }
    let end = unsigned.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(unsigned.len());
    let (number, suffix) = unsigned.split_at(end);
    let (digits, fraction) = decimal(number)?;
    let (exponent, binary) = suffix_of(suffix)?;
    Ok(Exact { digits, exponent: exponent - fraction, binary })
}

/// The digits of a number with at most one point, and the count of digits after the point.
fn decimal(number: &str) -> Result<(u128, i32), &'static str> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() && fraction.is_empty() || fraction.contains('.') {
        return Err("no number");
    }
    let all = whole.chars().chain(fraction.chars());
    let significant: String = all.skip_while(|c| *c == '0').collect();
    if significant.len() > MAX_DIGITS {
        return Err("too many digits");
    }
    let digits = if significant.is_empty() { 0 } else { significant.parse().map_err(|_| "no number")? };
    let places = i32::try_from(fraction.len()).map_err(|_| "too many digits")?;
    Ok((digits, places))
}

/// The power of ten and the power of two of a suffix.
fn suffix_of(suffix: &str) -> Result<(i32, u32), &'static str> {
    let decimal = |exponent| Ok((exponent, 0));
    let binary = |power| Ok((0, power));
    match suffix {
        "n" => decimal(-9),
        "u" => decimal(-6),
        "m" => decimal(-3),
        "" => decimal(0),
        "k" => decimal(3),
        "M" => decimal(6),
        "G" => decimal(9),
        "T" => decimal(12),
        "P" => decimal(15),
        // `E` alone is exa. With a number after it, it is an exponent.
        "E" => decimal(18),
        "Ki" => binary(10),
        "Mi" => binary(20),
        "Gi" => binary(30),
        "Ti" => binary(40),
        "Pi" => binary(50),
        "Ei" => binary(60),
        other => {
            let power = other.strip_prefix(['e', 'E']).ok_or("an unknown suffix")?;
            let digits = power.strip_prefix(['+', '-']).unwrap_or(power);
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("an unknown suffix");
            }
            decimal(power.parse().map_err(|_| "too large")?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strings of the test crate of research 11 (`$PROBE`, test `quantities`), with
    /// the value of the API.
    #[test]
    fn each_form_of_the_api_gives_its_exact_value() {
        let cpu = [
            ("6082165n", 6_082_165),
            ("250m", 250_000_000),
            ("1", 1_000_000_000),
            ("0.5", 500_000_000),
            ("1500u", 1_500_000),
            ("1k", 1_000_000_000_000),
            ("1e3", 1_000_000_000_000),
            ("1E3", 1_000_000_000_000),
            ("1e-3", 1_000_000),
            ("11973899059200m", 11_973_899_059_200_000_000),
            ("+2", 2_000_000_000),
            (".5", 500_000_000),
            ("5.", 5_000_000_000),
            ("0", 0),
            ("0.0m", 0),
        ];
        for (text, value) in cpu {
            assert_eq!(nanocores(text), Ok(value), "{text}");
        }
        let memory = [
            ("22272Ki", 22_806_528),
            ("512Mi", 536_870_912),
            ("1.5Gi", 1_610_612_736),
            ("2Ti", 2_199_023_255_552),
            ("1Pi", 1_125_899_906_842_624),
            ("1Ei", 1_152_921_504_606_846_976),
            ("1G", 1_000_000_000),
            ("500M", 500_000_000),
            ("129e6", 129_000_000),
            ("5E", 5_000_000_000_000_000_000),
            ("1P", 1_000_000_000_000_000),
            ("23855104", 23_855_104),
        ];
        for (text, value) in memory {
            assert_eq!(bytes(text), Ok(value), "{text}");
        }
    }

    #[test]
    fn a_part_of_a_unit_rounds_up_as_in_the_api() {
        assert_eq!(bytes("1.5"), Ok(2));
        assert_eq!(bytes("100m"), Ok(1));
        assert_eq!(nanocores("0.1n"), Ok(1));
        assert_eq!(nanocores("1e-30"), Ok(1), "a part of one nanocore");
        assert_eq!(bytes("0.000000000000000000000000000000000000000000001"), Ok(1), "a divisor above `u128`");
    }

    #[test]
    fn a_text_that_is_no_quantity_gives_its_reason() {
        let cases = [
            ("", "no number"),
            ("Mi", "no number"),
            (".", "no number"),
            ("1.2.3", "no number"),
            ("-1", "below zero"),
            ("+-1", "below zero"),
            ("1Xi", "an unknown suffix"),
            ("1 Mi", "an unknown suffix"),
            ("1e", "an unknown suffix"),
            ("1e3.5", "an unknown suffix"),
            ("1mi", "an unknown suffix"),
            ("16Ei", "too large"),
            ("20E", "too large"),
            ("1e99999999999", "too large"),
            ("123456789012345678901234567890123456789", "too many digits"),
        ];
        for (text, reason) in cases {
            assert_eq!(bytes(text), Err(reason), "{text:?}");
        }
        // 5 exa cores have more nanocores than `u64` holds; 5 exabytes fit.
        assert_eq!(nanocores("5E"), Err("too large"));
        assert_eq!(bytes("000000000000000000000000000000000000000001"), Ok(1), "leading zeros are no digits");
    }
}
