/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

//! 对表示基 10 数的字符串进行快速比较而不损失精度。
//!
//! 为了能够在比较时短路，[NumInfo] 必须与每个数字一起传递给 [numeric_str_cmp] 。
//! [NumInfo] 通常通过调用 [NumInfo::parse] 获得，并应缓存。
//! 允许事后任意修改指数，这相当于移动小数点。
//!
//! 更具体地说，可以将指数理解为原始数字的 (1..10)*10^exponent 值。
//! 由此得出该算法的限制条件： 它能够比较 ±(1*10^[i64::MIN]..10*10^[i64::MAX]) 范围内的数字。

use std::{cmp::Ordering, ops::Range};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
enum NumSign {
    Negative,
    Positive,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct NumInfo {
    exponent: i64,
    sign: NumSign,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct NumInfoParseSettings {
    pub accept_si_units: bool,
    pub thousands_separator: Option<char>,
    pub decimal_pt: Option<char>,
}

impl Default for NumInfoParseSettings {
    fn default() -> Self {
        Self {
            accept_si_units: false,
            thousands_separator: None,
            decimal_pt: Some('.'),
        }
    }
}

impl NumInfo {
    /// 为这个数字解析 NumInfo。
    /// 同时返回应传递给 numeric_str_cmp 的 num 范围。
    ///
    /// 返回的范围将不包括前导零。如果数字只包含零、
    /// 返回一个空范围（idx...idx），这样 idx 就是最后一个零后面的字符。
    /// 如果输入的不是数字（必须被视为零），返回的空范围
    /// 将返回 0...0。
    #[allow(clippy::cognitive_complexity)]
    pub fn parse(num: &str, parse_settings: &NumInfoParseSettings) -> (Self, Range<usize>) {
        let mut exponent = -1;
        let mut is_had_decimal_pt = false;
        let mut is_had_digit = false;
        let mut start = None;
        let mut sign = NumSign::Positive;

        let mut is_first_char = true;

        for (idx, char) in num.char_indices() {
            if is_first_char && char.is_whitespace() {
                continue;
            }

            if is_first_char && char == '-' {
                sign = NumSign::Negative;
                is_first_char = false;
                continue;
            }
            is_first_char = false;

            if matches!(
                parse_settings.thousands_separator,
                Some(c) if c == char
            ) {
                continue;
            }

            if Self::is_invalid_char(char, &mut is_had_decimal_pt, parse_settings) {
                return match start {
                    Some(start) => {
                        let has_si_unit = parse_settings.accept_si_units
                            && matches!(char, 'K' | 'k' | 'M' | 'G' | 'T' | 'P' | 'E' | 'Z' | 'Y');
                        (
                            Self { exponent, sign },
                            start..if has_si_unit { idx + 1 } else { idx },
                        )
                    }
                    _ => (
                        Self {
                            sign: NumSign::Positive,
                            exponent: 0,
                        },
                        match is_had_digit {
                            true => idx..idx,
                            false => 0..0,
                        },
                    ),
                };
            }
            if Some(char) == parse_settings.decimal_pt {
                continue;
            }
            is_had_digit = true;
            if start.is_none() && char == '0' {
                if is_had_decimal_pt {
                    // We're parsing a number whose first nonzero digit is after the decimal point.
                    exponent -= 1;
                } else {
                    // Skip leading zeroes
                    continue;
                }
            }
            if !is_had_decimal_pt {
                exponent += 1;
            }
            if start.is_none() && char != '0' {
                start = Some(idx);
            }
        }

        match start {
            Some(start) => (Self { exponent, sign }, start..num.len()),
            _ => (
                Self {
                    sign: NumSign::Positive,
                    exponent: 0,
                },
                match is_had_digit {
                    true => num.len()..num.len(),
                    false => 0..0,
                },
            ),
        }
    }

    fn is_invalid_char(
        c: char,
        had_decimal_pt: &mut bool,
        parse_settings: &NumInfoParseSettings,
    ) -> bool {
        if Some(c) == parse_settings.decimal_pt {
            if *had_decimal_pt {
                // 这是一个十进制 pt，但我们已经有了一个，所以无效
                true
            } else {
                *had_decimal_pt = true;
                false
            }
        } else {
            !c.is_ascii_digit()
        }
    }
}

fn num_cmp_get_unit(unit: Option<char>) -> u8 {
    match unit {
        Some('K' | 'k') => 1,
        Some('M') => 2,
        Some('G') => 3,
        Some('T') => 4,
        Some('P') => 5,
        Some('E') => 6,
        Some('Z') => 7,
        Some('Y') => 8,
        Some(_) | None => 0,
    }
}

/// 根据人类数字比较规则比较两个数字。
/// SI 单位优先于实际值（即 2000M < 1G）。
pub fn num_cmp_human_numeric_str_cmp(
    (a, a_info): (&str, &NumInfo),
    (b, b_info): (&str, &NumInfo),
) -> Ordering {
    // 1. Sign
    if a_info.sign != b_info.sign {
        return a_info.sign.cmp(&b_info.sign);
    }
    // 2. Unit
    let a_unit = num_cmp_get_unit(a.chars().next_back());
    let b_unit = num_cmp_get_unit(b.chars().next_back());
    let ordering = a_unit.cmp(&b_unit);

    match ordering {
        Ordering::Equal => numeric_str_cmp((a, a_info), (b, b_info)),
        _ => {
            if a_info.sign == NumSign::Negative {
                ordering.reverse()
            } else {
                ordering
            }
        }
    }
}

/// 将两个数字作为字符串进行比较，而不先将其解析为数字。这样做的性能会更好，也能更精确地处理数字。
/// 需要使用 NumInfo 为大多数数字提供快速路径。
#[inline(always)]
pub fn numeric_str_cmp((a, a_info): (&str, &NumInfo), (b, b_info): (&str, &NumInfo)) -> Ordering {
    // 检查符号是否有差异
    if a_info.sign != b_info.sign {
        return a_info.sign.cmp(&b_info.sign);
    }

    // 检查指数是否有差异
    let ordering = if a_info.exponent != b_info.exponent && !a.is_empty() && !b.is_empty() {
        a_info.exponent.cmp(&b_info.exponent)
    } else {
        // 从前面走过的字符，直到我们发现差异
        let mut a_chars = a.chars().filter(char::is_ascii_digit);
        let mut b_chars = b.chars().filter(char::is_ascii_digit);
        loop {
            let a_next = a_chars.next();
            let b_next = b_chars.next();
            match (a_next, b_next) {
                (None, None) => break Ordering::Equal,
                (Some(c), None) => {
                    let is_all_zeros = a_chars.all(|c| c == '0');
                    let comparison_result = if c == '0' && is_all_zeros {
                        Ordering::Equal
                    } else {
                        Ordering::Greater
                    };

                    break comparison_result;
                }
                (None, Some(c)) => {
                    let is_all_zeros = b_chars.all(|c| c == '0');
                    let comparison_result = if c == '0' && is_all_zeros {
                        Ordering::Equal
                    } else {
                        Ordering::Less
                    };

                    break comparison_result;
                }
                (Some(a_char), Some(b_char)) => {
                    let ord = a_char.cmp(&b_char);
                    if ord != Ordering::Equal {
                        break ord;
                    }
                }
            }
        }
    };

    match a_info.sign == NumSign::Negative {
        true => ordering.reverse(),
        false => ordering,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_is_invalid_char_digit() {
            let mut had_decimal_pt = false;
            let settings = NumInfoParseSettings::default();
            assert_eq!(
                NumInfo::is_invalid_char('1', &mut had_decimal_pt, &settings),
                false,
                "Digits should be valid"
            );
        }

        #[test]
        fn test_is_invalid_char_decimal_point() {
            let mut had_decimal_pt = false;
            let settings = NumInfoParseSettings {
                decimal_pt: Some('.'),
                ..Default::default()
            };
            assert_eq!(
                NumInfo::is_invalid_char('.', &mut had_decimal_pt, &settings),
                false,
                "First decimal point should be valid"
            );
            assert!(
                had_decimal_pt,
                "Decimal point flag should be set after parsing a decimal point"
            );
        }

        #[test]
        fn test_is_invalid_char_repeated_decimal_point() {
            let mut had_decimal_pt = true; // Simulate that we've already encountered a decimal point
            let settings = NumInfoParseSettings {
                decimal_pt: Some('.'),
                ..Default::default()
            };
            assert_eq!(
                NumInfo::is_invalid_char('.', &mut had_decimal_pt, &settings),
                true,
                "Repeated decimal point should be invalid"
            );
        }

        #[test]
        fn test_is_invalid_char_non_digit_non_decimal() {
            let mut had_decimal_pt = false;
            let settings = NumInfoParseSettings {
                decimal_pt: Some('.'),
                ..Default::default()
            };
            assert_eq!(
                NumInfo::is_invalid_char('a', &mut had_decimal_pt, &settings),
                true,
                "Non-digit, non-decimal characters should be invalid"
            );
        }

        #[test]
        fn test_is_invalid_char_thousands_separator() {
            let mut had_decimal_pt = false;
            let settings = NumInfoParseSettings {
                thousands_separator: Some(','),
                decimal_pt: Some('.'),
                ..Default::default()
            };
            // Thousands separator should not affect the validity of other characters
            assert_eq!(
                NumInfo::is_invalid_char(',', &mut had_decimal_pt, &settings),
                true,
                "Comma should not be considered invalid if it is set as a thousands separator"
            );
        }

        #[test]
        fn test_is_invalid_char_with_alternate_decimal_point() {
            let mut had_decimal_pt = false;
            let settings = NumInfoParseSettings {
                decimal_pt: Some(','),
                ..Default::default()
            };
            assert_eq!(
                NumInfo::is_invalid_char(',', &mut had_decimal_pt, &settings),
                false,
                "Comma as a decimal point should be valid"
            );
            assert!(
                had_decimal_pt,
                "Decimal point flag should be set after parsing a comma as a decimal point"
            );
        }

        #[test]
        fn test_is_invalid_char_with_control_chars() {
            let mut had_decimal_pt = false;
            let settings = NumInfoParseSettings::default();
            assert_eq!(
                NumInfo::is_invalid_char('\n', &mut had_decimal_pt, &settings),
                true,
                "Control characters should be invalid"
            );
            assert_eq!(
                NumInfo::is_invalid_char('\t', &mut had_decimal_pt, &settings),
                true,
                "Control characters should be invalid"
            );
        }

        #[test]
        fn test_is_invalid_char_with_whitespace() {
            let mut had_decimal_pt = false;
            let settings = NumInfoParseSettings::default();
            assert_eq!(
                NumInfo::is_invalid_char(' ', &mut had_decimal_pt, &settings),
                true,
                "Whitespace should be invalid"
            );
        }

        #[test]
        fn test_is_invalid_char_with_special_characters() {
            let mut had_decimal_pt = false;
            let settings = NumInfoParseSettings::default();
            assert_eq!(
                NumInfo::is_invalid_char('$', &mut had_decimal_pt, &settings),
                true,
                "Special characters like '$' should be invalid"
            );
            assert_eq!(
                NumInfo::is_invalid_char('&', &mut had_decimal_pt, &settings),
                true,
                "Special characters like '&' should be invalid"
            );
        }
    
        #[test]
        fn test_is_invalid_char_with_numeric_and_special_mixture() {
            let mut had_decimal_pt = false;
            let settings = NumInfoParseSettings::default();
            assert_eq!(
                NumInfo::is_invalid_char('3', &mut had_decimal_pt, &settings),
                false,
                "Digits should be valid"
            );
            assert_eq!(
                NumInfo::is_invalid_char('%', &mut had_decimal_pt, &settings),
                true,
                "Special characters like '%' immediately after a digit should be invalid"
            );
        }

        #[test]
        fn test_number_with_multiple_decimal_points() {
            let settings = NumInfoParseSettings {
                decimal_pt: Some('.'),
                ..Default::default()
            };
            // Expected to handle or reject multiple decimal points correctly
            let (num_info, range) = NumInfo::parse("12.34.56", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 1,
                    sign: NumSign::Positive,
                }
            ); // Parsing stops at the second decimal point
            assert_eq!(range, 0..5);
        }

        #[test]
        fn test_number_with_non_numeric_suffix() {
            let settings = NumInfoParseSettings::default();
            // Parsing should stop at the first invalid character
            let (num_info, range) = NumInfo::parse("789xyz", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 2,
                    sign: NumSign::Positive,
                }
            );
            assert_eq!(range, 0..3);
        }

        #[test]
        fn test_number_with_spaces_inside() {
            let settings = NumInfoParseSettings::default();
            // Spaces inside numbers are not typically allowed unless specified as thousands separators
            let (num_info, range) = NumInfo::parse("1 234", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 0,
                    sign: NumSign::Positive,
                }
            ); // Parsing should ideally stop before the space
            assert_eq!(range, 0..1);
        }

