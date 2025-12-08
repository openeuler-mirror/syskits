/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! Utilities for parsing numbers in various formats

// spell-checker:ignore powf copysign prec inity

#[derive(Clone, Copy, PartialEq)]
pub enum Base {
    CtHexadecimal = 16,
    CtDecimal = 10,
    CtOctal = 8,
    CtBinary = 2,
}

impl Base {
    /*
        digit 的方法，用于从数字字符串中解析一个数字。digit 方法将字符 c 作为输入，并返回一个 Option<u64>，
        该 Option<u64> 表示解析后的数字，如果该字符不是当前基数的有效数字，则返回 None。
    digit 方法使用匹配语句来确定数字的当前基数。
    基数包括二进制、八进制、十进制和十六进制。
    对于每个基数，它都会检查输入字符 c 是否是该基数的有效数字。如果是，则使用 from_decimal 辅助函数返回解析后的数字。
    from_decimal 辅助函数简单地将输入字符 c 转换为 u64 值。它减去字符 "0 "的 u64 值，得到数字的数值。
    对于十六进制基数，它还会检查输入字符 c 是否是有效的十六进制数字。
    如果是，则使用相同的 from_decimal 辅助函数返回解析后的数字。如果输入字符 c 是小写十六进制数字，则会在其数值上加上 10，以获得正确的十六进制值。

    如果输入字符 c 不是当前基数的有效数字，则 digit 方法返回 None。
        */
    pub fn digit(&self, c_char: char) -> Option<u64> {
        // Inner function to convert a decimal character to u64
        fn from_ct_decimal(c_char: char) -> u64 {
            u64::from(c_char) - u64::from('0')
        }
        match self {
            Self::CtBinary => {
                if ('0'..='1').contains(&c_char) {
                    Some(from_ct_decimal(c_char))
                } else {
                    None
                }
            }
            Self::CtOctal => {
                if ('0'..='7').contains(&c_char) {
                    Some(from_ct_decimal(c_char))
                } else {
                    None
                }
            }
            Self::CtDecimal => {
                if c_char.is_ascii_digit() {
                    Some(from_ct_decimal(c_char))
                } else {
                    None
                }
            }
            Self::CtHexadecimal => match c_char.to_ascii_lowercase() {
                '0'..='9' => Some(from_ct_decimal(c_char)),
                c_char @ 'a'..='f' => Some(u64::from(c_char) - u64::from('a') + 10),
                _ => None,
            },
        }
    }
}

/// Type returned if a number could not be parsed in its entirety
#[derive(Debug, PartialEq)]
pub enum ParseError<'a, T> {
    /// The input as a whole makes no sense
    CtNotNumeric,
    /// The beginning of the input made sense and has been parsed,
    /// while the remaining doesn't.
    CtPartialMatch(T, &'a str),
    /// The integral part has overflowed the requested type, or
    /// has overflowed the `u64` internal storage when parsing the
    /// integral part of a floating point number.
    CtOverflow,
}

impl<'a, T> ParseError<'a, T> {
    fn map<U>(self, f: impl FnOnce(T, &'a str) -> ParseError<'a, U>) -> ParseError<'a, U> {
        match self {
            Self::CtNotNumeric => ParseError::CtNotNumeric,
            Self::CtOverflow => ParseError::CtOverflow,
            Self::CtPartialMatch(v, s) => f(v, s),
        }
    }
}

/// A number parser for binary, octal, decimal, hexadecimal and single characters.
///
/// Internally, in order to get the maximum possible precision and cover the full
/// range of u64 and i64 without losing precision for f64, the returned number is
/// decomposed into:
///   - A `base` value
///   - A `neg` sign bit
///   - A `integral` positive part
///   - A `fractional` positive part
///   - A `precision` representing the number of digits in the fractional part
///
/// If the fractional part cannot be represented on a `u64`, parsing continues
/// silently by ignoring non-significant digits.
pub struct ParsedNumber {
    base: Base,
    negative: bool,
    integral: u64,
    fractional: u64,
    precision: usize,
}

impl ParsedNumber {
    fn into_i64(self) -> Option<i64> {
        match self.negative {
            true => i64::try_from(-i128::from(self.integral)).ok(),
            false => i64::try_from(self.integral).ok(),
        }
    }

