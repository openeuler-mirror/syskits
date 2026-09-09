/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

mod extendedbigdecimal;
mod long_double_format;
mod number;
mod numberparse;

pub use extendedbigdecimal::ExtendedBigDecimal;
pub use long_double_format::{
    GnuFloatFormat, GnuFormatError, long_double_linear_value, overflows_long_double,
    quantize_long_double,
};
pub use number::PreciseNumber;
pub use numberparse::ParseNumberError;

use super::num_parser::{ParseError, ParsedNumber};
use long_double_format::{current_decimal_point, underflows_long_double};

#[derive(Debug, PartialEq)]
pub(super) enum LongDoubleParseError<'a> {
    NotNumeric,
    PartialMatch(ExtendedBigDecimal, &'a str),
    OutOfRange(ExtendedBigDecimal),
}

pub(super) fn parse_long_double(
    input: &str,
) -> Result<ExtendedBigDecimal, LongDoubleParseError<'_>> {
    if input.is_empty() {
        return Ok(ExtendedBigDecimal::default());
    }

    let Some(attempt) = select_parse_attempt(input, &current_decimal_point()) else {
        return Err(LongDoubleParseError::NotNumeric);
    };
    let rest = &input[attempt.consumed..];
    match parse_quantized_long_double(&attempt.normalized_prefix) {
        Ok(value) if rest.is_empty() => Ok(value),
        Ok(value) => Err(LongDoubleParseError::PartialMatch(value, rest)),
        Err(LongDoubleParseError::OutOfRange(value)) => {
            Err(LongDoubleParseError::OutOfRange(value))
        }
        Err(_) => Err(LongDoubleParseError::NotNumeric),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParseAttempt {
    normalized_prefix: String,
    consumed: usize,
}

fn select_parse_attempt(input: &str, decimal_point: &str) -> Option<ParseAttempt> {
    let locale_attempt = parse_attempt(input, decimal_point, true);
    if locale_attempt
        .as_ref()
        .is_some_and(|attempt| attempt.consumed == input.len())
    {
        return locale_attempt;
    }

    let c_attempt = parse_attempt(input, ".", false);
    match (locale_attempt, c_attempt) {
        (Some(locale), Some(c)) if c.consumed > locale.consumed => Some(c),
        (Some(locale), _) => Some(locale),
        (None, c) => c,
    }
}

fn parse_attempt(input: &str, decimal_point: &str, locale: bool) -> Option<ParseAttempt> {
    let (normalized, offsets) = normalize_decimal_point(input, decimal_point, locale);
    let normalized_consumed = match ParsedNumber::parse_f64(&normalized) {
        Ok(_) => normalized.len(),
        Err(ParseError::CtPartialMatch(_, rest)) => normalized.len() - rest.len(),
        Err(ParseError::CtNotNumeric | ParseError::CtOverflow) => 0,
    };
    if normalized_consumed == 0 {
        return None;
    }

    Some(ParseAttempt {
        normalized_prefix: normalized[..normalized_consumed].to_string(),
        consumed: offsets[normalized_consumed],
    })
}

fn normalize_decimal_point(input: &str, decimal_point: &str, locale: bool) -> (String, Vec<usize>) {
    let mut normalized = String::with_capacity(input.len());
    let mut offsets = Vec::with_capacity(input.len() + 1);
    offsets.push(0);
    let mut index = 0;

    while index < input.len() {
        if !decimal_point.is_empty() && input[index..].starts_with(decimal_point) {
            normalized.push('.');
            index += decimal_point.len();
            offsets.push(index);
        } else if locale && decimal_point != "." && input.as_bytes()[index] == b'.' {
            normalized.push('\u{1}');
            index += 1;
            offsets.push(index);
        } else {
            let character = input[index..]
                .chars()
                .next()
                .expect("index is before the end of a UTF-8 string");
            normalized.push(character);
            for byte in 1..=character.len_utf8() {
                offsets.push(index + byte);
            }
            index += character.len_utf8();
        }
    }

    (normalized, offsets)
}

pub(super) fn long_double_from_f64(value: f64) -> ExtendedBigDecimal {
    if value.is_nan() {
        if value.is_sign_negative() {
            ExtendedBigDecimal::MinusNan
        } else {
            ExtendedBigDecimal::Nan
        }
    } else if value == f64::INFINITY {
        ExtendedBigDecimal::Infinity
    } else if value == f64::NEG_INFINITY {
        ExtendedBigDecimal::MinusInfinity
    } else {
        parse_complete_long_double(&value.to_string())
            .map(|parsed| parsed.value)
            .unwrap_or_default()
    }
}

struct ParsedLongDouble {
    value: ExtendedBigDecimal,
    range_error: bool,
}

fn parse_quantized_long_double(
    input: &str,
) -> Result<ExtendedBigDecimal, LongDoubleParseError<'_>> {
    let parsed = parse_complete_long_double(input).map_err(|_| LongDoubleParseError::NotNumeric)?;
    let range_error = parsed.range_error
        || overflows_long_double(&parsed.value)
        || underflows_long_double(&parsed.value);
    let value = quantize_long_double(&parsed.value);
    if range_error {
        Err(LongDoubleParseError::OutOfRange(value))
    } else {
        Ok(value)
    }
}

