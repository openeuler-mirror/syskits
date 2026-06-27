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
//!
//! This module provides an implementation of [`FromStr`] for the
//! [`PreciseNumber`] struct.
use std::str::FromStr;

use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use num_traits::Num;
use num_traits::Zero;

use crate::extendedbigdecimal::ExtendedBigDecimal;
use crate::number::PreciseNumber;

/// An error returned when parsing a number fails.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseNumberError {
    Float,
    Nan,
    Hex,
}

/// Decide whether a given string and its parsed `BigInt` is negative zero.
fn is_minus_zero_int(s: &str, n: &BigDecimal) -> bool {
    s.starts_with('-') && n == &BigDecimal::zero()
}

/// Decide whether a given string and its parsed `BigDecimal` is negative zero.
fn is_minus_zero_float(s: &str, x: &BigDecimal) -> bool {
    s.starts_with('-') && x == &BigDecimal::zero()
}

/// Parse a number with neither a decimal point nor an exponent.
///
/// # Errors
///
/// This function returns an error if the input string is a variant of
/// "NaN" or if no [`BigInt`] could be parsed from the string.
///
/// # Examples
///
/// ```rust,ignore
/// let actual = "0".parse::<Number>().unwrap().number;
/// let expected = Number::BigInt(BigInt::zero());
/// assert_eq!(actual, expected);
/// ```
fn parse_no_decimal_no_exponent(s: &str) -> Result<PreciseNumber, ParseNumberError> {
    match s.parse::<BigDecimal>() {
        Ok(n) => {
            if is_minus_zero_int(s, &n) {
                Ok(PreciseNumber::new(ExtendedBigDecimal::MinusZero, 2, 0))
            } else {
                Ok(PreciseNumber::new(
                    ExtendedBigDecimal::BigDecimal(n),
                    s.len(),
                    0,
                ))
            }
        }
        Err(_) => match s.to_ascii_lowercase().as_str() {
            "inf" | "infinity" => Ok(PreciseNumber::new(ExtendedBigDecimal::Infinity, 0, 0)),
            "-inf" | "-infinity" => Ok(PreciseNumber::new(ExtendedBigDecimal::MinusInfinity, 0, 0)),
            "nan" | "-nan" => Err(ParseNumberError::Nan),
            _ => Err(ParseNumberError::Float),
        },
    }
}

/// Parse a number with an exponent but no decimal point.
///
/// # Errors
///
/// This function returns an error if `s` is not a valid number.
///
/// # Examples
///
/// ```rust,ignore
/// let actual = "1e2".parse::<Number>().unwrap().number;
/// let expected = "100".parse::<BigInt>().unwrap();
/// assert_eq!(actual, expected);
/// ```
fn parse_exponent_no_decimal(s: &str, j: usize) -> Result<PreciseNumber, ParseNumberError> {
    let exponent: i64 = s[j + 1..].parse().map_err(|_| ParseNumberError::Float)?;
    let x: BigDecimal = s.parse().map_err(|_| ParseNumberError::Float)?;

    let num_integral_digits = if is_minus_zero_int(s, &x) {
        if exponent > 0 {
            2 + exponent as usize
        } else {
            2
        }
    } else if s.starts_with('-') && exponent < 0 {
        2 // 对于负数且负指数的情况，始终返回2位
    } else if exponent >= 0 {
        (j as i64 + exponent) as usize
    } else {
        1
    };

    Ok(PreciseNumber::new(
        if is_minus_zero_int(s, &x) {
            ExtendedBigDecimal::MinusZero
        } else {
            ExtendedBigDecimal::BigDecimal(x)
        },
        num_integral_digits,
        if exponent < 0 { -exponent as usize } else { 0 },
    ))
}

/// Parse a number with a decimal point but no exponent.
///
/// # Errors
///
/// This function returns an error if `s` is not a valid number.
///
/// # Examples
///
/// ```rust,ignore
/// let actual = "1.2".parse::<Number>().unwrap().number;
/// let expected = "1.2".parse::<BigDecimal>().unwrap();
/// assert_eq!(actual, expected);
/// ```
fn parse_decimal_no_exponent(s: &str, i: usize) -> Result<PreciseNumber, ParseNumberError> {
    let x: BigDecimal = s.parse().map_err(|_| ParseNumberError::Float)?;
    let num_integral_digits = if s.starts_with("-.") { i + 1 } else { i };
    let num_fractional_digits = s.len() - (i + 1);

    Ok(PreciseNumber::new(
        if is_minus_zero_float(s, &x) {
            ExtendedBigDecimal::MinusZero
        } else {
            ExtendedBigDecimal::BigDecimal(x)
        },
        num_integral_digits,
        num_fractional_digits,
    ))
}

