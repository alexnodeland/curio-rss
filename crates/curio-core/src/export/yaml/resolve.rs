//! Plain-scalar resolution: which untagged plain scalars read back as
//! null, a boolean, an integer or a float rather than a string.
//!
//! Both directions share these rules. The deserializer resolves plain
//! scalars with them, and the serializer quotes any string they would
//! not read back as a string. They are the `serde_yaml` 0.9 rules, kept
//! exactly: YAML 1.2 core-schema spellings (`yes`/`on` stay strings),
//! `0x`/`0o`/`0b` radix prefixes, and leading-zero digit runs (`007`)
//! stay strings.

use std::num::ParseIntError;

/// What an untagged plain scalar resolves to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Resolved<'a> {
    /// Empty, `~`, `null`, `Null` or `NULL`.
    Null,
    /// `true`/`True`/`TRUE`, `false`/`False`/`FALSE`.
    Bool(bool),
    /// A non-negative integer that fits `u64`.
    U64(u64),
    /// A negative integer that fits `i64`.
    I64(i64),
    /// A non-negative integer that fits `u128` but not `u64`.
    U128(u128),
    /// A negative integer that fits `i128` but not `i64`.
    I128(i128),
    /// A float (finite, or one of the `.inf`/`.nan` spellings).
    F64(f64),
    /// Anything else: the scalar is a string.
    Str(&'a str),
}

pub(super) fn parse_null(scalar: &str) -> bool {
    matches!(scalar, "null" | "Null" | "NULL" | "~")
}

pub(super) fn parse_bool(scalar: &str) -> Option<bool> {
    match scalar {
        "true" | "True" | "TRUE" => Some(true),
        "false" | "False" | "FALSE" => Some(false),
        _ => None,
    }
}

fn starts_with_sign(s: &str) -> bool {
    s.starts_with(['+', '-'])
}

/// A negative radix-prefixed (`-0x`/`-0o`/`-0b`) integer.
fn negative_radix<T>(
    scalar: &str,
    from_str_radix: fn(&str, u32) -> Result<T, ParseIntError>,
) -> Option<T> {
    for (prefix, radix) in [("-0x", 16), ("-0o", 8), ("-0b", 2)] {
        if let Some(rest) = scalar.strip_prefix(prefix)
            && let Ok(int) = from_str_radix(&format!("-{rest}"), radix)
        {
            return Some(int);
        }
    }
    None
}

pub(super) fn parse_unsigned_int<T>(
    scalar: &str,
    from_str_radix: fn(&str, u32) -> Result<T, ParseIntError>,
) -> Option<T> {
    let unpositive = scalar.strip_prefix('+').unwrap_or(scalar);
    // The radix prefixes are tried in turn; a sign after one ends the
    // attempt outright (`0x+1` is a string, not 1).
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(rest) = unpositive.strip_prefix(prefix) {
            if starts_with_sign(rest) {
                return None;
            }
            if let Ok(int) = from_str_radix(rest, radix) {
                return Some(int);
            }
        }
    }
    if starts_with_sign(unpositive) || digits_but_not_number(scalar) {
        return None;
    }
    from_str_radix(unpositive, 10).ok()
}

pub(super) fn parse_signed_int<T>(
    scalar: &str,
    from_str_radix: fn(&str, u32) -> Result<T, ParseIntError>,
) -> Option<T> {
    let unpositive = if let Some(unpositive) = scalar.strip_prefix('+') {
        if starts_with_sign(unpositive) {
            return None;
        }
        unpositive
    } else {
        scalar
    };
    // Per radix: the positive form first, then the negative one.
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(rest) = unpositive.strip_prefix(prefix) {
            if starts_with_sign(rest) {
                return None;
            }
            if let Ok(int) = from_str_radix(rest, radix) {
                return Some(int);
            }
        }
        if let Some(rest) = scalar.strip_prefix(&format!("-{prefix}"))
            && let Ok(int) = from_str_radix(&format!("-{rest}"), radix)
        {
            return Some(int);
        }
    }
    if digits_but_not_number(scalar) {
        return None;
    }
    from_str_radix(unpositive, 10).ok()
}

fn parse_negative_int<T>(
    scalar: &str,
    from_str_radix: fn(&str, u32) -> Result<T, ParseIntError>,
) -> Option<T> {
    if let Some(int) = negative_radix(scalar, from_str_radix) {
        return Some(int);
    }
    if digits_but_not_number(scalar) {
        return None;
    }
    from_str_radix(scalar, 10).ok()
}

pub(super) fn parse_f64(scalar: &str) -> Option<f64> {
    let unpositive = if let Some(unpositive) = scalar.strip_prefix('+') {
        if starts_with_sign(unpositive) {
            return None;
        }
        unpositive
    } else {
        scalar
    };
    if let ".inf" | ".Inf" | ".INF" = unpositive {
        return Some(f64::INFINITY);
    }
    if let "-.inf" | "-.Inf" | "-.INF" = scalar {
        return Some(f64::NEG_INFINITY);
    }
    if let ".nan" | ".NaN" | ".NAN" = scalar {
        return Some(f64::NAN.copysign(1.0));
    }
    unpositive
        .parse::<f64>()
        .ok()
        .filter(|float| float.is_finite())
}

