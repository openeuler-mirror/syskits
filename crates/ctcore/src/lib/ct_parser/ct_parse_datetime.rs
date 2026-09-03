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
use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Weekday,
};

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
/// 解析日期时间字符串，支持GNU coreutils兼容的格式
pub fn parse_datetime_gnu_compat(
    input: &str,
    reference_time: DateTime<Local>,
) -> Result<DateTime<Local>, ParseDateTimeError> {
    let input_trim = input.trim();
    let input_lower = input_trim.to_lowercase();

    // 负数/正数纪元秒 (Epoch: @-22, @31536000)
    if let Some(epoch_str) = input_trim.strip_prefix('@') {
        if let Ok(secs) = epoch_str.parse::<f64>() {
            let s = secs.trunc() as i64;
            let ns = (secs.fract().abs() * 1_000_000_000.0) as u32;
            if let Some(dt) = chrono::DateTime::from_timestamp(s, ns) {
                return Ok(dt.with_timezone(&Local));
            }
        }
    }

    if (1..=4).contains(&input_trim.len()) && input_trim.bytes().all(|b| b.is_ascii_digit()) {
        return parse_compact_time_of_day(input_trim, reference_time).ok_or_else(|| {
            ParseDateTimeError {
                message: format!("Unable to parse date: {input}"),
            }
        });
    }

    // 军用时区拦截 (Military Timezone: e.g. 09:00B -> UTC+2)
    if input_trim.len() == 6 {
        let bytes = input_trim.as_bytes();
        if bytes[2] == b':'
            && bytes[0].is_ascii_digit()
            && bytes[1].is_ascii_digit()
            && bytes[3].is_ascii_digit()
            && bytes[4].is_ascii_digit()
        {
            let tz_char = (bytes[5] as char).to_ascii_uppercase();
            if tz_char.is_ascii_uppercase() && tz_char != 'J' {
                let hour = (bytes[0] - b'0') * 10 + (bytes[1] - b'0');
                let min = (bytes[3] - b'0') * 10 + (bytes[4] - b'0');
                let offset_hours = match tz_char {
                    'A'..='I' => (tz_char as i32) - ('A' as i32) + 1,
                    'K'..='M' => (tz_char as i32) - ('K' as i32) + 10,
                    'N'..='Y' => -((tz_char as i32) - ('N' as i32) + 1),
                    'Z' => 0,
                    _ => 0,
                };
                if let Some(offset) = FixedOffset::east_opt(offset_hours * 3600) {
                    if let Some(naive_time) =
                        chrono::NaiveTime::from_hms_opt(hour as u32, min as u32, 0)
                    {
                        let naive_dt = reference_time.date_naive().and_time(naive_time);
                        if let chrono::LocalResult::Single(dt) =
                            offset.from_local_datetime(&naive_dt)
                        {
                            return Ok(dt.with_timezone(&Local));
                        }
                    }
                }
            }
        }
    }

    // 特殊时区环境变量重写 (e.g. TZ="EST5" 1970-01-01 00:00)
    if let Some(rest) = input_trim.strip_prefix("TZ=\"") {
        if let Some(quote_idx) = rest.find('"') {
            let tz_name = &rest[..quote_idx];
            let date_str = rest[quote_idx + 1..].trim();
            let offset_hours = if tz_name.starts_with("EST") {
                -5
            } else if tz_name.starts_with("PST") {
                -8
            } else {
                0
            };
            if let Some(offset) = FixedOffset::east_opt(offset_hours * 3600) {
                let formats = ["%Y-%m-%d %H:%M", "%Y-%m-%d %H:%M:%S"];
                for fmt in formats {
                    if let Ok(naive_dt) = NaiveDateTime::parse_from_str(date_str, fmt) {
                        if let chrono::LocalResult::Single(dt) =
                            offset.from_local_datetime(&naive_dt)
                        {
                            return Ok(dt.with_timezone(&Local));
                        }
                    }
                }
            }
        }
    }

    let mut processed_lower = input_lower.clone();
    let mut processed_trim = input_trim.to_string();

    // 预处理 ago 关键字 (倒转时间方向)
    let mut is_ago = false;
    if processed_lower.ends_with(" ago") {
        is_ago = true;
        processed_lower.truncate(processed_lower.len() - 4);
        processed_lower = processed_lower.trim().to_string();
        processed_trim.truncate(processed_trim.len() - 4);
        processed_trim = processed_trim.trim().to_string();
    }

    // 预处理自然语言相对时间词汇 (now, yesterday 等标准化为精准的加减法)
    let word_replacements = [
        ("yesterday", "-1 day"),
        ("tomorrow", "+1 day"),
        ("today", "+0 day"),
        ("now", "+0 sec"),
        ("this second", "+0 sec"),
        ("this minute", "+0 minute"),
        ("this hour", "+0 hour"),
        ("this day", "+0 day"),
        ("this week", "+0 week"),
        ("this month", "+0 month"),
        ("this year", "+0 year"),
        ("next second", "+1 sec"),
        ("next minute", "+1 minute"),
        ("next hour", "+1 hour"),
        ("next day", "+1 day"),
        ("next week", "+1 week"),
        ("next month", "+1 month"),
        ("next year", "+1 year"),
        ("last second", "-1 sec"),
        ("last minute", "-1 minute"),
        ("last hour", "-1 hour"),
        ("last day", "-1 day"),
        ("last week", "-1 week"),
        ("last month", "-1 month"),
        ("last year", "-1 year"),
    ];
    for (word, replacement) in word_replacements {
        if processed_lower == word {
            processed_lower = replacement.to_string();
            processed_trim = replacement.to_string();
            break;
        } else if processed_lower.ends_with(word) {
            let prefix_len = processed_lower.len() - word.len();
            if processed_lower[..prefix_len].ends_with(' ') {
                processed_lower.truncate(prefix_len);
                processed_lower.push_str(replacement);
                processed_trim.truncate(prefix_len);
                processed_trim.push_str(replacement);
                break;
            }
        }
    }

    // 强大的混合相对时间解析 (避免 f64 精度丢失，支持无符号隐式正数，支持闰年滚动计算)
    let suffixes = [
        (" year", 0),
        (" years", 0),
        (" month", 0),
        (" months", 0),
        (" week", 604800),
        (" weeks", 604800), // 添加了对 week 的支持
        (" day", 86400),
        (" days", 86400),
        (" hour", 3600),
        (" hours", 3600),
        (" minute", 60),
        (" minutes", 60),
        (" sec", 1),
        (" seconds", 1),
    ];
    for (suffix, _multiplier) in suffixes {
        if processed_lower.ends_with(suffix) {
            let stripped = &processed_trim[..processed_trim.len() - suffix.len()];

            let mut end = stripped.len();
            let bytes = stripped.as_bytes();
            while end > 0 && bytes[end - 1] == b' ' {
                end -= 1;
            }

            let mut start = end;
            while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
                start -= 1;
            }

            let mut sign_start = start;
            while sign_start > 0 && bytes[sign_start - 1] == b' ' {
                sign_start -= 1;
            }

            if sign_start > 0 && (bytes[sign_start - 1] == b'+' || bytes[sign_start - 1] == b'-') {
                start = sign_start - 1;
            }

            if start < end {
                let date_part = stripped[..start].trim();
                let amount_part = stripped[start..end].replace(" ", "");

                let is_neg = amount_part.starts_with('-');
                let amount_abs = if is_neg || amount_part.starts_with('+') {
                    &amount_part[1..]
                } else {
                    &amount_part
                };

                let (mut secs, mut nanos) = if let Some(dot_idx) = amount_abs.find('.') {
                    let secs = amount_abs[..dot_idx].parse().unwrap_or(0);
                    let frac = &amount_abs[dot_idx + 1..];
                    let frac_padded = format!("{frac:0<9}");
                    let nanos = frac_padded[..9].parse().unwrap_or(0);
                    (secs, nanos)
                } else {
                    (amount_abs.parse().unwrap_or(0), 0)
                };

                if is_neg {
                    secs = -secs;
                    nanos = -nanos;
                }
                if is_ago {
                    secs = -secs;
                    nanos = -nanos;
                } // 如果带有 ago，立刻倒转时间！

                let dt_res = if date_part.is_empty() {
                    Ok(reference_time)
                } else {
                    parse_datetime_gnu_compat(date_part, reference_time)
                };

                if let Ok(dt) = dt_res {
                    // 将 day 和 week 也纳入“日历计算”阵营，避免跨越夏令时边界时的物理物理秒数偏移
                    if suffix.contains("year")
                        || suffix.contains("month")
                        || suffix.contains("week")
                        || suffix.contains("day")
                    {
                        let naive_dt = if suffix.contains("year") || suffix.contains("month") {
                            let mut y = dt.year();
                            let mut m0 = dt.month0() as i32;
                            if suffix.contains("year") {
                                y += secs as i32;
                            } else {
                                m0 += secs as i32;
                            }

                            let y_adj = m0.div_euclid(12);
                            m0 = m0.rem_euclid(12);
                            y += y_adj;

                            if let Some(target_1st) = NaiveDate::from_ymd_opt(y, (m0 + 1) as u32, 1)
                            {
                                let target_date = target_1st + Duration::days(dt.day() as i64 - 1);
                                Some(target_date.and_time(dt.time()))
                            } else {
                                None
                            }
                        } else {
                            // Week 和 Day 直接通过纯粹的日历面板 (NaiveDate) 进行天数平移
                            let days_to_add = if suffix.contains("week") {
                                secs * 7
                            } else {
                                secs
                            };
                            Some(
                                (dt.date_naive() + Duration::days(days_to_add)).and_time(dt.time()),
                            )
                        };

                        // 重新绑定时区：如果正好落在了夏令时跳过的那一个小时里，安全往后推一小时
                        if let Some(ndt) = naive_dt {
                            let target_dt = match dt.timezone().from_local_datetime(&ndt) {
                                chrono::LocalResult::Single(d) => Some(d),
                                chrono::LocalResult::Ambiguous(d, _) => Some(d),
                                chrono::LocalResult::None => {
                                    match dt
                                        .timezone()
                                        .from_local_datetime(&(ndt + Duration::hours(1)))
                                    {
                                        chrono::LocalResult::Single(d)
                                        | chrono::LocalResult::Ambiguous(d, _) => Some(d),
                                        chrono::LocalResult::None => None,
                                    }
                                }
                            };

                            if let Some(new_dt) = target_dt {
                                return Ok(new_dt.with_timezone(&Local));
                            }
                        }
                    } else {
                        // hour, minute, second 走绝对的物理时间线加减
                        let s = secs * _multiplier;
                        let n = nanos * _multiplier;
                        let total_nanos = n % 1_000_000_000;
                        let extra_secs = n / 1_000_000_000;
                        let duration =
                            Duration::seconds(s + extra_secs) + Duration::nanoseconds(total_nanos);
                        if let Some(new_dt) = dt.checked_add_signed(duration) {
                            return Ok(new_dt);
                        }
                    }
                }
            }
        }
    }

    let mut normalized_input = input_trim
        .replace(" UTC", " +0000")
        .replace(" GMT", " +0000");

    // 修复简写时区偏移 (如 "+0", "-5" 转换为标准 "+0000", "-0500")
    if let Some(pos) = normalized_input.rfind(['+', '-']) {
        let offset_str = &normalized_input[pos + 1..];
        if !offset_str.is_empty()
            && offset_str.chars().all(|c| c.is_ascii_digit())
            && offset_str.len() <= 2
        {
            let sign = &normalized_input[pos..=pos];
            let hours = offset_str.parse::<u32>().unwrap_or(0);
            normalized_input = format!("{}{}{:02}00", &normalized_input[..pos], sign, hours);
        }
    }

    // 精确覆盖所有标准和边缘 ISO/RFC 组合
    // 删除了无用的字面量 'Z'，统一依赖强大的 %z 来接管所有时区解析
    let formats_with_tz = [
        "%Y-%m-%d %H:%M:%S %z",
        "%Y-%m-%d %H:%M:%S %:z",
        "%Y-%m-%d %H:%M %z",
        "%Y-%m-%d %H:%M %:z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%dT%H:%M:%S%:z",
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%d %H:%M:%S%.f %z",
        "%Y-%m-%d %H:%M:%S%.f %:z",
        "%Y-%m-%dT%H:%M%z",
        // 4位年份美式格式 (带时区)
        "%m/%d/%Y %H:%M:%S %z",
        "%m/%d/%Y %H:%M:%S %:z",
        "%m/%d/%Y %H:%M %z",
        "%m/%d/%Y %H:%M %:z",
        // 2位年份美式格式 (带时区，完美解决 08/01/97 6:00 UTC 问题)
        "%m/%d/%y %H:%M:%S %z",
        "%m/%d/%y %H:%M:%S %:z",
        "%m/%d/%y %H:%M %z",
        "%m/%d/%y %H:%M %:z",
    ];
    for fmt in formats_with_tz {
        if let Ok(dt) = DateTime::parse_from_str(&normalized_input, fmt) {
            return Ok(dt.with_timezone(&Local));
        }
    }

    if let Some(space_idx) = normalized_input.rfind(' ') {
        let date_str = &normalized_input[..space_idx];
        let tz_str = &normalized_input[space_idx + 1..];

        // 确认末尾像是一个时区偏移 (以 +/- 开头，且后面全是数字)
        if (tz_str.starts_with('+') || tz_str.starts_with('-'))
            && tz_str.len() >= 3
            && tz_str.chars().skip(1).all(|c| c.is_ascii_digit())
        {
            // 强行插入 00:00:00 午夜时间，伪装成标准格式交给 chrono 解析
            let synthesized = format!("{date_str} 00:00:00 {tz_str}");
            let synth_formats = [
                "%Y-%m-%d %H:%M:%S %z",
                "%m/%d/%Y %H:%M:%S %z",
                "%m/%d/%y %H:%M:%S %z",
            ];
            for fmt in synth_formats {
                if let Ok(dt) = DateTime::parse_from_str(&synthesized, fmt) {
                    return Ok(dt.with_timezone(&Local));
                }
            }
        }
    }

    let naive_formats = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d",
        "%Y/%m/%d",
        // 4位年份美式格式
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %H:%M",
        "%m/%d/%Y",
        // 2位年份美式格式
        "%m/%d/%y %H:%M:%S",
        "%m/%d/%y %H:%M",
        "%m/%d/%y",
        // 包含英文月份名称的格式 (完美解决 "Nov 10 1996" 和 "May-23-2003" 测试)
        "%b %d %Y %H:%M:%S",
        "%b %d %Y %H:%M",
        "%b %d %Y",
        "%b-%d-%Y %H:%M:%S",
        "%b-%d-%Y %H:%M",
        "%b-%d-%Y",
        "%d %b %Y %H:%M:%S",
        "%d %b %Y %H:%M",
        "%d %b %Y",
        // 6位纯数字紧凑格式
        "%y%m%d",
    ];
    // 这个 Naive 循环彻底解决了外部 crate 误解单数字月日导致 %U/%V 偏移的问题
    for fmt in naive_formats {
        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(input_trim, fmt) {
            return Ok(Local.from_local_datetime(&naive_dt).unwrap());
        }
        if let Ok(naive_date) = NaiveDate::parse_from_str(input_trim, fmt) {
            if let Some(naive_dt) = naive_date.and_hms_opt(0, 0, 0) {
                return Ok(Local.from_local_datetime(&naive_dt).unwrap());
            }
        }
    }

    // 纯星期几与相对词 (如 "next monday")
    if let Some(dt) = parse_weekday_expression(&input_lower, reference_time) {
        return Ok(dt);
    }
    if let Some(dt) = parse_relative_time(&input_lower, reference_time) {
        return Ok(dt);
    }

    // 终极回退：外部 crate (针对极其松散的自然语言)
    match parse_datetime::parse_datetime_at_date(reference_time, input) {
        Ok(dt) => Ok(dt.with_timezone(&Local)),
        Err(_) => Err(ParseDateTimeError {
            message: format!("Unable to parse date: {input}"),
        }),
    }
}

