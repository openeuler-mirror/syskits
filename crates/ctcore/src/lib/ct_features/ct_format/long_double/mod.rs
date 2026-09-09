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

pub(super) fn parse_long_double(
    input: &str,
) -> Result<ExtendedBigDecimal, ParseError<'_, ExtendedBigDecimal>> {
    match ParsedNumber::parse_f64(input) {
        Ok(_) => parse_complete_long_double(input).map_err(|_| ParseError::CtNotNumeric),
        Err(ParseError::CtPartialMatch(_, rest)) => {
            let parsed_len = input.len() - rest.len();
            let value = parse_complete_long_double(&input[..parsed_len])
                .map_err(|_| ParseError::CtNotNumeric)?;
            Err(ParseError::CtPartialMatch(value, rest))
        }
        Err(ParseError::CtNotNumeric) => Err(ParseError::CtNotNumeric),
        Err(ParseError::CtOverflow) => Err(ParseError::CtOverflow),
    }
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
        parse_complete_long_double(&value.to_string()).unwrap_or_default()
    }
}

fn parse_complete_long_double(input: &str) -> Result<ExtendedBigDecimal, ParseNumberError> {
    let trimmed = input.trim_start_matches(char::is_whitespace);
    if let Some(rest) = trimmed.strip_prefix(['\'', '"']) {
        return rest
            .chars()
            .next()
            .map(|value| ExtendedBigDecimal::from_u64(u64::from(value)))
            .ok_or(ParseNumberError::Float);
    }

    let (negative, unsigned) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    if unsigned.to_ascii_lowercase().starts_with("nan") {
        return Ok(if negative {
            ExtendedBigDecimal::MinusNan
        } else {
            ExtendedBigDecimal::Nan
        });
    }

    if let Some(value) = classify_extreme_decimal(trimmed) {
        return Ok(value);
    }

    input.parse::<PreciseNumber>().map(|number| number.number)
}

fn classify_extreme_decimal(input: &str) -> Option<ExtendedBigDecimal> {
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
        return Some(if negative {
            ExtendedBigDecimal::MinusZero
        } else {
            ExtendedBigDecimal::default()
        });
    };

    let decimal_exponent = (point as i64)
        .saturating_sub(first_nonzero as i64)
        .saturating_sub(1)
        .saturating_add(exponent);
    if decimal_exponent > 100_000 {
        Some(if negative {
            ExtendedBigDecimal::MinusInfinity
        } else {
            ExtendedBigDecimal::Infinity
        })
    } else if decimal_exponent < -100_000 {
        Some(if negative {
            ExtendedBigDecimal::MinusZero
        } else {
            ExtendedBigDecimal::default()
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
    use super::{ExtendedBigDecimal, parse_long_double};

    #[test]
    fn parses_extreme_decimal_exponents_without_losing_the_value_class() {
        assert_eq!(
            parse_long_double("1e100001").unwrap(),
            ExtendedBigDecimal::Infinity
        );
        assert_eq!(
            parse_long_double("-0e999999").unwrap(),
            ExtendedBigDecimal::MinusZero
        );
    }
}