/// Leading zero(s) followed by digits is a string per YAML 1.2 (`007`).
pub(super) fn digits_but_not_number(scalar: &str) -> bool {
    let scalar = scalar.strip_prefix(['-', '+']).unwrap_or(scalar);
    scalar.len() > 1 && scalar.starts_with('0') && scalar[1..].bytes().all(|b| b.is_ascii_digit())
}

/// The integer an untagged plain scalar spells, widest last.
pub(super) fn resolve_int(scalar: &str) -> Option<Resolved<'_>> {
    if let Some(int) = parse_unsigned_int(scalar, u64::from_str_radix) {
        return Some(Resolved::U64(int));
    }
    if let Some(int) = parse_negative_int(scalar, i64::from_str_radix) {
        return Some(Resolved::I64(int));
    }
    if let Some(int) = parse_unsigned_int(scalar, u128::from_str_radix) {
        return Some(Resolved::U128(int));
    }
    if let Some(int) = parse_negative_int(scalar, i128::from_str_radix) {
        return Some(Resolved::I128(int));
    }
    None
}

/// Resolves an untagged plain scalar.
pub(super) fn resolve_plain(scalar: &str) -> Resolved<'_> {
    if scalar.is_empty() || parse_null(scalar) {
        return Resolved::Null;
    }
    if let Some(boolean) = parse_bool(scalar) {
        return Resolved::Bool(boolean);
    }
    if let Some(int) = resolve_int(scalar) {
        return int;
    }
    if !digits_but_not_number(scalar)
        && let Some(float) = parse_f64(scalar)
    {
        return Resolved::F64(float);
    }
    Resolved::Str(scalar)
}

/// Whether a string written as a plain scalar would read back as
/// something else (or is a digit run YAML 1.1 readers take as octal) —
/// such strings are single-quoted.
pub(super) fn needs_quotes(value: &str) -> bool {
    digits_but_not_number(value) || !matches!(resolve_plain(value), Resolved::Str(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_scalars_resolve_like_the_yaml_1_2_core_schema() {
        assert_eq!(resolve_plain(""), Resolved::Null);
        assert_eq!(resolve_plain("~"), Resolved::Null);
        assert_eq!(resolve_plain("NULL"), Resolved::Null);
        assert_eq!(resolve_plain("True"), Resolved::Bool(true));
        assert_eq!(resolve_plain("yes"), Resolved::Str("yes"));
        assert_eq!(resolve_plain("+12"), Resolved::U64(12));
        assert_eq!(resolve_plain("0x1F"), Resolved::U64(31));
        assert_eq!(resolve_plain("-0x10"), Resolved::I64(-16));
        assert_eq!(resolve_plain("0o17"), Resolved::U64(15));
        assert_eq!(resolve_plain("0b101"), Resolved::U64(5));
        assert_eq!(resolve_plain("0x+1"), Resolved::Str("0x+1"));
        assert_eq!(resolve_plain("++5"), Resolved::Str("++5"));
        assert_eq!(resolve_plain("007"), Resolved::Str("007"));
        assert_eq!(resolve_plain("-007"), Resolved::Str("-007"));
        assert_eq!(
            resolve_plain("18446744073709551616"),
            Resolved::U128(18_446_744_073_709_551_616)
        );
        assert_eq!(
            resolve_plain("-9223372036854775809"),
            Resolved::I128(-9_223_372_036_854_775_809)
        );
        assert_eq!(resolve_plain("1e3"), Resolved::F64(1000.0));
        assert_eq!(resolve_plain("-.Inf"), Resolved::F64(f64::NEG_INFINITY));
        assert!(matches!(resolve_plain(".NaN"), Resolved::F64(f) if f.is_nan()));
        assert_eq!(resolve_plain("inf"), Resolved::Str("inf"));
        assert_eq!(resolve_plain("1_000"), Resolved::Str("1_000"));
    }

    #[test]
    fn typed_integer_parsers_follow_the_same_spellings() {
        assert_eq!(parse_signed_int("+0x10", i64::from_str_radix), Some(16));
        assert_eq!(parse_signed_int("-0b11", i64::from_str_radix), Some(-3));
        assert_eq!(parse_signed_int("-0o7", i64::from_str_radix), Some(-7));
        assert_eq!(parse_signed_int("+-1", i64::from_str_radix), None);
        assert_eq!(parse_signed_int("0x-1", i64::from_str_radix), None);
        assert_eq!(parse_signed_int("-007", i64::from_str_radix), None);
        assert_eq!(parse_signed_int("-12", i64::from_str_radix), Some(-12));
        assert_eq!(parse_unsigned_int("-1", u64::from_str_radix), None);
        assert_eq!(parse_unsigned_int("0b+1", u64::from_str_radix), None);
        assert_eq!(parse_f64("+-1.0"), None);
    }

    #[test]
    fn strings_that_would_resolve_otherwise_need_quotes() {
        for quoted in ["", "null", "true", "42", "-7", "1.5", ".inf", "007", "0x1F"] {
            assert!(needs_quotes(quoted), "{quoted:?}");
        }
        for plain in ["yes", "inf", "0x", "1_000", "hello", "2026-07-01"] {
            assert!(!needs_quotes(plain), "{plain:?}");
        }
    }
}