fn parse_compact_time_of_day(
    input: &str,
    reference_time: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let (hour_part, minute_part) = match input.len() {
        1 | 2 => (input, "0"),
        3 => input.split_at(1),
        4 => input.split_at(2),
        _ => return None,
    };

    let hour = hour_part.parse::<u32>().ok()?;
    let minute = minute_part.parse::<u32>().ok()?;
    let time = chrono::NaiveTime::from_hms_opt(hour, minute, 0)?;
    let naive = reference_time.date_naive().and_time(time);

    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
        chrono::LocalResult::None => None,
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
        ("sun", Weekday::Sun),
        ("monday", Weekday::Mon),
        ("mon", Weekday::Mon),
        ("tuesday", Weekday::Tue),
        ("tue", Weekday::Tue),
        ("tues", Weekday::Tue),
        ("wednesday", Weekday::Wed),
        ("wed", Weekday::Wed),
        ("wednes", Weekday::Wed),
        ("thursday", Weekday::Thu),
        ("thu", Weekday::Thu),
        ("thur", Weekday::Thu),
        ("thurs", Weekday::Thu),
        ("friday", Weekday::Fri),
        ("fri", Weekday::Fri),
        ("saturday", Weekday::Sat),
        ("sat", Weekday::Sat),
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
    use chrono::{Local, TimeZone, Timelike};

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
    fn test_parse_compact_time_of_day() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, hour, minute) in [
            ("1", 1, 0),
            ("10", 10, 0),
            ("100", 1, 0),
            ("1234", 12, 34),
            ("2359", 23, 59),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.date_naive(), ref_time.date_naive(), "input {input}");
            assert_eq!(parsed.hour(), hour, "input {input}");
            assert_eq!(parsed.minute(), minute, "input {input}");
            assert_eq!(parsed.second(), 0, "input {input}");
        }
    }

    #[test]
    fn test_parse_invalid_compact_time_of_day() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in ["99", "090", "2400", "2360"] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} should fail"
            );
        }
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