/// 计算最小整数位数
fn calculate_minimum_digits(s: &str, j: usize) -> Result<usize, ParseNumberError> {
    let integral_part: f64 = s[..j].parse().map_err(|_| ParseNumberError::Float)?;
    Ok(if integral_part.is_sign_negative() {
        2
    } else {
        1
    })
}

/// 计算总整数位数
fn calculate_total_digits(s: &str, i: usize, exponent: i64) -> i64 {
    if s.starts_with("-.") {
        i as i64 + exponent + 1
    } else {
        i as i64 + exponent
    }
}

/// 构建扩展数字字符串
fn build_expanded_number(s: &str, i: usize, j: usize, zeros_count: usize) -> String {
    let zeros = "0".repeat(zeros_count);
    [&s[0..i], &s[i + 1..j], &zeros].concat()
}

/// Parse a number with both a decimal point and an exponent.
///
/// # Errors
///
/// This function returns an error if `s` is not a valid number.
///
/// # Examples
///
/// ```rust,ignore
/// let actual = "1.2e3".parse::<Number>().unwrap().number;
/// let expected = "1200".parse::<BigInt>().unwrap();
/// assert_eq!(actual, expected);
/// ```
fn parse_decimal_and_exponent(
    s: &str,
    i: usize,
    j: usize,
) -> Result<PreciseNumber, ParseNumberError> {
    let num_digits_between_decimal_point_and_e = (j - (i + 1)) as i64;
    let exponent: i64 = s[j + 1..].parse().map_err(|_| ParseNumberError::Float)?;
    let val: BigDecimal = s.parse().map_err(|_| ParseNumberError::Float)?;

    let num_integral_digits = {
        let minimum = calculate_minimum_digits(s, j)?;
        let total = calculate_total_digits(s, i, exponent);
        if total < minimum as i64 {
            minimum
        } else {
            total.try_into().unwrap()
        }
    };

    let num_fractional_digits = if num_digits_between_decimal_point_and_e < exponent {
        0
    } else {
        (num_digits_between_decimal_point_and_e - exponent)
            .try_into()
            .unwrap()
    };

    if num_digits_between_decimal_point_and_e <= exponent {
        if is_minus_zero_float(s, &val) {
            Ok(PreciseNumber::new(
                ExtendedBigDecimal::MinusZero,
                num_integral_digits,
                num_fractional_digits,
            ))
        } else {
            let zeros_count = (exponent - num_digits_between_decimal_point_and_e)
                .try_into()
                .unwrap();
            let expanded = build_expanded_number(s, i, j, zeros_count);
            let n = expanded.parse().map_err(|_| ParseNumberError::Float)?;
            Ok(PreciseNumber::new(
                ExtendedBigDecimal::BigDecimal(n),
                num_integral_digits,
                num_fractional_digits,
            ))
        }
    } else if is_minus_zero_float(s, &val) {
        Ok(PreciseNumber::new(
            ExtendedBigDecimal::MinusZero,
            num_integral_digits,
            num_fractional_digits,
        ))
    } else {
        Ok(PreciseNumber::new(
            ExtendedBigDecimal::BigDecimal(val),
            num_integral_digits,
            num_fractional_digits,
        ))
    }
}

/// Parse a hexadecimal integer from a string.
///
/// # Errors
///
/// This function returns an error if no [`BigInt`] could be parsed from
/// the string.
///
/// # Examples
///
/// ```rust,ignore
/// let actual = "0x0".parse::<Number>().unwrap().number;
/// let expected = Number::BigInt(BigInt::zero());
/// assert_eq!(actual, expected);
/// ```
fn parse_hexadecimal(s: &str) -> Result<PreciseNumber, ParseNumberError> {
    let (is_neg, s) = if s.starts_with('-') {
        (true, &s[3..])
    } else {
        (false, &s[2..])
    };

    if s.starts_with('-') || s.starts_with('+') {
        // Even though this is more like an invalid hexadecimal number,
        // GNU reports this as an invalid floating point number, so we
        // use `ParseNumberError::Float` to match that behavior.
        return Err(ParseNumberError::Float);
    }

    let num = BigInt::from_str_radix(s, 16).map_err(|_| ParseNumberError::Hex)?;
    let num = BigDecimal::from(num);

    match (is_neg, num == BigDecimal::zero()) {
        (true, true) => Ok(PreciseNumber::new(ExtendedBigDecimal::MinusZero, 2, 0)),
        (true, false) => Ok(PreciseNumber::new(
            ExtendedBigDecimal::BigDecimal(-num),
            0,
            0,
        )),
        (false, _) => Ok(PreciseNumber::new(
            ExtendedBigDecimal::BigDecimal(num),
            0,
            0,
        )),
    }
}

