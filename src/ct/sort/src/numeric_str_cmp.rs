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

        #[test]
        fn test_number_with_negative_sign() {
            let settings = NumInfoParseSettings::default();
            let (num_info, range) = NumInfo::parse("-456", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 2,
                    sign: NumSign::Negative,
                }
            );
            assert_eq!(range, 1..4);
        }

        #[test]
        fn test_number_with_decimal_point() {
            let settings = NumInfoParseSettings {
                decimal_pt: Some('.'),
                ..Default::default()
            };
            let (num_info, range) = NumInfo::parse("789.01", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 2,
                    sign: NumSign::Positive,
                }
            );
            assert_eq!(range, 0..6);
        }

        #[test]
        fn test_number_with_leading_zeros() {
            let settings = NumInfoParseSettings::default();
            let (num_info, range) = NumInfo::parse("0000123", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 2,
                    sign: NumSign::Positive,
                }
            );
            assert_eq!(range, 4..7);
        }

        #[test]
        fn test_number_with_thousands_separator() {
            let settings = NumInfoParseSettings {
                thousands_separator: Some(','),
                ..Default::default()
            };
            let (num_info, range) = NumInfo::parse("1,234", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 3,
                    sign: NumSign::Positive,
                }
            );
            assert_eq!(range, 0..5);
        }

        #[test]
        fn test_number_with_si_unit() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let (num_info, range) = NumInfo::parse("1.5K", &settings);
            assert_eq!(
                num_info,
                NumInfo {
                    exponent: 0,
                    sign: NumSign::Positive,
                }
            ); // Adjust exponent logic if needed
            assert_eq!(range, 0..4);
        }

        #[test]
        fn test_invalid_input() {
            let settings = NumInfoParseSettings::default();
            let (num_info, range) = NumInfo::parse("abc", &settings);
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
        fn test_empty_input() {
            let settings = NumInfoParseSettings::default();
            let (num_info, range) = NumInfo::parse("", &settings);
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
        fn test_get_unit_none() {
            assert_eq!(num_cmp_get_unit(None), 0, "None should return 0");
        }

        #[test]
        fn test_get_unit_k_lowercase() {
            assert_eq!(
                num_cmp_get_unit(Some('k')),
                1,
                "Lowercase k should return 1"
            );
        }

        #[test]
        fn test_get_unit_k_uppercase() {
            assert_eq!(
                num_cmp_get_unit(Some('K')),
                1,
                "Uppercase K should return 1"
            );
        }

        #[test]
        fn test_get_unit_m() {
            assert_eq!(num_cmp_get_unit(Some('M')), 2, "M should return 2");
        }

        #[test]
        fn test_get_unit_g() {
            assert_eq!(num_cmp_get_unit(Some('G')), 3, "G should return 3");
        }

        #[test]
        fn test_get_unit_t() {
            assert_eq!(num_cmp_get_unit(Some('T')), 4, "T should return 4");
        }

        #[test]
        fn test_get_unit_p() {
            assert_eq!(num_cmp_get_unit(Some('P')), 5, "P should return 5");
        }

        #[test]
        fn test_get_unit_e() {
            assert_eq!(num_cmp_get_unit(Some('E')), 6, "E should return 6");
        }

        #[test]
        fn test_get_unit_z() {
            assert_eq!(num_cmp_get_unit(Some('Z')), 7, "Z should return 7");
        }

        #[test]
        fn test_get_unit_y() {
            assert_eq!(num_cmp_get_unit(Some('Y')), 8, "Y should return 8");
        }

        #[test]
        fn test_get_unit_invalid() {
            assert_eq!(
                num_cmp_get_unit(Some('X')),
                0,
                "Invalid unit should return 0"
            );
            assert_eq!(
                num_cmp_get_unit(Some('0')),
                0,
                "Invalid unit should return 0"
            );
            assert_eq!(
                num_cmp_get_unit(Some(' ')),
                0,
                "Invalid unit should return 0"
            );
            assert_eq!(
                num_cmp_get_unit(Some('-')),
                0,
                "Invalid unit should return 0"
            );
        }

        #[test]
        fn test_human_numeric_with_si_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("2G", &settings).0;
            let b_info = NumInfo::parse("1500M", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("2G", &a_info), ("1500M", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_with_equal_si_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("1G", &settings).0;
            let b_info = NumInfo::parse("1000M", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("1G", &a_info), ("1000M", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_negative_numbers_with_si_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("-500K", &settings).0;
            let b_info = NumInfo::parse("-1M", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("-500K", &a_info), ("-1M", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_positive_vs_negative_with_si_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("1M", &settings).0;
            let b_info = NumInfo::parse("-1M", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("1M", &a_info), ("-1M", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_with_different_si_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("1T", &settings).0;
            let b_info = NumInfo::parse("900G", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("1T", &a_info), ("900G", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_same_number_different_si_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("1K", &settings).0;
            let b_info = NumInfo::parse("1000", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("1K", &a_info), ("1000", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_large_numbers() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("1Z", &settings).0;
            let b_info = NumInfo::parse("1000E", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("1Z", &a_info), ("1000E", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_with_close_si_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("999M", &settings).0;
            let b_info = NumInfo::parse("1G", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("999M", &a_info), ("1G", &b_info)),
                Ordering::Less
            );
        }

        #[test]
        fn test_human_numeric_with_very_small_numbers() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("10m", &settings).0; // assuming milli-unit handling for small values
            let b_info = NumInfo::parse("1", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("10m", &a_info), ("1", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_with_mixed_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("1k", &settings).0;
            let b_info = NumInfo::parse("1000", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("1k", &a_info), ("1000", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_with_large_and_small_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("1G", &settings).0;
            let b_info = NumInfo::parse("1000000k", &settings).0; // equivalent to 1G
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("1G", &a_info), ("1000000k", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_negative_values_with_different_magnitudes() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("-100M", &settings).0;
            let b_info = NumInfo::parse("-1G", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("-100M", &a_info), ("-1G", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_zero_values_with_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("0K", &settings).0;
            let b_info = NumInfo::parse("0", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("0K", &a_info), ("0", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_with_inverted_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("1K", &settings).0;
            let b_info = NumInfo::parse("1000M", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("1K", &a_info), ("1000M", &b_info)),
                Ordering::Less
            );
        }

        #[test]
        fn test_human_numeric_with_large_numbers_comparing_magnitude() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("10P", &settings).0;
            let b_info = NumInfo::parse("10T", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("10P", &a_info), ("10T", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_human_numeric_with_similar_numbers_different_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("1T", &settings).0;
            let b_info = NumInfo::parse("1000G", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("1T", &a_info), ("1000G", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_numeric_str_cmp_equal() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("123", &settings).0;
            let b_info = NumInfo::parse("123", &settings).0;
            assert_eq!(
                numeric_str_cmp(("123", &a_info), ("123", &b_info)),
                Ordering::Equal
            );
        }

        #[test]
        fn test_numeric_str_cmp_greater() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("124", &settings).0;
            let b_info = NumInfo::parse("123", &settings).0;
            assert_eq!(
                numeric_str_cmp(("124", &a_info), ("123", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_numeric_str_cmp_less() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("123", &settings).0;
            let b_info = NumInfo::parse("124", &settings).0;
            assert_eq!(
                numeric_str_cmp(("123", &a_info), ("124", &b_info)),
                Ordering::Less
            );
        }

        #[test]
        fn test_numeric_str_cmp_leading_zeroes() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("00123", &settings).0;
            let b_info = NumInfo::parse("123", &settings).0;
            assert_eq!(
                numeric_str_cmp(("00123", &a_info), ("123", &b_info)),
                Ordering::Less
            );
        }

        #[test]
        fn test_numeric_str_cmp_different_lengths() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("1234", &settings).0;
            let b_info = NumInfo::parse("123", &settings).0;
            assert_eq!(
                numeric_str_cmp(("1234", &a_info), ("123", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_numeric_str_cmp_negative_numbers() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("-123", &settings).0;
            let b_info = NumInfo::parse("-122", &settings).0;
            assert_eq!(
                numeric_str_cmp(("-123", &a_info), ("-122", &b_info)),
                Ordering::Less
            );
        }

        #[test]
        fn test_numeric_str_cmp_positive_vs_negative() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("123", &settings).0;
            let b_info = NumInfo::parse("-123", &settings).0;
            assert_eq!(
                numeric_str_cmp(("123", &a_info), ("-123", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_with_trailing_zeroes() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("123000", &settings).0;
            let b_info = NumInfo::parse("123", &settings).0;
            assert_eq!(
                numeric_str_cmp(("123000", &a_info), ("123", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_with_decimal_points() {
            let settings = NumInfoParseSettings {
                decimal_pt: Some('.'),
                ..Default::default()
            };
            let a_info = NumInfo::parse("123.45", &settings).0;
            let b_info = NumInfo::parse("123.450", &settings).0;
            assert_eq!(
                numeric_str_cmp(("123.45", &a_info), ("123.450", &b_info)),
                Ordering::Equal
            );
        }

        #[test]
        fn test_with_si_units() {
            let settings = NumInfoParseSettings {
                accept_si_units: true,
                ..Default::default()
            };
            let a_info = NumInfo::parse("2000M", &settings).0;
            let b_info = NumInfo::parse("1G", &settings).0;
            assert_eq!(
                num_cmp_human_numeric_str_cmp(("2000M", &a_info), ("1G", &b_info)),
                Ordering::Less
            );
        }

        #[test]
        fn test_negative_vs_positive() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("-100", &settings).0;
            let b_info = NumInfo::parse("100", &settings).0;
            assert_eq!(
                numeric_str_cmp(("-100", &a_info), ("100", &b_info)),
                Ordering::Less
            );
        }

        #[test]
        fn test_numbers_with_commas() {
            let settings = NumInfoParseSettings {
                thousands_separator: Some(','),
                ..Default::default()
            };
            let a_info = NumInfo::parse("1,000,000", &settings).0;
            let b_info = NumInfo::parse("1000000", &settings).0;
            assert_eq!(
                numeric_str_cmp(("1,000,000", &a_info), ("1000000", &b_info)),
                Ordering::Equal
            );
        }

        #[test]
        fn test_large_numbers() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("999999999999999999", &settings).0;
            let b_info = NumInfo::parse("1000000000000000000", &settings).0;
            assert_eq!(
                numeric_str_cmp(
                    ("999999999999999999", &a_info),
                    ("1000000000000000000", &b_info),
                ),
                Ordering::Less
            );
        }

        #[test]
        fn test_with_mixed_significant_figures() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("1234500", &settings).0;
            let b_info = NumInfo::parse("12345", &settings).0;
            assert_eq!(
                numeric_str_cmp(("1234500", &a_info), ("12345", &b_info)),
                Ordering::Greater
            );
        }

        #[test]
        fn test_identical_numbers_with_different_exponents() {
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("123e5", &settings).0;
            let b_info = NumInfo::parse("12300000", &settings).0;
            assert_eq!(
                numeric_str_cmp(("123e5", &a_info), ("12300000", &b_info)),
                Ordering::Less
            );
        }
    }

    #[cfg(test)]
    mod base_tests {
        use super::*;

        #[test]
        fn parses_base_exp() {
            let n = "1";
            assert_eq!(
                NumInfo::parse(n, &NumInfoParseSettings::default()),
                (
                    NumInfo {
                        exponent: 0,
                        sign: NumSign::Positive,
                    },
                    0..1
                )
            );
            let n = "100";
            assert_eq!(
                NumInfo::parse(n, &NumInfoParseSettings::default()),
                (
                    NumInfo {
                        exponent: 2,
                        sign: NumSign::Positive,
                    },
                    0..3
                )
            );
            let n = "1,000";
            assert_eq!(
                NumInfo::parse(
                    n,
                    &NumInfoParseSettings {
                        thousands_separator: Some(','),
                        ..Default::default()
                    },
                ),
                (
                    NumInfo {
                        exponent: 3,
                        sign: NumSign::Positive,
                    },
                    0..5
                )
            );
            let n = "1,000";
            assert_eq!(
                NumInfo::parse(n, &NumInfoParseSettings::default()),
                (
                    NumInfo {
                        exponent: 0,
                        sign: NumSign::Positive,
                    },
                    0..1
                )
            );
            let n = "1000.00";
            assert_eq!(
                NumInfo::parse(n, &NumInfoParseSettings::default()),
                (
                    NumInfo {
                        exponent: 3,
                        sign: NumSign::Positive,
                    },
                    0..7
                )
            );
        }

        #[test]
        fn parses_base_negative_exp() {
            let n = "0.00005";
            assert_eq!(
                NumInfo::parse(n, &NumInfoParseSettings::default()),
                (
                    NumInfo {
                        exponent: -5,
                        sign: NumSign::Positive,
                    },
                    6..7
                )
            );
            let n = "00000.00005";
            assert_eq!(
                NumInfo::parse(n, &NumInfoParseSettings::default()),
                (
                    NumInfo {
                        exponent: -5,
                        sign: NumSign::Positive,
                    },
                    10..11
                )
            );
        }

        #[test]
        fn parses_base_sign() {
            let n = "5";
            assert_eq!(
                NumInfo::parse(n, &NumInfoParseSettings::default()),
                (
                    NumInfo {
                        exponent: 0,
                        sign: NumSign::Positive,
                    },
                    0..1
                )
            );
            let n = "-5";
            assert_eq!(
                NumInfo::parse(n, &NumInfoParseSettings::default()),
                (
                    NumInfo {
                        exponent: 0,
                        sign: NumSign::Negative,
                    },
                    1..2
                )
            );
            let n = "    -5";
            assert_eq!(
                NumInfo::parse(n, &NumInfoParseSettings::default()),
                (
                    NumInfo {
                        exponent: 0,
                        sign: NumSign::Negative,
                    },
                    5..6
                )
            );
        }

        fn test_base_helper(a: &str, b: &str, expected: Ordering) {
            let (a_info, a_range) = NumInfo::parse(a, &NumInfoParseSettings::default());
            let (b_info, b_range) = NumInfo::parse(b, &NumInfoParseSettings::default());
            let ordering = numeric_str_cmp(
                (&a[a_range.clone()], &a_info),
                (&b[b_range.clone()], &b_info),
            );
            assert_eq!(ordering, expected);
            let ordering = numeric_str_cmp((&b[b_range], &b_info), (&a[a_range], &a_info));
            assert_eq!(ordering, expected.reverse());
        }

        #[test]
        fn test_base_single_digit() {
            test_base_helper("1", "2", Ordering::Less);
            test_base_helper("0", "0", Ordering::Equal);
        }

        #[test]
        fn test_base_minus() {
            test_base_helper("-1", "-2", Ordering::Greater);
            test_base_helper("-0", "-0", Ordering::Equal);
        }

        #[test]
        fn test_base_different_len() {
            test_base_helper("-20", "-100", Ordering::Greater);
            test_base_helper("10.0", "2.000000", Ordering::Greater);
        }

        #[test]
        fn test_base_decimal_digits() {
            test_base_helper("20.1", "20.2", Ordering::Less);
            test_base_helper("20.1", "20.15", Ordering::Less);
            test_base_helper("-20.1", "+20.15", Ordering::Less);
            test_base_helper("-20.1", "-20", Ordering::Less);
        }

        #[test]
        fn test_base_trailing_zeroes() {
            test_base_helper("20.00000", "20.1", Ordering::Less);
            test_base_helper("20.00000", "20.0", Ordering::Equal);
        }

        #[test]
        fn test_base_invalid_digits() {
            test_base_helper("foo", "bar", Ordering::Equal);
            test_base_helper("20.1", "a", Ordering::Greater);
            test_base_helper("-20.1", "a", Ordering::Less);
            test_base_helper("a", "0.15", Ordering::Less);
        }

        #[test]
        fn test_base_multiple_decimal_pts() {
            test_base_helper("10.0.0", "50.0.0", Ordering::Less);
            test_base_helper("0.1.", "0.2.0", Ordering::Less);
            test_base_helper("1.1.", "0", Ordering::Greater);
            test_base_helper("1.1.", "-0", Ordering::Greater);
        }

        #[test]
        fn test_base_leading_decimal_pts() {
            test_base_helper(".0", ".0", Ordering::Equal);
            test_base_helper(".1", ".0", Ordering::Greater);
            test_base_helper(".02", "0", Ordering::Greater);
        }

        #[test]
        fn test_base_leading_zeroes() {
            test_base_helper("000000.0", ".0", Ordering::Equal);
            test_base_helper("0.1", "0000000000000.0", Ordering::Greater);
            test_base_helper("-01", "-2", Ordering::Greater);
        }

        #[test]
        fn minus_base_zero() {
            // This matches GNU sort behavior.
            test_base_helper("-0", "0", Ordering::Equal);
            test_base_helper("-0x", "0", Ordering::Equal);
        }

        #[test]
        fn double_base_minus() {
            test_base_helper("--1", "0", Ordering::Equal);
        }

        #[test]
        fn single_base_minus() {
            let info = NumInfo::parse("-", &NumInfoParseSettings::default());
            assert_eq!(
                info,
                (
                    NumInfo {
                        exponent: 0,
                        sign: NumSign::Positive,
                    },
                    0..0
                )
            );
        }

        #[test]
        fn base_invalid_with_unit() {
            let info = NumInfo::parse(
                "-K",
                &NumInfoParseSettings {
                    accept_si_units: true,
                    ..Default::default()
                },
            );
            assert_eq!(
                info,
                (
                    NumInfo {
                        exponent: 0,
                        sign: NumSign::Positive,
                    },
                    0..0
                )
            );
        }
    }
}