    /// Parse a number as i64. No fractional part is allowed.
    pub fn parse_i64(input: &str) -> Result<i64, ParseError<'_, i64>> {
        match Self::parse(input, true) {
            Ok(v) => {
                if let Some(i64_val) = v.into_i64() {
                    Ok(i64_val)
                } else {
                    Err(ParseError::CtOverflow)
                }
            }
            Err(e) => Err(e.map(|v, rest| {
                if let Some(i64_val) = v.into_i64() {
                    ParseError::CtPartialMatch(i64_val, rest)
                } else {
                    ParseError::CtOverflow
                }
            })),
        }
    }

    /// Parse a number as u64. No fractional part is allowed.
    pub fn parse_u64(input: &str) -> Result<u64, ParseError<'_, u64>> {
        match Self::parse(input, true) {
            Ok(v) | Err(ParseError::CtPartialMatch(v, _)) if v.negative => {
                Err(ParseError::CtNotNumeric)
            }
            Ok(v) => Ok(v.integral),
            Err(e) => Err(e.map(|v, rest| {
                let ct_integral = v.integral;
                ParseError::CtPartialMatch(ct_integral, rest)
            })),
        }
    }

    fn into_f64(self) -> f64 {
        let n = self.integral as f64
            + (self.fractional as f64) / (self.base as u8 as f64).powf(self.precision as f64);
        match self.negative {
            true => -n,
            false => n,
        }
    }

    /// Parse a number as f64
    pub fn parse_f64(input: &str) -> Result<f64, ParseError<'_, f64>> {
        match Self::parse(input, false) {
            Ok(v) => {
                let v64 = v.into_f64();
                Ok(v64)
            }
            Err(ParseError::CtNotNumeric) => Self::parse_f64_special_values(input),
            Err(e) => Err(e.map(|v, rest| {
                let ct_64 = v.into_f64();
                ParseError::CtPartialMatch(ct_64, rest)
            })),
        }
    }

    fn parse_f64_special_values(input: &str) -> Result<f64, ParseError<'_, f64>> {
        let (sign, rest) = if let Some(input) = input.strip_prefix('-') {
            (-1.0, input)
        } else {
            (1.0, input)
        };

        let prefix = rest
            .chars()
            .take(3)
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>();
        let special = match prefix.as_str() {
            "inf" => f64::INFINITY,
            "nan" => f64::NAN,
            _ => return Err(ParseError::CtNotNumeric),
        }
        .copysign(sign);
        if rest.len() == 3 {
            Ok(special)
        } else {
            Err(ParseError::CtPartialMatch(special, &rest[3..]))
        }
    }

    #[allow(clippy::cognitive_complexity)]
    fn parse(input: &str, integral_only: bool) -> Result<Self, ParseError<'_, Self>> {
        // Parse the "'" prefix separately
        match input.strip_prefix('\'') {
            Some(rest) => {
                let mut chars = rest.char_indices().fuse();
                let v = chars.next().map(|(_, c)| Self {
                    base: Base::CtDecimal,
                    negative: false,
                    integral: u64::from(c),
                    fractional: 0,
                    precision: 0,
                });

                let chars_next = match (v, chars.next()) {
                    (Some(v), None) => Ok(v),
                    (Some(v), Some((i, _))) => Err(ParseError::CtPartialMatch(v, &rest[i..])),
                    (None, _) => Err(ParseError::CtNotNumeric),
                };

                return chars_next;
            }

            None => {
                // 不处理
            }
        };

        // Initial minus sign
        let (negative, unsigned_str) = match input.strip_prefix('-') {
            Some(input) => (true, input),
            None => (false, input),
        };

        // Parse an optional base prefix ("0b" / "0B" / "0" / "0x" / "0X"). "0" is octal unless a
        // fractional part is allowed in which case it is an insignificant leading 0. A "0" prefix
        // will not be consumed in case the parsable string contains only "0": the leading extra "0"
        // will have no influence on the result.
        let (base, rest) = if let Some(rest) = unsigned_str.strip_prefix('0') {
            if let Some(rest) = rest.strip_prefix(['b', 'B']) {
                (Base::CtBinary, rest)
            } else if let Some(rest) = rest.strip_prefix(['x', 'X']) {
                (Base::CtHexadecimal, rest)
            } else if integral_only {
                (Base::CtOctal, unsigned_str)
            } else {
                (Base::CtDecimal, unsigned_str)
            }
        } else {
            (Base::CtDecimal, unsigned_str)
        };
        if rest.is_empty() {
            return Err(ParseError::CtNotNumeric);
        }

        // Parse the integral part of the number
        let mut chars = rest.chars().enumerate().fuse().peekable();
        let mut integral = 0u64;
        while let Some(d) = chars.peek().and_then(|&(_, c)| base.digit(c)) {
            chars.next();
            integral = match integral.checked_mul(base as u64) {
                Some(n) => match n.checked_add(d) {
                    Some(result) => result,
                    None => return Err(ParseError::CtOverflow),
                },
                None => return Err(ParseError::CtOverflow),
            };
        }

        // Parse the fractional part of the number if there can be one and the input contains
        // a '.' decimal separator.
        let (mut fractional, mut precision) = (0u64, 0);
        if matches!(chars.peek(), Some(&(_, '.')))
            && matches!(base, Base::CtDecimal | Base::CtHexadecimal)
            && !integral_only
        {
            chars.next();
            let mut ended = false;
            while let Some(d) = chars.peek().and_then(|&(_, c)| base.digit(c)) {
                chars.next();
                if !ended {
                    match fractional
                        .checked_mul(base as u64)
                        .and_then(|n| n.checked_add(d))
                    {
                        Some(f) => {
                            fractional = f;
                            precision += 1;
                        }
                        None => ended = true,
                    }
                }
            }
        }

        // If nothing has been parsed, declare the parsing unsuccessful
        if let Some((0, _)) = chars.peek() {
            return Err(ParseError::CtNotNumeric);
        }
        // Return what has been parsed so far. It there are extra characters, mark the
        // parsing as a partial match.
        let parsed = Self {
            base,
            negative,
            integral,
            fractional,
            precision,
        };
        match chars.next() {
            Some((first_unparsed, _)) => {
                Err(ParseError::CtPartialMatch(parsed, &rest[first_unparsed..]))
            }
            None => Ok(parsed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ParseError, ParsedNumber};
    use crate::format::num_parser::Base;

    #[test]
    fn test_into_f64_decimal() {
        let parsed_number = ParsedNumber {
            base: Base::CtDecimal,
            negative: false,
            integral: 123,
            fractional: 456,
            precision: 3,
        };
        assert_eq!(parsed_number.into_f64(), 123.456);
    }

    #[test]
    fn test_into_f64_octal() {
        let parsed_number = ParsedNumber {
            base: Base::CtOctal,
            negative: false,
            integral: 123,
            fractional: 456,
            precision: 3,
        };
        assert_eq!(parsed_number.into_f64(), 123.890625);
    }

    #[test]
    fn test_into_f64_binary() {
        let parsed_number = ParsedNumber {
            base: Base::CtBinary,
            negative: false,
            integral: 101,
            fractional: 110,
            precision: 3,
        };
        assert_eq!(parsed_number.into_f64(), 114.75);
    }

    #[test]
    fn test_into_f64_hexadecimal() {
        let parsed_number = ParsedNumber {
            base: Base::CtHexadecimal,
            negative: false,
            integral: 0xAB,
            fractional: 0xCD,
            precision: 3,
        };
        assert_eq!(parsed_number.into_f64(), 171.050048828125);
    }

    #[test]
    fn test_into_f64_decimal_basic() {
        let parsed_number = ParsedNumber {
            base: Base::CtDecimal,
            negative: false,
            integral: 123,
            fractional: 456,
            precision: 3,
        };
        assert_eq!(parsed_number.into_f64(), 123.456);
    }

    #[test]
    fn test_into_f64_decimal_negative() {
        let parsed_number = ParsedNumber {
            base: Base::CtDecimal,
            negative: true,
            integral: 789,
            fractional: 123,
            precision: 2,
        };
        assert_eq!(parsed_number.into_f64(), -790.23);
    }

    #[test]
    fn test_into_f64_decimal_zero() {
        let parsed_number = ParsedNumber {
            base: Base::CtDecimal,
            negative: false,
            integral: 0,
            fractional: 0,
            precision: 0,
        };
        assert_eq!(parsed_number.into_f64(), 0.0);
    }

    #[test]
    fn test_into_f64_decimal_large_precision() {
        let parsed_number = ParsedNumber {
            base: Base::CtDecimal,
            negative: false,
            integral: 123,
            fractional: 456,
            precision: 10,
        };
        assert_eq!(parsed_number.into_f64(), 123.0000000456);
    }

    #[test]
    fn test_into_f64_decimal_negative_fractional() {
        let parsed_number = ParsedNumber {
            base: Base::CtDecimal,
            negative: false,
            integral: 123,
            fractional: 456,
            precision: 3,
        };
        assert_eq!(parsed_number.into_f64(), 123.456);
    }

    #[test]
    fn test_into_f64_max_value() {
        let parsed_number = ParsedNumber {
            base: Base::CtDecimal,
            negative: false,
            integral: u64::MAX,
            fractional: u64::MAX,
            precision: 10,
        };
        assert_eq!(parsed_number.into_f64(), 1.8446744075554226e19);
    }

      #[test]
    fn test_into_f64_min_value() {
        let parsed_number = ParsedNumber {
            base: Base::CtDecimal,
            negative: true,
            integral: u64::MAX,
            fractional: u64::MAX,
            precision: 10,
        };
        assert_eq!(parsed_number.into_f64(), -1.8446744075554226e19);
    }

    #[test]
    fn test_digit_binary() {
        let base = Base::CtBinary;
        // let parsed = ParsedNumber::new(Base::Binary, false, 0, 0, 0);
        assert_eq!(Some(1), base.digit('1'));
        assert_eq!(None, base.digit('2'));
        assert_eq!(None, base.digit('a'));
    }

    #[test]
    fn test_digit_octal() {
        let parsed = Base::CtOctal;
        assert_eq!(Some(1), parsed.digit('1'));
        assert_eq!(Some(2), parsed.digit('2'));
        assert_eq!(Some(3), parsed.digit('3'));
        assert_eq!(Some(4), parsed.digit('4'));
        assert_eq!(Some(5), parsed.digit('5'));
        assert_eq!(Some(6), parsed.digit('6'));
        assert_eq!(Some(7), parsed.digit('7'));
        assert_eq!(None, parsed.digit('8'));
        assert_eq!(None, parsed.digit('9'));
        assert_eq!(None, parsed.digit('a'));
    }

    #[test]
    fn test_digit_decimal() {
        let parsed = Base::CtDecimal;

        assert_eq!(Some(1), parsed.digit('1'));
        assert_eq!(Some(2), parsed.digit('2'));
        assert_eq!(Some(3), parsed.digit('3'));
        assert_eq!(Some(4), parsed.digit('4'));
        assert_eq!(Some(5), parsed.digit('5'));
        assert_eq!(Some(6), parsed.digit('6'));
        assert_eq!(Some(7), parsed.digit('7'));
        assert_eq!(Some(8), parsed.digit('8'));
        assert_eq!(Some(9), parsed.digit('9'));
        assert_eq!(None, parsed.digit('a'));
    }

    #[test]
    fn test_digit_hexadecimal() {
        let parsed = Base::CtHexadecimal;

        assert_eq!(Some(1), parsed.digit('1'));
        assert_eq!(Some(2), parsed.digit('2'));
        assert_eq!(Some(3), parsed.digit('3'));
        assert_eq!(Some(4), parsed.digit('4'));
        assert_eq!(Some(5), parsed.digit('5'));
        assert_eq!(Some(6), parsed.digit('6'));
        assert_eq!(Some(7), parsed.digit('7'));
        assert_eq!(Some(8), parsed.digit('8'));
        assert_eq!(Some(9), parsed.digit('9'));
        assert_eq!(Some(10), parsed.digit('a'));
        assert_eq!(Some(11), parsed.digit('b'));
        assert_eq!(Some(12), parsed.digit('c'));
        assert_eq!(Some(13), parsed.digit('d'));
        assert_eq!(Some(14), parsed.digit('e'));
        assert_eq!(Some(15), parsed.digit('f'));
        assert_eq!(None, parsed.digit('g'));
    }
    #[test]
    fn test_digit_binary_overflow() {
        let base = Base::CtBinary;
        assert_eq!(None, base.digit('2'));
    }

    #[test]
    fn test_digit_decimal_overflow() {
        let base = Base::CtDecimal;
        assert_eq!(None, base.digit('a'));
    }

    #[test]
    fn test_digit_hexadecimal_overflow() {
        let base = Base::CtHexadecimal;
        assert_eq!(None, base.digit('h'));
    }

    #[test]
    fn test_digit_octal_overflow() {
        let base = Base::CtOctal;
        assert_eq!(None, base.digit('8'));
    }

    #[test]
    fn test_digit_binary_invalid() {
        let base = Base::CtBinary;
        assert_eq!(None, base.digit('a'));
    }

}