impl FromStr for PreciseNumber {
    type Err = ParseNumberError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim_start();
        let s = s.strip_prefix('+').unwrap_or(s);

        // 处理十六进制数字
        if let Some(i) = s.to_lowercase().find("0x") {
            if i <= 1 {
                return parse_hexadecimal(s);
            }
        }

        // 找到小数点和指数位置
        let decimal_pos = s.find('.');
        let exp_pos = s.find('e');

        match (decimal_pos, exp_pos) {
            (None, None) => parse_no_decimal_no_exponent(s),
            (None, Some(e)) => parse_exponent_no_decimal(s, e),
            (Some(d), None) => parse_decimal_no_exponent(s, d),
            (Some(d), Some(e)) if d < e => parse_decimal_and_exponent(s, d, e),
            _ => Err(ParseNumberError::Float),
        }
    }
}

#[cfg(test)]
mod tests {
    use bigdecimal::BigDecimal;

    use crate::extendedbigdecimal::ExtendedBigDecimal;
    use crate::number::PreciseNumber;
    use crate::numberparse::ParseNumberError;

    /// Convenience function for parsing a [`Number`] and unwrapping.
    fn parse(s: &str) -> ExtendedBigDecimal {
        s.parse::<PreciseNumber>().unwrap().number
    }

    /// Convenience function for getting the number of integral digits.
    fn num_integral_digits(s: &str) -> usize {
        s.parse::<PreciseNumber>().unwrap().num_integral_digits
    }

    /// Convenience function for getting the number of fractional digits.
    fn num_fractional_digits(s: &str) -> usize {
        s.parse::<PreciseNumber>().unwrap().num_fractional_digits
    }

    #[test]
    fn test_parse_minus_zero_int() {
        assert_eq!(parse("-0e0"), ExtendedBigDecimal::MinusZero);
        assert_eq!(parse("-0e-0"), ExtendedBigDecimal::MinusZero);
        assert_eq!(parse("-0e1"), ExtendedBigDecimal::MinusZero);
        assert_eq!(parse("-0e+1"), ExtendedBigDecimal::MinusZero);
        assert_eq!(parse("-0.0e1"), ExtendedBigDecimal::MinusZero);
        assert_eq!(parse("-0x0"), ExtendedBigDecimal::MinusZero);
    }

    #[test]
    fn test_parse_minus_zero_float() {
        assert_eq!(parse("-0.0"), ExtendedBigDecimal::MinusZero);
        assert_eq!(parse("-0e-1"), ExtendedBigDecimal::MinusZero);
        assert_eq!(parse("-0.0e-1"), ExtendedBigDecimal::MinusZero);
    }

    #[test]
    fn test_parse_big_int() {
        assert_eq!(parse("0"), ExtendedBigDecimal::zero());
        assert_eq!(parse("0.1e1"), ExtendedBigDecimal::one());
        assert_eq!(
            parse("1.0e1"),
            ExtendedBigDecimal::BigDecimal("10".parse::<BigDecimal>().unwrap())
        );
    }

    #[test]
    fn test_parse_hexadecimal_big_int() {
        assert_eq!(parse("0x0"), ExtendedBigDecimal::zero());
        assert_eq!(
            parse("0x10"),
            ExtendedBigDecimal::BigDecimal("16".parse::<BigDecimal>().unwrap())
        );
    }

    #[test]
    fn test_parse_big_decimal() {
        assert_eq!(
            parse("0.0"),
            ExtendedBigDecimal::BigDecimal("0.0".parse::<BigDecimal>().unwrap())
        );
        assert_eq!(
            parse(".0"),
            ExtendedBigDecimal::BigDecimal("0.0".parse::<BigDecimal>().unwrap())
        );
        assert_eq!(
            parse("1.0"),
            ExtendedBigDecimal::BigDecimal("1.0".parse::<BigDecimal>().unwrap())
        );
        assert_eq!(
            parse("10e-1"),
            ExtendedBigDecimal::BigDecimal("1.0".parse::<BigDecimal>().unwrap())
        );
        assert_eq!(
            parse("-1e-3"),
            ExtendedBigDecimal::BigDecimal("-0.001".parse::<BigDecimal>().unwrap())
        );
    }

    #[test]
    fn test_parse_inf() {
        assert_eq!(parse("inf"), ExtendedBigDecimal::Infinity);
        assert_eq!(parse("infinity"), ExtendedBigDecimal::Infinity);
        assert_eq!(parse("+inf"), ExtendedBigDecimal::Infinity);
        assert_eq!(parse("+infinity"), ExtendedBigDecimal::Infinity);
        assert_eq!(parse("-inf"), ExtendedBigDecimal::MinusInfinity);
        assert_eq!(parse("-infinity"), ExtendedBigDecimal::MinusInfinity);
    }

