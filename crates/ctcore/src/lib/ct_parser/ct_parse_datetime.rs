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

//! GNU coreutils兼容的日期时间解析器
//!
//! 这个模块提供了与GNU coreutils parse-datetime兼容的日期时间解析功能，
//! 支持自然语言日期表达式，如"next Friday"、"last Monday"等。
//!
//! 基于GNU coreutils-9.4/lib/parse-datetime.y的实现。

use crate::ct_error::{CTResult, CtSimpleError};
use chrono::{DateTime, Datelike, Duration, Local, Weekday};

/// 日期时间解析错误
#[derive(Debug, Clone)]
pub struct ParseDateTimeError {
    pub message: String,
}

impl std::fmt::Display for ParseDateTimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParseDateTimeError {}

/// 解析日期时间字符串，支持GNU coreutils兼容的格式
///
/// 支持的格式包括：
/// - 星期几名称: "monday", "friday", "saturday" 等
/// - 带修饰的星期几: "next monday", "last friday", "this wednesday" 等
/// - 相对时间: "tomorrow", "yesterday", "today" 等
/// - 绝对日期: "2023-12-25", "@1234567890" 等
/// - 以及其他GNU parse_datetime支持的格式
///
/// # 参数
/// * `input` - 要解析的日期时间字符串
/// * `reference_time` - 用作相对时间计算基准的参考时间
///
/// # 返回值
/// 成功时返回解析后的DateTime<Local>，失败时返回ParseDateTimeError
///
/// # 示例
/// ```rust
/// use chrono::Local;
/// use ctcore::ct_parse_datetime::parse_datetime_gnu_compat;
///
/// let now = Local::now();
/// let result = parse_datetime_gnu_compat("next friday", now);
/// assert!(result.is_ok());
/// ```
pub fn parse_datetime_gnu_compat(
    input: &str,
    reference_time: DateTime<Local>,
) -> Result<DateTime<Local>, ParseDateTimeError> {
    let input_lower = input.trim().to_lowercase();

    // 1. 尝试解析星期几相关的表达式
    if let Some(dt) = parse_weekday_expression(&input_lower, reference_time) {
        return Ok(dt);
    }

    // 2. 尝试解析相对时间表达式
    if let Some(dt) = parse_relative_time(&input_lower, reference_time) {
        return Ok(dt);
    }

    // 3. 回退到现有的parse_datetime crate
    match parse_datetime::parse_datetime_at_date(reference_time, input) {
        Ok(dt) => Ok(dt.with_timezone(&Local)),
        Err(_) => Err(ParseDateTimeError {
            message: format!("Unable to parse date: {input}"),
        }),
    }
}

/// 解析包含星期几名称的表达式
fn parse_weekday_expression(
    input: &str,
    reference_time: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let parts: Vec<&str> = input.split_whitespace().collect();

    if parts.is_empty() {
        return None;
    }

    // 星期几名称映射 (基于GNU coreutils parse-datetime.y)
    let weekdays = [
        ("sunday", Weekday::Sun),
        ("monday", Weekday::Mon),
        ("tuesday", Weekday::Tue),
        ("tues", Weekday::Tue),
        ("wednesday", Weekday::Wed),
        ("wednes", Weekday::Wed),
        ("thursday", Weekday::Thu),
        ("thur", Weekday::Thu),
        ("thurs", Weekday::Thu),
        ("friday", Weekday::Fri),
        ("saturday", Weekday::Sat),
    ];

    // 查找星期几
    let mut target_weekday = None;
    for (name, weekday) in &weekdays {
        if parts.contains(name) {
            target_weekday = Some(*weekday);
            break;
        }
    }

    let target_weekday = target_weekday?;
    let current_weekday = reference_time.weekday();

    // 根据修饰词计算目标日期
    let days_offset = match parts.first() {
        Some(&"next") => {
            // "next weekday" - GNU语义：如果目标星期几距离超过1天，则指本周；否则指下周
            let days = (target_weekday.num_days_from_monday() as i32
                - current_weekday.num_days_from_monday() as i32
                + 7)
                % 7;
            if days == 0 {
                7 // 如果今天就是目标星期几，下一个是下周
            } else if days == 1 {
                7 + 1 // 如果明天是目标星期几，下一个是下周
            } else {
                days // 如果目标星期几在本周后面几天，就是本周
            }
        }
        Some(&"last") => {
            // "last Friday" - 上一个星期五（不包括今天）
            let days = (current_weekday.num_days_from_monday() as i32
                - target_weekday.num_days_from_monday() as i32
                + 7)
                % 7;
            if days == 0 { -7 } else { -days }
        }
        Some(&"this") => {
            // "this Friday" - 本周的星期五
            let days = target_weekday.num_days_from_monday() as i32
                - current_weekday.num_days_from_monday() as i32;
            if days < 0 { days + 7 } else { days }
        }
        _ => {
            // 只有星期几名称，例如 "Friday"
            // GNU的行为：如果今天是该星期几则返回今天，否则返回下一个该星期几
            let days = target_weekday.num_days_from_monday() as i32
                - current_weekday.num_days_from_monday() as i32;
            if days < 0 { days + 7 } else { days }
        }
    };

    // 计算目标日期并设置时间为午夜00:00:00（匹配GNU coreutils行为）
    Duration::try_days(days_offset as i64)
        .and_then(|duration| reference_time.checked_add_signed(duration))
        .map(|dt| {
            dt.date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap()
        })
}

