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

//! 解析字符串表示的持续时间。
//!
//! 使用`from_str`函数从字符串解析出一个`Duration`实例。

use std::time::Duration;

use crate::ct_display::Quotable;

/// 从字符串解析持续时间。
///
/// 字符串可以只包含一个数字，如 "123" 或 "4.5"，也可以包含一个带有单位标识符的数字，如 "123s" 表示一百二十三秒或 "4.5d" 表示四天半。如果没有指定单位，则假定单位为秒。
///
/// 允许的后缀包括
///
/// * "s" 表示秒，
/// * "m" 表示分钟，
/// * "h" 表示小时，
/// * "d" 表示天。
///
/// 该函数使用 Duration::saturating_mul 计算秒数，因此不会发生溢出。如果发生了溢出，将返回 Duration::MAX。
///
/// # 错误
///
/// 当输入字符串为空、输入不是有效的数字，或者单位标识符无效或未知时，此函数会返回错误。
///
/// # 示例
///
/// ```rust
/// use std::time::Duration;
/// use ctcore::ct_parse_time::ct_from_str;
/// assert_eq!(ct_from_str("123"), Ok(Duration::from_secs(123)));
/// assert_eq!(ct_from_str("2d"), Ok(Duration::from_secs(60 * 60 * 24 * 2)));
/// ```
pub fn ct_from_str(string: &str) -> Result<Duration, String> {
    if string.is_empty() {
        return Err("empty string".to_owned());
    }

    let slice = if let Some(s) = string.get(..string.len() - 1) {
        s
    } else {
        return Err(format!("invalid time interval {}", string.quote()));
    };

    let last_char = string.chars().next_back().unwrap();

    let (numstr, times) = {
        if last_char == 's' {
            (slice, 1)
        } else if last_char == 'm' {
            (slice, 60)
        } else if last_char == 'h' {
            (slice, 3600)
        } else if last_char == 'd' {
            (slice, 86400)
        } else if !last_char.is_alphabetic() {
            (string, 1)
        } else if string == "inf" || string == "infinity" {
            ("inf", 1)
        } else {
            return Err(format!("invalid time interval {}", string.quote()));
        }
    };

    let num = numstr
        .parse::<f64>()
        .map_err(|e| format!("invalid time interval {}: {}", string.quote(), e))?;

    if num < 0. {
        return Err(format!("invalid time interval {}", string.quote()));
    }

    const NANOS_PER_SEC: u32 = 1_000_000_000;
    let whole_secs = num.trunc();
    let nanos = (num.fract() * (NANOS_PER_SEC as f64)).trunc();
    let duration = Duration::new(whole_secs as u64, nanos as u32);
    Ok(duration.saturating_mul(times))
}

#[cfg(test)]
mod tests {

    use crate::ct_parse_time::ct_from_str;
    use std::time::Duration;

    #[test]
    fn test_basic_seconds() {
        assert_eq!(ct_from_str("300s"), Ok(Duration::from_secs(300)));
    }

    #[test]
    fn test_minutes() {
        assert_eq!(ct_from_str("5m"), Ok(Duration::from_secs(300)));
    }

    #[test]
    fn test_hours() {
        assert_eq!(ct_from_str("2h"), Ok(Duration::from_secs(7200)));
    }

    #[test]
    fn test_days() {
        assert_eq!(ct_from_str("1d"), Ok(Duration::from_secs(86400)));
    }

    #[test]
    fn test_fractional_seconds() {
        assert_eq!(ct_from_str("0.5s"), Ok(Duration::new(0, 500_000_000)));
    }

    #[test]
    fn test_fractional_minutes() {
        assert_eq!(ct_from_str("1.5m"), Ok(Duration::from_secs(90)));
    }

    #[test]
    fn test_invalid_number() {
        assert!(ct_from_str("abc").is_err());
    }

    #[test]
    fn test_invalid_suffix() {
        assert!(ct_from_str("10x").is_err());
    }

    #[test]
    fn test_invalid_combination() {
        assert!(ct_from_str("10sd").is_err());
    }

    #[test]
    fn test_empty_string() {
        assert!(ct_from_str("").is_err());
    }

    #[test]
    fn test_no_suffix_with_decimal() {
        assert_eq!(ct_from_str("12.34"), Ok(Duration::new(12, 339_999_999)));
    }

    #[test]
    fn test_large_number() {
        let large_number = "1000000000000000000000";
        assert!(ct_from_str(large_number).is_ok());
    }

    #[test]
    fn test_overflow() {
        let overflow_number = "18446744073709551615.999999999";
        assert_eq!(
            ct_from_str(overflow_number),
            Ok(Duration::from_secs(18446744073709551615))
        );
    }

    #[test]
    fn test_infinity_strings() {
        assert_eq!(ct_from_str("inf"), Ok(Duration::from_secs(u64::MAX)));
        assert_eq!(ct_from_str("infinity"), Ok(Duration::from_secs(u64::MAX)));
    }

    #[test]
    fn test_negative_input() {
        assert!(ct_from_str("-100").is_err());
    }

    #[test]
    fn test_whitespace_handling() {
        assert_eq!(
            ct_from_str(" 300s "),
            Err(String::from(
                "invalid time interval ' 300s ': invalid float literal"
            ))
        );
    }

    #[test]
    fn test_multiple_suffixes() {
        assert!(ct_from_str("10smh").is_err());
    }

    #[test]
    fn test_special_characters() {
        assert!(ct_from_str("10$").is_err());
        assert!(ct_from_str("10#").is_err());
    }

    #[test]
    fn test_large_fractional_part() {
        assert_eq!(ct_from_str("0.0000001s"), Ok(Duration::new(0, 100)));
    }

    #[test]
    fn test_very_large_fractional_seconds() {
        assert_eq!(
            ct_from_str("0.9999999999s"),
            Ok(Duration::new(0, 999_999_999))
        );
    }

    #[test]
    fn test_basic_no_units() {
        assert_eq!(ct_from_str("123"), Ok(Duration::from_secs(123)));
    }

    #[test]
    fn test_basic_units() {
        assert_eq!(ct_from_str("2d"), Ok(Duration::from_secs(60 * 60 * 24 * 2)));
    }

    #[test]
    fn test_basic_saturating_mul() {
        assert_eq!(ct_from_str("9223372036854775808d"), Ok(Duration::MAX));
    }

    #[test]
    fn test_basic_error_empty() {
        assert!(ct_from_str("").is_err());
    }

    #[test]
    fn test_basic_error_invalid_unit() {
        assert!(ct_from_str("123X").is_err());
    }

    #[test]
    fn test_basic_error_multi_bytes_characters() {
        assert!(ct_from_str("10€").is_err());
    }

    #[test]
    fn test_basic_error_invalid_magnitude() {
        assert!(ct_from_str("12abc3s").is_err());
    }

    #[test]
    fn test_basic_negative() {
        assert!(ct_from_str("-1").is_err());
    }

    /// Test that capital letters are not allowed in suffixes.
    #[test]
    fn test_basic_no_capital_letters() {
        assert!(ct_from_str("1S").is_err());
        assert!(ct_from_str("1M").is_err());
        assert!(ct_from_str("1H").is_err());
        assert!(ct_from_str("1D").is_err());
    }
}