    #[test]
    fn test_parse_invalid_float() {
        assert_eq!(
            "1.2.3".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Float
        );
        assert_eq!(
            "1e2e3".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Float
        );
        assert_eq!(
            "1e2.3".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Float
        );
        assert_eq!(
            "-+-1".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Float
        );
    }

    #[test]
    fn test_parse_invalid_hex() {
        assert_eq!(
            "0xg".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Hex
        );
    }

    #[test]
    fn test_parse_invalid_nan() {
        assert_eq!(
            "nan".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Nan
        );
        assert_eq!(
            "NAN".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Nan
        );
        assert_eq!(
            "NaN".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Nan
        );
        assert_eq!(
            "nAn".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Nan
        );
        assert_eq!(
            "-nan".parse::<PreciseNumber>().unwrap_err(),
            ParseNumberError::Nan
        );
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_num_integral_digits() {
        let test_cases = [
            ("123", 3),
            ("123.45", 3),
            ("-0.1", 2),
            ("-.1", 2),
            ("123e4", 7),
            ("123e-4", 1),
            ("-1e-3", 2),
            ("123.45e6", 9),
            ("123.45e-6", 1),
            ("123.45e-1", 2),
            ("-0.1e0", 2),
            ("-0.1e2", 4),
            ("-.1e0", 2),
            ("-.1e2", 4),
            ("-1.e-3", 2),
            ("-1.0e-4", 2),
        ];

        for (input, expected) in test_cases {
            let result = num_integral_digits(input);
            println!("Testing '{}': expected {}, got {}", input, expected, result);
            assert_eq!(
                result, expected,
                "Failed for input: '{}', expected: {}, got: {}",
                input, expected, result
            );
        }
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_num_fractional_digits() {
        // no decimal, no exponent
        assert_eq!(num_fractional_digits("123"), 0);
        assert_eq!(num_fractional_digits("0xff"), 0);
        // decimal, no exponent
        assert_eq!(num_fractional_digits("123.45"), 2);
        assert_eq!(num_fractional_digits("-0.1"), 1);
        assert_eq!(num_fractional_digits("-.1"), 1);
        // exponent, no decimal
        assert_eq!(num_fractional_digits("123e4"), 0);
        assert_eq!(num_fractional_digits("123e-4"), 4);
        assert_eq!(num_fractional_digits("123e-1"), 1);
        assert_eq!(num_fractional_digits("-1e-3"), 3);
        // decimal and exponent
        assert_eq!(num_fractional_digits("123.45e6"), 0);
        assert_eq!(num_fractional_digits("123.45e1"), 1);
        assert_eq!(num_fractional_digits("123.45e-6"), 8);
        assert_eq!(num_fractional_digits("123.45e-1"), 3);
        assert_eq!(num_fractional_digits("-0.1e0"), 1);
        assert_eq!(num_fractional_digits("-0.1e2"), 0);
        assert_eq!(num_fractional_digits("-.1e0"), 1);
        assert_eq!(num_fractional_digits("-.1e2"), 0);
        assert_eq!(num_fractional_digits("-1.e-3"), 3);
        assert_eq!(num_fractional_digits("-1.0e-4"), 5);
        // minus zero int
        assert_eq!(num_fractional_digits("-0e0"), 0);
        assert_eq!(num_fractional_digits("-0e-0"), 0);
        assert_eq!(num_fractional_digits("-0e1"), 0);
        assert_eq!(num_fractional_digits("-0e+1"), 0);
        assert_eq!(num_fractional_digits("-0.0e1"), 0);
        // minus zero float
        assert_eq!(num_fractional_digits("-0.0"), 1);
        assert_eq!(num_fractional_digits("-0e-1"), 1);
        assert_eq!(num_fractional_digits("-0.0e-1"), 2);
    }

    #[test]
    fn test_edge_cases() {
        let test_cases = [
            ("0", 1),
            ("-0", 2),
            ("0.0", 1),
            ("-0.0", 2),
            ("0e0", 1),
            ("-0e0", 2),
            ("0.0e0", 1),
            ("-0.0e0", 2),
            ("1e-10", 1),
            ("-1e-10", 2),
            ("0.1e-10", 1),
            ("-0.1e-10", 2),
        ];

        for (input, expected) in test_cases {
            let result = num_integral_digits(input);
            println!(
                "Testing edge case '{}': expected {}, got {}",
                input, expected, result
            );
            assert_eq!(result, expected, "Failed for edge case: '{}'", input);
        }
    }
}