/// 解析相对时间表达式
fn parse_relative_time(input: &str, reference_time: DateTime<Local>) -> Option<DateTime<Local>> {
    // 基于GNU coreutils relative_time_table的映射
    let relative_times = [("tomorrow", 1), ("yesterday", -1), ("today", 0), ("now", 0)];

    for (name, days_offset) in &relative_times {
        if input == *name {
            return Duration::try_days(*days_offset as i64)
                .and_then(|duration| reference_time.checked_add_signed(duration));
        }
    }

    None
}

/// 为兼容性提供的简化接口，与filetime::FileTime一起使用
pub fn parse_datetime_to_filetime(
    input: &str,
    reference_time: DateTime<Local>,
) -> CTResult<filetime::FileTime> {
    match parse_datetime_gnu_compat(input, reference_time) {
        Ok(dt) => Ok(filetime::FileTime::from_unix_time(
            dt.timestamp(),
            dt.timestamp_subsec_nanos(),
        )),
        Err(e) => Err(CtSimpleError::new(1, e.message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    #[test]
    fn test_parse_weekday_simple() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap(); // Thursday

        // Test simple weekday names
        let friday = parse_datetime_gnu_compat("friday", ref_time).unwrap();
        assert_eq!(friday.weekday(), Weekday::Fri);

        let monday = parse_datetime_gnu_compat("monday", ref_time).unwrap();
        assert_eq!(monday.weekday(), Weekday::Mon);
    }

    #[test]
    fn test_parse_weekday_with_modifiers() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap(); // Thursday

        // Test next/last/this modifiers
        let next_friday = parse_datetime_gnu_compat("next friday", ref_time).unwrap();
        assert_eq!(next_friday.weekday(), Weekday::Fri);
        assert!(next_friday > ref_time);

        let last_monday = parse_datetime_gnu_compat("last monday", ref_time).unwrap();
        assert_eq!(last_monday.weekday(), Weekday::Mon);
        assert!(last_monday < ref_time);

        let this_saturday = parse_datetime_gnu_compat("this saturday", ref_time).unwrap();
        assert_eq!(this_saturday.weekday(), Weekday::Sat);
    }

    #[test]
    fn test_parse_relative_time() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        let tomorrow = parse_datetime_gnu_compat("tomorrow", ref_time).unwrap();
        assert_eq!(tomorrow.day(), 25);

        let yesterday = parse_datetime_gnu_compat("yesterday", ref_time).unwrap();
        assert_eq!(yesterday.day(), 23);

        let today = parse_datetime_gnu_compat("today", ref_time).unwrap();
        assert_eq!(today.day(), 24);
    }

    #[test]
    fn test_parse_abbreviations() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        // Test abbreviations supported by GNU
        let tues = parse_datetime_gnu_compat("tues", ref_time).unwrap();
        assert_eq!(tues.weekday(), Weekday::Tue);

        let thurs = parse_datetime_gnu_compat("thurs", ref_time).unwrap();
        assert_eq!(thurs.weekday(), Weekday::Thu);
    }

    #[test]
    fn test_case_insensitive() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        // Test case insensitivity
        let friday_upper = parse_datetime_gnu_compat("FRIDAY", ref_time).unwrap();
        let friday_mixed = parse_datetime_gnu_compat("Friday", ref_time).unwrap();
        let friday_lower = parse_datetime_gnu_compat("friday", ref_time).unwrap();

        assert_eq!(friday_upper.weekday(), Weekday::Fri);
        assert_eq!(friday_mixed.weekday(), Weekday::Fri);
        assert_eq!(friday_lower.weekday(), Weekday::Fri);
    }

    #[test]
    fn test_invalid_input() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        // Test invalid inputs
        let result = parse_datetime_gnu_compat("invalid_day", ref_time);
        assert!(result.is_err());

        let result = parse_datetime_gnu_compat("", ref_time);
        assert!(result.is_err());
    }

    #[test]
    fn test_fallback_to_parse_datetime() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        // Test that existing parse_datetime functionality still works
        let result = parse_datetime_gnu_compat("1 week", ref_time);
        assert!(result.is_ok());

        let result = parse_datetime_gnu_compat("2023-12-25", ref_time);
        assert!(result.is_ok());
    }
}