fn parse_complete_long_double(input: &str) -> Result<ParsedLongDouble, ParseNumberError> {
    let trimmed = input.trim_start_matches(char::is_whitespace);
    if let Some(rest) = trimmed.strip_prefix(['\'', '"']) {
        return rest
            .chars()
            .next()
            .map(|value| ExtendedBigDecimal::from_u64(u64::from(value)))
            .map(|value| ParsedLongDouble {
                value,
                range_error: false,
            })
            .ok_or(ParseNumberError::Float);
    }

    let (negative, unsigned) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    if unsigned.to_ascii_lowercase().starts_with("nan") {
        return Ok(ParsedLongDouble {
            value: if negative {
                ExtendedBigDecimal::MinusNan
            } else {
                ExtendedBigDecimal::Nan
            },
            range_error: false,
        });
    }

    if let Some(parsed) = classify_extreme_decimal(trimmed) {
        return Ok(parsed);
    }

    input
        .parse::<PreciseNumber>()
        .map(|number| ParsedLongDouble {
            value: number.number,
            range_error: false,
        })
}

fn classify_extreme_decimal(input: &str) -> Option<ParsedLongDouble> {
    let (negative, unsigned) = match input.as_bytes().first() {
        Some(b'-') => (true, &input[1..]),
        Some(b'+') => (false, &input[1..]),
        _ => (false, input),
    };
    if unsigned.starts_with("0x") || unsigned.starts_with("0X") {
        return None;
    }

    let exponent_index = unsigned.find(['e', 'E'])?;
    let exponent = parse_saturating_exponent(&unsigned[exponent_index + 1..])?;
    let mantissa = &unsigned[..exponent_index];
    let point = mantissa.find('.').unwrap_or(mantissa.len());
    let digits = mantissa.bytes().filter(|byte| byte.is_ascii_digit());
    let first_nonzero = digits.clone().position(|byte| byte != b'0');
    let Some(first_nonzero) = first_nonzero else {
        return Some(ParsedLongDouble {
            value: if negative {
                ExtendedBigDecimal::MinusZero
            } else {
                ExtendedBigDecimal::default()
            },
            range_error: false,
        });
    };

    let decimal_exponent = (point as i64)
        .saturating_sub(first_nonzero as i64)
        .saturating_sub(1)
        .saturating_add(exponent);
    if decimal_exponent > 100_000 {
        Some(ParsedLongDouble {
            value: if negative {
                ExtendedBigDecimal::MinusInfinity
            } else {
                ExtendedBigDecimal::Infinity
            },
            range_error: true,
        })
    } else if decimal_exponent < -100_000 {
        Some(ParsedLongDouble {
            value: if negative {
                ExtendedBigDecimal::MinusZero
            } else {
                ExtendedBigDecimal::default()
            },
            range_error: true,
        })
    } else {
        None
    }
}

fn parse_saturating_exponent(exponent: &str) -> Option<i64> {
    let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(exponent.parse().unwrap_or_else(|_| {
        if exponent.starts_with('-') {
            i64::MIN
        } else {
            i64::MAX
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        ExtendedBigDecimal, LongDoubleParseError, ParseAttempt, parse_long_double,
        select_parse_attempt,
    };

    #[test]
    fn parses_extreme_decimal_exponents_without_losing_the_value_class() {
        assert_eq!(
            parse_long_double("1e100001"),
            Err(LongDoubleParseError::OutOfRange(
                ExtendedBigDecimal::Infinity
            ))
        );
        assert_eq!(
            parse_long_double("-0e999999").unwrap(),
            ExtendedBigDecimal::MinusZero
        );
    }

    #[test]
    fn explicit_empty_argument_is_zero() {
        assert_eq!(
            parse_long_double("").unwrap(),
            ExtendedBigDecimal::default()
        );
    }

    #[test]
    fn reports_range_errors_with_the_quantized_long_double_value() {
        for (input, expected) in [
            ("2e4932", ExtendedBigDecimal::Infinity),
            ("1e-5000", ExtendedBigDecimal::default()),
            ("0x1p16384", ExtendedBigDecimal::Infinity),
            ("0x1p-20000", ExtendedBigDecimal::default()),
        ] {
            assert_eq!(
                parse_long_double(input),
                Err(LongDoubleParseError::OutOfRange(expected)),
                "input: {input}"
            );
        }
    }

    #[test]
    fn selects_the_longer_locale_or_c_decimal_prefix() {
        assert_eq!(
            select_parse_attempt("1,5", ","),
            Some(ParseAttempt {
                normalized_prefix: "1.5".to_string(),
                consumed: 3,
            })
        );
        assert_eq!(
            select_parse_attempt("1.5", ","),
            Some(ParseAttempt {
                normalized_prefix: "1.5".to_string(),
                consumed: 3,
            })
        );
        assert_eq!(
            select_parse_attempt("1,5.6", ","),
            Some(ParseAttempt {
                normalized_prefix: "1.5".to_string(),
                consumed: 3,
            })
        );
    }
}