        #[test]
        fn test_number_with_si_units_and_decimal() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                decimal_pt: Some('.'),
                ..Default::default()
            };
            let (num_info, range) = NumInfo::parse("3.14M", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 0,
                    sign: NumSign::Positive,
                }
            ); // Adjust based on your implementation details
            assert_eq!(range, 0..5);
        }

        #[test]
        fn test_number_with_invalid_thousands_separator() {
            let settings = NumInfoParseSettings {
                thousands_separator: Some('\''),
                ..Default::default()
            };
            let (num_info, range) = NumInfo::parse("1'234'567", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 6,
                    sign: NumSign::Positive,
                }
            );
            assert_eq!(range, 0..9);
        }

        #[test]
        fn test_number_with_multiple_signs() {
            let settings = NumInfoParseSettings::default();
            // Multiple signs should result in invalid parsing or default to the first applicable sign
            let (num_info, range) = NumInfo::parse("+-123", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 0,
                    sign: NumSign::Positive,
                }
            );
            assert_eq!(range, 0..0);
        }

        #[test]
        fn test_number_with_leading_and_trailing_whitespace() {
            let settings = NumInfoParseSettings::default();
            let (num_info, range) = NumInfo::parse("  4567  ", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 3,
                    sign: NumSign::Positive,
                }
            );
            assert_eq!(range, 2..6);
        }

        #[test]
        fn test_full_zero_input() {
            let settings = NumInfoParseSettings::default();
            let (num_info, range) = NumInfo::parse("0000", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 0,
                    sign: NumSign::Positive,
                }
            ); // Adjust if you handle full zero input differently
            assert_eq!(range, 4..4); // Range shows where parsing effectively ended
        }

        #[test]
        fn test_simple_number() {
            let settings = NumInfoParseSettings::default();
            let (num_info, range) = NumInfo::parse("123", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 2,
                    sign: NumSign::Positive,
                }
            );
            assert_eq!(range, 0..3);
        }


    }
}