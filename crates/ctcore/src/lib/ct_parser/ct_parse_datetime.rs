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
    DateTime, Datelike, Duration, FixedOffset, Local, LocalResult, NaiveDate, NaiveDateTime,
    NaiveTime, TimeZone, Utc, Weekday,
};
use chrono_tz::Tz;
#[cfg(target_os = "linux")]
use std::ffi::CStr;

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
    parse_datetime_gnu_compat_impl(input, reference_time, false)
}

/// Normalize GNU's named ordinal relative-time forms into numeric forms that
/// the relative-time parser handles below.
fn normalize_gnu_ordinal_relative_units(input: &str) -> Option<String> {
    let tokens = input
        .split_ascii_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut date_tokens = Vec::with_capacity(tokens.len());
    let mut relative_tokens = Vec::new();
    let mut changed = false;
    let mut index = 0;

    while index < tokens.len() {
        let Some(unit_token) = tokens.get(index + 1) else {
            date_tokens.push(tokens[index].clone());
            break;
        };

        let amount = match tokens[index].as_str() {
            "first" => 1,
            "third" => 3,
            "fourth" => 4,
            "fifth" => 5,
            "sixth" => 6,
            "seventh" => 7,
            "eighth" => 8,
            "ninth" => 9,
            "tenth" => 10,
            "eleventh" => 11,
            "twelfth" => 12,
            _ => {
                date_tokens.push(tokens[index].clone());
                index += 1;
                continue;
            }
        };

        let unit = unit_token.strip_suffix('s').unwrap_or(unit_token);
        let canonical_unit = match unit {
            "year" | "month" | "fortnight" | "week" | "day" | "hour" | "minute" => unit.to_owned(),
            "min" => "minute".to_owned(),
            "second" | "sec" => "sec".to_owned(),
            _ => {
                date_tokens.push(tokens[index].clone());
                index += 1;
                continue;
            }
        };

        relative_tokens.push(format!("+{amount}"));
        relative_tokens.push(canonical_unit);
        changed = true;
        index += 2;
    }

    changed.then(|| {
        date_tokens.extend(relative_tokens);
        date_tokens.join(" ")
    })
}

/// Parse a GNU relative-time number with an optional decimal second fraction.
fn parse_gnu_relative_number(input: &str, is_negative: bool) -> Option<(i64, i64)> {
    let Some(separator) = input.bytes().position(|byte| matches!(byte, b'.' | b',')) else {
        return Some((input.parse().ok()?, 0));
    };

    let integer = &input[..separator];
    let fraction = &input[separator + 1..];
    if integer.is_empty()
        || fraction.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let mut seconds = integer.parse::<i64>().ok()?;
    let mut nanoseconds = 0;
    let mut fractional_digits = 0;
    let mut negative_tail = false;

    for byte in fraction.bytes() {
        if fractional_digits < 9 {
            nanoseconds = nanoseconds * 10 + i64::from(byte - b'0');
            fractional_digits += 1;
        } else if is_negative && byte != b'0' {
            negative_tail = true;
        }
    }
    for _ in fractional_digits..9 {
        nanoseconds *= 10;
    }

    // GNU truncates positive fractions and rounds a negative excess tail
    // toward negative infinity before normalizing the timespec.
    if negative_tail {
        nanoseconds += 1;
        if nanoseconds == 1_000_000_000 {
            seconds = seconds.checked_add(1)?;
            nanoseconds = 0;
        }
    }

    Some((seconds, nanoseconds))
}

#[derive(Default)]
struct GnuCalendarRelativeOffset {
    years: i64,
    months: i64,
    days: i64,
    terms: usize,
}

/// Collect a trailing chain of GNU calendar relative items.
///
/// GNU's parser accumulates every relative year, month, and day in one
/// `relative_time` value before normalizing the resulting calendar date.
/// Applying terms recursively changes results around short months.
fn collect_gnu_calendar_relative_offsets(
    input: &str,
    is_ago: bool,
) -> Option<(String, GnuCalendarRelativeOffset)> {
    let tokens = input.split_ascii_whitespace().collect::<Vec<_>>();
    let mut offset = GnuCalendarRelativeOffset::default();
    let mut index = tokens.len();

    while index > 0 {
        let unit = tokens[index - 1].to_ascii_lowercase();
        let (mut amount, consumed) = if index >= 2 {
            match tokens[index - 2].parse::<i64>() {
                Ok(amount) => (amount, 2),
                Err(_) => (1, 1),
            }
        } else {
            (1, 1)
        };
        if is_ago && offset.terms == 0 {
            amount = amount.checked_neg()?;
        }

        match unit.as_str() {
            "year" | "years" => offset.years = offset.years.checked_add(amount)?,
            "month" | "months" => offset.months = offset.months.checked_add(amount)?,
            "fortnight" | "fortnights" => {
                offset.days = offset.days.checked_add(amount.checked_mul(14)?)?
            }
            "week" | "weeks" => offset.days = offset.days.checked_add(amount.checked_mul(7)?)?,
            "day" | "days" => offset.days = offset.days.checked_add(amount)?,
            _ => break,
        }

        offset.terms += 1;
        index -= consumed;
    }

    (offset.terms >= 1).then(|| (tokens[..index].join(" "), offset))
}

fn apply_gnu_calendar_relative_offsets(
    dt: DateTime<Local>,
    offset: &GnuCalendarRelativeOffset,
) -> Option<DateTime<Local>> {
    let month = i64::from(dt.month0()).checked_add(offset.months)?;
    let year = i64::from(dt.year())
        .checked_add(offset.years)?
        .checked_add(month.div_euclid(12))?;
    let year = i32::try_from(year).ok()?;
    let month = u32::try_from(month.rem_euclid(12)).ok()? + 1;
    let day_index = i64::from(dt.day())
        .checked_add(offset.days)?
        .checked_sub(1)?;
    let target_date = NaiveDate::from_ymd_opt(year, month, 1)?
        .checked_add_signed(Duration::try_days(day_index)?)?;
    let target = target_date.and_time(dt.time());

    match Local.from_local_datetime(&target) {
        LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => Some(dt),
        LocalResult::None => Local
            .from_local_datetime(&(target + Duration::hours(1)))
            .earliest(),
    }
}

fn parse_datetime_gnu_compat_impl(
    input: &str,
    reference_time: DateTime<Local>,
    normalized_extended_year: bool,
) -> Result<DateTime<Local>, ParseDateTimeError> {
    let input_without_comments = strip_gnu_parenthesized_comments(input);
    let input_trim = input_without_comments.trim();

    if let Some(normalized) = normalize_gnu_dotted_words(input_trim, reference_time) {
        return parse_datetime_gnu_compat_impl(
            &normalized,
            reference_time,
            normalized_extended_year,
        );
    }

    let input_lower = input_trim.to_lowercase();

    // GNU parse-datetime treats a standalone `--` as an empty date
    // specification, retaining the reference date and resetting the clock.
    if input_trim == "--" {
        let midnight = reference_time.date_naive().and_time(NaiveTime::MIN);
        return match Local.from_local_datetime(&midnight) {
            LocalResult::Single(datetime) | LocalResult::Ambiguous(datetime, _) => Ok(datetime),
            LocalResult::None => Err(ParseDateTimeError {
                message: format!("Unable to parse date: {input}"),
            }),
        };
    }

    if contains_leap_second(input_trim) {
        return Err(ParseDateTimeError {
            message: format!("Unable to parse date: {input}"),
        });
    }

    if gnu_rejects_space_separated_numeric_date(input_trim) {
        return Err(ParseDateTimeError {
            message: format!("Unable to parse date: {input}"),
        });
    }

    if let Some(normalized) = normalize_comma_fractional_seconds(input_trim) {
        return parse_datetime_gnu_compat_impl(
            &normalized,
            reference_time,
            normalized_extended_year,
        );
    }

    if !normalized_extended_year {
        if has_explicit_plus_extended_year(input_trim) {
            return Err(ParseDateTimeError {
                message: format!("Unable to parse date: {input}"),
            });
        }
        if let Some(normalized) = normalize_gnu_extended_year(input_trim) {
            return parse_datetime_gnu_compat_impl(&normalized, reference_time, true);
        }
    }

    if let Some(normalized) = normalize_gnu_two_digit_hyphen_year(input_trim) {
        return parse_datetime_gnu_compat_impl(
            &normalized,
            reference_time,
            normalized_extended_year,
        );
    }

    if let Some(normalized) = normalize_gnu_lowercase_iso_separator(input_trim) {
        return parse_datetime_gnu_compat_impl(
            &normalized,
            reference_time,
            normalized_extended_year,
        );
    }

    // GNU ignores a weekday when an explicit date is also present.
    if let Some(input_without_weekday) = strip_weekday_from_explicit_date(input_trim) {
        return parse_datetime_gnu_compat_impl(
            &input_without_weekday,
            reference_time,
            normalized_extended_year,
        );
    }

    // 负数/正数纪元秒 (Epoch: @-22, @31536000)
    if let Some(epoch_str) = input_trim.strip_prefix('@') {
        if let Some((s, ns)) = parse_epoch_decimal(epoch_str) {
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

    if is_gnu_date_only_with_numeric_timezone(input_trim) {
        return Err(ParseDateTimeError {
            message: format!("Unable to parse date: {input}"),
        });
    }

    if gnu_rejects_iso_date_followed_by_standalone_t_timezone(input_trim) {
        return Err(ParseDateTimeError {
            message: format!("Unable to parse date: {input}"),
        });
    }

    if gnu_has_multiple_timezones(input_trim) {
        return Err(ParseDateTimeError {
            message: format!("Unable to parse date: {input}"),
        });
    }

    if let Some(dt) = parse_gnu_time_only_utc_designator(input_trim, reference_time) {
        return Ok(dt);
    }

    if let Some(dt) =
        parse_gnu_numeric_timezone(input_trim, reference_time, normalized_extended_year)
    {
        return Ok(dt);
    }

    if let Some(dt) = parse_military_timezone_only(input_trim, reference_time) {
        return Ok(dt);
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
            if let Some(Some(offset_hours)) = military_timezone_offset_hours(tz_char) {
                let hour = (bytes[0] - b'0') * 10 + (bytes[1] - b'0');
                let min = (bytes[3] - b'0') * 10 + (bytes[4] - b'0');
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

    if let Some(local_result) =
        parse_gnu_local_timezone(input_trim, reference_time, normalized_extended_year)
    {
        return local_result.ok_or_else(|| ParseDateTimeError {
            message: format!("Unable to parse date: {input}"),
        });
    }

    if let Some(dt) =
        parse_gnu_military_timezone(input_trim, reference_time, normalized_extended_year)
    {
        return Ok(dt);
    }

    if let Some(dt) = parse_gnu_named_timezone(input_trim, reference_time, normalized_extended_year)
    {
        return Ok(dt);
    }

    if let Some(dt) =
        parse_gnu_meridian_datetime(input_trim, reference_time, normalized_extended_year)
    {
        return Ok(dt);
    }

    if let Some(dt) = parse_embedded_timezone(input_trim) {
        return Ok(dt);
    }

    if let Some(dt) = parse_rfc5322_datetime(input_trim) {
        return Ok(dt);
    }

    // GNU parse-datetime parses day shifts and times as independent grammar
    // items, so both "tomorrow 12:34" and "12:34 tomorrow" are valid.
    if let Some(dt) = parse_relative_day_with_explicit_time(input_trim, reference_time) {
        return Ok(dt);
    }

    if let Some(dt) = parse_gnu_explicit_time_before_iso_date(input_trim, reference_time) {
        return Ok(dt);
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
    } else if processed_lower.ends_with(" hence") {
        processed_lower.truncate(processed_lower.len() - 6);
        processed_lower = processed_lower.trim().to_string();
        processed_trim.truncate(processed_trim.len() - 6);
        processed_trim = processed_trim.trim().to_string();
    }

    if let Some(normalized) = normalize_gnu_ordinal_relative_units(&processed_lower) {
        processed_lower = normalized.clone();
        processed_trim = normalized;
    }

    // 预处理自然语言相对时间词汇 (now, yesterday 等标准化为精准的加减法)
    let word_replacements = [
        ("yesterday", "-1 day"),
        ("tomorrow", "+1 day"),
        ("today", "+0 day"),
        ("now", "+0 sec"),
        ("this year", "+0 year"),
        ("this years", "+0 year"),
        ("this month", "+0 month"),
        ("this months", "+0 month"),
        ("this fortnight", "+0 fortnight"),
        ("this fortnights", "+0 fortnight"),
        ("this week", "+0 week"),
        ("this weeks", "+0 week"),
        ("this day", "+0 day"),
        ("this days", "+0 day"),
        ("this hour", "+0 hour"),
        ("this hours", "+0 hour"),
        ("this minute", "+0 minute"),
        ("this minutes", "+0 minute"),
        ("this min", "+0 minute"),
        ("this mins", "+0 minute"),
        ("this second", "+0 sec"),
        ("this seconds", "+0 sec"),
        ("this sec", "+0 sec"),
        ("this secs", "+0 sec"),
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

    if let Some((_, replacement)) = [
        ("year", "+1 year"),
        ("month", "+1 month"),
        ("fortnight", "+1 fortnight"),
        ("week", "+1 week"),
        ("day", "+1 day"),
        ("hour", "+1 hour"),
        ("minute", "+1 minute"),
        ("min", "+1 minute"),
        ("second", "+1 sec"),
        ("sec", "+1 sec"),
    ]
    .into_iter()
    .find(|(word, _)| processed_lower == *word)
    {
        processed_lower = replacement.to_string();
        processed_trim = replacement.to_string();
    }

    if let Some((date_part, offset)) =
        collect_gnu_calendar_relative_offsets(&processed_trim, is_ago)
    {
        let base = if date_part.is_empty() {
            Ok(reference_time)
        } else {
            parse_datetime_gnu_compat_impl(&date_part, reference_time, normalized_extended_year)
        };
        if let Ok(base) = base {
            if let Some(adjusted) = apply_gnu_calendar_relative_offsets(base, &offset) {
                return Ok(adjusted);
            }
        }
    }

    // 强大的混合相对时间解析 (避免 f64 精度丢失，支持无符号隐式正数，支持闰年滚动计算)
    let suffixes = [
        (" year", 0),
        (" years", 0),
        (" month", 0),
        (" months", 0),
        (" fortnight", 0),
        (" fortnights", 0),
        (" week", 604800),
        (" weeks", 604800), // 添加了对 week 的支持
        (" day", 86400),
        (" days", 86400),
        (" hour", 3600),
        (" hours", 3600),
        (" minute", 60),
        (" minutes", 60),
        (" min", 60),
        (" mins", 60),
        (" second", 1),
        (" seconds", 1),
        (" sec", 1),
        (" secs", 1),
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
            while start > 0
                && (bytes[start - 1].is_ascii_digit() || matches!(bytes[start - 1], b'.' | b','))
            {
                start -= 1;
            }

            let mut sign_start = start;
            while sign_start > 0 && bytes[sign_start - 1].is_ascii_whitespace() {
                sign_start -= 1;
            }

            if sign_start > 0 && (bytes[sign_start - 1] == b'+' || bytes[sign_start - 1] == b'-') {
                start = sign_start - 1;
            }

            if start < end {
                let date_part = stripped[..start].trim();
                let amount_part = stripped[start..end]
                    .chars()
                    .filter(|character| !character.is_ascii_whitespace())
                    .collect::<String>();

                let is_neg = amount_part.starts_with('-');
                let amount_abs = if is_neg || amount_part.starts_with('+') {
                    &amount_part[1..]
                } else {
                    &amount_part
                };
                if amount_abs.contains(['.', ','])
                    && !matches!(suffix, " second" | " seconds" | " sec" | " secs")
                {
                    return Err(ParseDateTimeError {
                        message: format!("Unable to parse date: {input}"),
                    });
                }

                let (mut secs, mut nanos) = parse_gnu_relative_number(amount_abs, is_neg)
                    .ok_or_else(|| ParseDateTimeError {
                        message: format!("Unable to parse date: {input}"),
                    })?;

                if is_neg {
                    secs = secs.checked_neg().ok_or_else(|| ParseDateTimeError {
                        message: format!("Unable to parse date: {input}"),
                    })?;
                    nanos = -nanos;
                }
                if is_ago {
                    secs = -secs;
                    nanos = -nanos;
                } // 如果带有 ago，立刻倒转时间！

                let dt_res = if date_part.is_empty() {
                    Ok(reference_time)
                } else {
                    parse_datetime_gnu_compat_impl(
                        date_part,
                        reference_time,
                        normalized_extended_year,
                    )
                };

                if let Ok(dt) = dt_res {
                    // 将 day 和 week 也纳入“日历计算”阵营，避免跨越夏令时边界时的物理物理秒数偏移
                    if suffix.contains("year")
                        || suffix.contains("month")
                        || suffix.contains("fortnight")
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
                            let days_to_add = if suffix.contains("fortnight") {
                                secs * 14
                            } else if suffix.contains("week") {
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
    let has_iso_utc_designator = normalized_input
        .as_bytes()
        .last()
        .is_some_and(|byte| matches!(byte, b'Z' | b'z'))
        && normalized_input
            .as_bytes()
            .get(normalized_input.len().saturating_sub(2))
            .is_some_and(u8::is_ascii_digit);
    if has_iso_utc_designator {
        normalized_input.pop();
        normalized_input.push_str("+0000");
    }

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

    // 精确覆盖所有标准和边缘 ISO/RFC 组合。
    let formats_with_tz = [
        "%Y-%m-%d %H:%M:%S %z",
        "%Y-%m-%d %H:%M:%S %:z",
        "%Y-%m-%d %H:%M %z",
        "%Y-%m-%d %H:%M %:z",
        "%Y-%m-%d %H%z",
        "%Y-%m-%dT%H%z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%dT%H:%M:%S%:z",
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%d %H:%M:%S%.f %z",
        "%Y-%m-%d %H:%M:%S%.f %:z",
        "%Y-%m-%dT%H:%M%z",
        // 两位年份必须先于%Y尝试，因为chrono的%Y也接受短年份。
        "%m/%d/%y %H:%M:%S %z",
        "%m/%d/%y %H:%M:%S %:z",
        "%m/%d/%y %H:%M %z",
        "%m/%d/%y %H:%M %:z",
        "%m/%d/%Y %H:%M:%S %z",
        "%m/%d/%Y %H:%M:%S %:z",
        "%m/%d/%Y %H:%M %z",
        "%m/%d/%Y %H:%M %:z",
        "%b %d, %Y %H:%M:%S %z",
        "%b %d, %Y %H:%M:%S %:z",
        "%b %d, %Y %H:%M %z",
        "%b %d, %Y %H:%M %:z",
        "%B %d, %Y %H:%M:%S %z",
        "%B %d, %Y %H:%M %z",
    ];
    let slash_date_has_two_digit_year = gnu_slash_date_has_two_digit_year(input_trim);
    for fmt in formats_with_tz {
        if fmt.contains("%m/%d/%y") && !slash_date_has_two_digit_year {
            continue;
        }
        if let Ok(dt) = DateTime::parse_from_str(&normalized_input, fmt) {
            if let Some(dt) = expand_year_for_format(dt, fmt) {
                return Ok(dt.with_timezone(&Local));
            }
        }
    }

    if let Some(dt) = parse_gnu_utc_date(&normalized_input) {
        return Ok(dt);
    }

    let naive_formats = [
        // Chrono's ISO parser accepts slash separators too, so GNU slash
        // dates must be attempted before the general year-first formats.
        // Two-digit years must precede %Y because chrono accepts short years.
        "%m/%d/%y %H:%M:%S",
        "%m/%d/%y %H:%M",
        "%m/%d/%y",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %H:%M",
        "%m/%d/%Y",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d",
        "%Y/%m/%d %H:%M:%S%.f",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%Y/%m/%d",
        "%Y%m%d %H:%M:%S%.f",
        "%Y%m%d %H:%M:%S",
        "%Y%m%d %H:%M",
        "%Y%m%d",
        // 包含英文月份名称的格式 (完美解决 "Nov 10 1996" 和 "May-23-2003" 测试)
        "%b %d %Y %H:%M:%S",
        "%b %d %Y %H:%M",
        "%b %d %H:%M:%S %Y",
        "%b %d %H:%M %Y",
        "%B %d %Y %H:%M:%S",
        "%B %d %Y %H:%M",
        "%B %d %H:%M:%S %Y",
        "%B %d %H:%M %Y",
        "%H:%M:%S %b %d %Y",
        "%H:%M %b %d %Y",
        "%H:%M:%S %B %d %Y",
        "%H:%M %B %d %Y",
        "%b %d %Y",
        "%B %d %Y",
        "%b-%d-%Y %H:%M:%S",
        "%b-%d-%Y %H:%M",
        "%b-%d-%Y",
        "%B-%d-%Y %H:%M:%S",
        "%B-%d-%Y %H:%M",
        "%B-%d-%Y",
        "%d-%b-%Y %H:%M:%S",
        "%d-%b-%Y %H:%M",
        "%d-%b-%Y",
        "%d-%B-%Y %H:%M:%S",
        "%d-%B-%Y %H:%M",
        "%d-%B-%Y",
        "%d %b %Y %H:%M:%S",
        "%d %b %Y %H:%M",
        "%d %b %Y",
        "%d %B %Y %H:%M:%S",
        "%d %B %Y %H:%M",
        "%d %B %Y",
        "%b %d, %Y %H:%M:%S",
        "%b %d, %Y %H:%M",
        "%b %d, %Y",
        "%B %d, %Y %H:%M:%S",
        "%B %d, %Y %H:%M",
        "%B %d, %Y",
        // 6位纯数字紧凑格式
        "%y%m%d",
    ];
    // GNU only interprets a three-field slash date as year-first when the
    // first field has at least four digits; otherwise it is MM/DD/YY[YY].
    let slash_year_first = input_trim.split_once('/').is_some_and(|(year, remainder)| {
        remainder.contains('/') && year.len() >= 4 && year.as_bytes().iter().all(u8::is_ascii_digit)
    });

    // 这个 Naive 循环彻底解决了外部 crate 误解单数字月日导致 %U/%V 偏移的问题
    for fmt in naive_formats {
        if fmt.starts_with("%Y/") && !slash_year_first {
            continue;
        }
        if fmt.starts_with("%m/%d/%y") && !slash_date_has_two_digit_year {
            continue;
        }
        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(input_trim, fmt) {
            if let Some(naive_dt) = expand_year_for_format(naive_dt, fmt) {
                return Ok(Local.from_local_datetime(&naive_dt).unwrap());
            }
        }
        if let Ok(naive_date) = NaiveDate::parse_from_str(input_trim, fmt) {
            if let Some(naive_date) = expand_year_for_format(naive_date, fmt) {
                if let Some(naive_dt) = naive_date.and_hms_opt(0, 0, 0) {
                    return Ok(Local.from_local_datetime(&naive_dt).unwrap());
                }
            }
        }
    }

    if let Some(dt) =
        parse_gnu_compact_time_with_date(input_trim, reference_time, normalized_extended_year)
    {
        return Ok(dt);
    }

    if let Some(dt) =
        parse_gnu_relative_unit_with_weekday(input_trim, reference_time, normalized_extended_year)
    {
        return Ok(dt);
    }

    if let Some(dt) = parse_gnu_date_without_year(input_trim, reference_time) {
        return Ok(dt);
    }

    if let Some(dt) = parse_gnu_time_only(input_trim, reference_time) {
        return Ok(dt);
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

/// Return whether a slash date has the exact two-digit year form that GNU
/// maps with its XPG4 century rule.  One-digit and three-or-more-digit years
/// are literal years, even though chrono's `%y` parser accepts them too.
fn gnu_slash_date_has_two_digit_year(input: &str) -> bool {
    let date = input.split_ascii_whitespace().next().unwrap_or_default();
    let Some((_, month_and_year)) = date.split_once('/') else {
        return false;
    };
    let Some((_, year)) = month_and_year.split_once('/') else {
        return false;
    };

    year.len() == 2 && year.bytes().all(|byte| byte.is_ascii_digit())
}

/// GNU's grammar requires date fields to use an ISO, slash, or word-month
/// separator. Chrono accepts whitespace before adjacent numeric directives,
/// causing our compact `%Y%m%d` format to accept this non-GNU form.
fn gnu_rejects_space_separated_numeric_date(input: &str) -> bool {
    let mut fields = input.split_ascii_whitespace();
    let (Some(first), Some(second), Some(third)) = (fields.next(), fields.next(), fields.next())
    else {
        return false;
    };

    [first, second, third]
        .iter()
        .all(|field| field.bytes().all(|byte| byte.is_ascii_digit()))
}

/// GNU's lexer treats a lowercase `t` immediately after an ISO date as the
/// ISO 8601 date-time separator. Restrict normalization to that position so
/// an unrelated `t` token keeps its GNU grammar meaning.
fn normalize_gnu_lowercase_iso_separator(input: &str) -> Option<String> {
    let separator = input.bytes().position(|byte| byte == b't')?;
    let date = &input[..separator];
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;

    Some(format!("{date}T{}", &input[separator + 1..]))
}

/// Normalize the two dotted-word forms accepted by GNU `parse-datetime`.
///
/// GNU's lexer treats a trailing period as part of a three-letter month or
/// weekday abbreviation.  For timezone words, it retries lookup after
/// removing periods, allowing inputs such as `P.S.T.`.  It does not remove
/// periods from arbitrary words, so retain every other token unchanged.
fn normalize_gnu_dotted_words(input: &str, reference_time: DateTime<Local>) -> Option<String> {
    const DATE_ABBREVIATIONS: &[&str] = &[
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec", "sun",
        "mon", "tue", "wed", "thu", "fri", "sat",
    ];

    let bytes = input.as_bytes();
    let mut normalized = String::with_capacity(input.len());
    let mut copied_until = 0;
    let mut index = 0;
    let mut changed = false;

    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic() {
            index += 1;
            continue;
        }

        let start = index;
        index += 1;
        while index < bytes.len() && (bytes[index].is_ascii_alphabetic() || bytes[index] == b'.') {
            index += 1;
        }
        let word = &input[start..index];

        if word.len() == 4
            && word.ends_with('.')
            && DATE_ABBREVIATIONS
                .iter()
                .any(|abbreviation| word[..3].eq_ignore_ascii_case(abbreviation))
        {
            normalized.push_str(&input[copied_until..start]);
            normalized.push_str(&word[..3]);
            copied_until = index;
            changed = true;
        } else if word.contains('.') {
            let without_periods = word.replace('.', "");
            if is_gnu_known_timezone(&without_periods, reference_time) {
                normalized.push_str(&input[copied_until..start]);
                normalized.push_str(&without_periods);
                copied_until = index;
                changed = true;
            }
        }
    }

    changed.then(|| {
        normalized.push_str(&input[copied_until..]);
        normalized
    })
}

fn is_gnu_known_timezone(input: &str, reference_time: DateTime<Local>) -> bool {
    GNU_NAMED_TIMEZONES
        .iter()
        .any(|(name, _)| input.eq_ignore_ascii_case(name))
        || local_timezone_info(reference_time.timestamp())
            .is_some_and(|(name, _)| input.eq_ignore_ascii_case(&name))
}

/// GNU date syntax ignores parenthesized comments.  Nested and unterminated
/// comments are accepted; an unmatched closing parenthesis remains input.
fn strip_gnu_parenthesized_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut depth = 0usize;

    for character in input.chars() {
        match character {
            '(' if depth == 0 => {
                if !result.ends_with(char::is_whitespace) {
                    result.push(' ');
                }
                depth = 1;
            }
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => result.push(character),
            _ => {}
        }
    }

    result
}

fn expand_year_for_format<T: Datelike>(value: T, format: &str) -> Option<T> {
    if !format.contains("%y") {
        return Some(value);
    }

    let two_digit_year = value.year().rem_euclid(100);
    let expanded = if two_digit_year < 69 {
        two_digit_year + 2000
    } else {
        two_digit_year + 1900
    };
    value.with_year(expanded)
}

/// Expand GNU's two-digit year mapping for a hyphenated ISO date item.
///
/// `chrono` accepts a short year for `%Y` literally.  GNU's parser instead
/// applies its XPG4 two-digit year mapping before validating the date.
fn normalize_gnu_two_digit_hyphen_year(input: &str) -> Option<String> {
    let bytes = input.as_bytes();

    for start in 0..bytes.len().saturating_sub(4) {
        if !bytes[start].is_ascii_digit()
            || !bytes[start + 1].is_ascii_digit()
            || bytes[start + 2] != b'-'
            || start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_')
        {
            continue;
        }

        let mut month_end = start + 3;
        while month_end < bytes.len()
            && bytes[month_end].is_ascii_digit()
            && month_end - (start + 3) < 2
        {
            month_end += 1;
        }
        if month_end == start + 3 || bytes.get(month_end) != Some(&b'-') {
            continue;
        }

        let mut day_end = month_end + 1;
        while day_end < bytes.len()
            && bytes[day_end].is_ascii_digit()
            && day_end - (month_end + 1) < 2
        {
            day_end += 1;
        }
        let invalid_trailer = bytes.get(day_end).is_some_and(|byte| {
            (byte.is_ascii_alphanumeric() && !matches!(*byte, b'T' | b't')) || *byte == b'_'
        });
        if day_end == month_end + 1 || invalid_trailer {
            continue;
        }

        let year = input[start..start + 2].parse::<u32>().ok()?;
        let expanded = if year < 69 { year + 2000 } else { year + 1900 };
        return Some(format!(
            "{}{}{}",
            &input[..start],
            expanded,
            &input[start + 2..]
        ));
    }

    None
}

/// Parse GNU month/day forms whose omitted year defaults to the reference year.
fn parse_gnu_date_without_year(
    input: &str,
    reference_time: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let parts = input.split_ascii_whitespace().collect::<Vec<_>>();
    let (month, day) = match parts.as_slice() {
        [slash_date] => {
            let (month, day) = slash_date.split_once('/')?;
            (month.parse().ok()?, day.parse().ok()?)
        }
        [first, second] => {
            if let Some(month) = gnu_month_number(first) {
                (month, second.parse().ok()?)
            } else {
                (gnu_month_number(second)?, first.parse().ok()?)
            }
        }
        _ => return None,
    };

    let date = NaiveDate::from_ymd_opt(reference_time.year(), month, day)?;
    match reference_time
        .timezone()
        .from_local_datetime(&date.and_time(NaiveTime::MIN))
    {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
        chrono::LocalResult::None => None,
    }
}

fn parse_gnu_iso_hour(input: &str) -> Option<NaiveDateTime> {
    let (date, hour) = input.split_once(['T', 't'])?;
    if !(1..=2).contains(&hour.len()) || !hour.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(hour.parse().ok()?, 0, 0)
}

/// GNU parses date and time items independently, including a clock item that
/// precedes an ISO date such as `12:34 2024-02-29`.
fn parse_gnu_explicit_time_before_iso_date(
    input: &str,
    reference_time: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let tokens = input.split_ascii_whitespace().collect::<Vec<_>>();
    let [time_input, date_input] = tokens.as_slice() else {
        return None;
    };
    let time = ["%H:%M:%S%.f", "%H:%M:%S", "%H:%M"]
        .into_iter()
        .find_map(|format| NaiveTime::parse_from_str(time_input, format).ok())?;
    let date = NaiveDate::parse_from_str(date_input, "%Y-%m-%d").ok()?;

    match reference_time
        .timezone()
        .from_local_datetime(&date.and_time(time))
    {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
        chrono::LocalResult::None => None,
    }
}

/// Parse a GNU 24-hour clock item with the date inherited from the reference.
fn parse_gnu_time_only(input: &str, reference_time: DateTime<Local>) -> Option<DateTime<Local>> {
    let time = parse_gnu_24_hour_clock(input)?;
    let naive = reference_time.date_naive().and_time(time);

    match reference_time.timezone().from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
        chrono::LocalResult::None => None,
    }
}

fn parse_gnu_24_hour_clock(input: &str) -> Option<NaiveTime> {
    let mut components = input.split(':');
    let hour = components.next()?.parse::<u32>().ok()?;
    let minute = components.next()?.parse::<u32>().ok()?;
    let seconds = components.next().unwrap_or("0");
    if components.next().is_some() || minute > 59 {
        return None;
    }

    let (second, nanoseconds) = if let Some((second, fraction)) = seconds.split_once('.') {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let mut nanoseconds = fraction
            .bytes()
            .take(9)
            .fold(0_u32, |value, digit| value * 10 + u32::from(digit - b'0'));
        for _ in fraction.len().min(9)..9 {
            nanoseconds *= 10;
        }
        (second.parse::<u32>().ok()?, nanoseconds)
    } else {
        (seconds.parse::<u32>().ok()?, 0)
    };

    NaiveTime::from_hms_nano_opt(hour, minute, second, nanoseconds)
}

fn parse_gnu_compact_time_with_date(
    input: &str,
    reference_time: DateTime<Local>,
    normalized_extended_year: bool,
) -> Option<DateTime<Local>> {
    let tokens = input.split_ascii_whitespace().collect::<Vec<_>>();
    let [first, second] = tokens.as_slice() else {
        return None;
    };

    for (time_input, date_input) in [(*first, *second), (*second, *first)] {
        if parse_weekday_name(date_input).is_some() {
            continue;
        }
        if is_gnu_relative_unit_expression(&[date_input]) {
            continue;
        }
        let Some(time) = parse_compact_time_of_day(time_input, reference_time).map(|dt| dt.time())
        else {
            continue;
        };
        let Ok(date) =
            parse_datetime_gnu_compat_impl(date_input, reference_time, normalized_extended_year)
        else {
            continue;
        };
        let naive = date.date_naive().and_time(time);
        match date.timezone().from_local_datetime(&naive) {
            chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
                return Some(dt);
            }
            chrono::LocalResult::None => {}
        }
    }

    None
}

fn parse_gnu_relative_unit_with_weekday(
    input: &str,
    reference_time: DateTime<Local>,
    normalized_extended_year: bool,
) -> Option<DateTime<Local>> {
    let tokens = input.split_ascii_whitespace().collect::<Vec<_>>();
    let weekday_indexes = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| parse_weekday_name(token).map(|_| index))
        .collect::<Vec<_>>();
    let [weekday_index] = weekday_indexes.as_slice() else {
        return None;
    };

    let relative_tokens = tokens
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != *weekday_index)
        .map(|(_, token)| *token)
        .collect::<Vec<_>>();
    if !is_gnu_relative_unit_expression(&relative_tokens) {
        return None;
    }

    let weekday = parse_weekday_expression(tokens[*weekday_index], reference_time)?;
    parse_datetime_gnu_compat_impl(
        &relative_tokens.join(" "),
        weekday,
        normalized_extended_year,
    )
    .ok()
}

fn is_gnu_relative_unit_expression(tokens: &[&str]) -> bool {
    match tokens {
        [unit] => is_gnu_relative_unit(unit),
        [amount, unit]
            if amount.parse::<f64>().is_ok()
                || matches!(amount.to_ascii_lowercase().as_str(), "next" | "last") =>
        {
            is_gnu_relative_unit(unit)
        }
        _ => false,
    }
}

fn is_gnu_relative_unit(input: &str) -> bool {
    matches!(
        input.to_ascii_lowercase().as_str(),
        "year"
            | "years"
            | "month"
            | "months"
            | "fortnight"
            | "fortnights"
            | "week"
            | "weeks"
            | "day"
            | "days"
            | "hour"
            | "hours"
            | "minute"
            | "minutes"
            | "min"
            | "second"
            | "seconds"
            | "sec"
    )
}

fn gnu_month_number(input: &str) -> Option<u32> {
    match input.to_ascii_lowercase().as_str() {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "sept" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

fn parse_epoch_decimal(input: &str) -> Option<(i64, u32)> {
    let (negative, unsigned) = match input.as_bytes().first() {
        Some(b'-') => (true, &input[1..]),
        Some(b'+') => (false, &input[1..]),
        _ => (false, input),
    };

    let separator = unsigned
        .bytes()
        .position(|byte| byte == b'.' || byte == b',');
    let (integer, fraction) = match separator {
        Some(index) => (&unsigned[..index], Some(&unsigned[index + 1..])),
        None => (unsigned, None),
    };
    if integer.is_empty() || !integer.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    let magnitude = integer.parse::<i128>().ok()?;
    let signed = if negative { -magnitude } else { magnitude };
    let mut seconds = i64::try_from(signed).ok()?;
    let mut nanoseconds = 0_u32;

    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }

        for index in 0..9 {
            nanoseconds *= 10;
            if let Some(digit) = fraction.as_bytes().get(index) {
                nanoseconds += u32::from(*digit - b'0');
            }
        }

        if negative
            && fraction
                .as_bytes()
                .get(9..)
                .is_some_and(|rest| rest.iter().any(|digit| *digit != b'0'))
        {
            nanoseconds += 1;
        }

        if negative && nanoseconds != 0 {
            seconds = seconds.checked_sub(1)?;
            nanoseconds = 1_000_000_000 - nanoseconds;
        }
    }

    Some((seconds, nanoseconds))
}

fn military_timezone_offset_hours(tz_char: char) -> Option<Option<i32>> {
    match tz_char.to_ascii_uppercase() {
        'A'..='I' => Some(Some(
            (tz_char.to_ascii_uppercase() as i32) - ('A' as i32) + 1,
        )),
        'J' => Some(None),
        'K'..='M' => Some(Some(
            (tz_char.to_ascii_uppercase() as i32) - ('K' as i32) + 10,
        )),
        'N'..='Y' => Some(Some(
            -((tz_char.to_ascii_uppercase() as i32) - ('N' as i32) + 1),
        )),
        'Z' => Some(Some(0)),
        _ => None,
    }
}

/// Return whether a date string contains a leap-second field rejected by GNU
/// `parse-datetime`.
pub fn contains_leap_second(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.windows(3).enumerate().any(|(seconds_colon, window)| {
        if window != b":60" || bytes.get(seconds_colon + 3).is_some_and(u8::is_ascii_digit) {
            return false;
        }

        let minute_start = bytes[..seconds_colon]
            .iter()
            .rposition(|byte| !byte.is_ascii_digit())
            .map_or(0, |index| index + 1);
        let minute_digits = &bytes[minute_start..seconds_colon];
        if !(1..=2).contains(&minute_digits.len())
            || minute_start == 0
            || bytes[minute_start - 1] != b':'
        {
            return false;
        }
        let minute = minute_digits
            .iter()
            .fold(0_u8, |value, digit| value * 10 + (digit - b'0'));
        if minute > 59 {
            return false;
        }

        let hour_end = minute_start - 1;
        let hour_start = bytes[..hour_end]
            .iter()
            .rposition(|byte| !byte.is_ascii_digit())
            .map_or(0, |index| index + 1);
        let hour_digits = &bytes[hour_start..hour_end];
        (1..=2).contains(&hour_digits.len())
            && hour_start
                .checked_sub(1)
                .and_then(|index| bytes.get(index))
                .is_none_or(|byte| !byte.is_ascii_digit())
    })
}

fn normalize_comma_fractional_seconds(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut normalized = bytes.to_vec();
    let mut changed = false;

    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b',' && is_fractional_seconds_comma(bytes, index) {
            normalized[index] = b'.';
            changed = true;
        }
    }

    changed.then(|| String::from_utf8(normalized).expect("ASCII replacement preserves UTF-8"))
}

fn is_fractional_seconds_comma(bytes: &[u8], comma: usize) -> bool {
    if !bytes.get(comma + 1).is_some_and(u8::is_ascii_digit) {
        return false;
    }

    let mut seconds_start = comma;
    while seconds_start > 0 && bytes[seconds_start - 1].is_ascii_digit() {
        seconds_start -= 1;
    }
    if seconds_start == comma || seconds_start == 0 || bytes[seconds_start - 1] != b':' {
        return false;
    }

    let minute_end = seconds_start - 1;
    let mut minute_start = minute_end;
    while minute_start > 0 && bytes[minute_start - 1].is_ascii_digit() {
        minute_start -= 1;
    }
    if minute_start == minute_end || minute_start == 0 || bytes[minute_start - 1] != b':' {
        return false;
    }

    let hour_end = minute_start - 1;
    hour_end > 0 && bytes[hour_end - 1].is_ascii_digit()
}

fn has_explicit_plus_extended_year(input: &str) -> bool {
    let Some(rest) = input.strip_prefix('+') else {
        return false;
    };
    let year_digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    year_digits >= 5 && matches!(rest.as_bytes().get(year_digits), Some(b'-' | b'/'))
}

fn normalize_gnu_extended_year(input: &str) -> Option<String> {
    let year_digits = input.bytes().take_while(u8::is_ascii_digit).count();
    if year_digits < 5 || !matches!(input.as_bytes().get(year_digits), Some(b'-' | b'/')) {
        return None;
    }

    let normalized = if input.as_bytes()[year_digits] == b'/' {
        input.replacen('/', "-", 2)
    } else {
        input.to_string()
    };
    Some(format!("+{normalized}"))
}

fn parse_gnu_numeric_timezone(
    input: &str,
    reference_time: DateTime<Local>,
    normalized_extended_year: bool,
) -> Option<DateTime<Local>> {
    let sign_index = input.rfind(['+', '-'])?;
    let zone = &input[sign_index + 1..];

    let wall_time_input = input[..sign_index].trim_end();
    let last_word = wall_time_input.split_ascii_whitespace().next_back()?;
    let last_time_token = last_word
        .rsplit_once(['T', 't'])
        .map_or(last_word, |(_, token)| token);
    let has_explicit_time = wall_time_input.contains(':')
        || ((1..=2).contains(&last_time_token.len())
            && last_time_token.bytes().all(|byte| byte.is_ascii_digit()));
    if !has_explicit_time {
        return None;
    }

    let negative = input.as_bytes()[sign_index] == b'-';
    let offset_minutes = if let Some((hours, minutes)) = zone.split_once(':') {
        let hours = hours.parse::<i64>().ok()?;
        let minutes = minutes.parse::<i64>().ok()?;
        let hours_in_minutes = hours.checked_mul(60)?;
        if negative {
            hours_in_minutes.checked_neg()?.checked_sub(minutes)?
        } else {
            hours_in_minutes.checked_add(minutes)?
        }
    } else {
        let value = zone.parse::<i64>().ok()?;
        let offset = if zone.len() <= 2 {
            value.checked_mul(60)?
        } else {
            (value / 100).checked_mul(60)?.checked_add(value % 100)?
        };
        if negative { -offset } else { offset }
    };
    if !(-24 * 60..=24 * 60).contains(&offset_minutes) {
        return None;
    }

    let wall_time = if let Some(naive) = parse_gnu_iso_hour(wall_time_input) {
        match Local.from_local_datetime(&naive) {
            chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => dt,
            chrono::LocalResult::None => return None,
        }
    } else {
        parse_datetime_gnu_compat_impl(wall_time_input, reference_time, normalized_extended_year)
            .ok()?
    };
    let utc_naive = wall_time
        .naive_local()
        .checked_sub_signed(Duration::minutes(offset_minutes))?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(utc_naive, Utc).with_timezone(&Local))
}

/// GNU accepts a numeric UTC offset only as part of an explicit time item.
/// Keep date-only forms out of generic parser fallbacks that synthesize midnight.
fn is_gnu_date_only_with_numeric_timezone(input: &str) -> bool {
    let Some(sign_index) = input.rfind(['+', '-']) else {
        return false;
    };
    let offset = &input[sign_index + 1..];
    if offset.is_empty() || !offset.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }

    let date = input[..sign_index].trim_end();
    [
        "%Y-%m-%d", "%Y/%m/%d", "%m/%d/%y", "%m/%d/%Y", "%Y%m%d", "%y%m%d",
    ]
    .into_iter()
    .any(|format| NaiveDate::parse_from_str(date, format).is_ok())
}

/// GNU's `T` token is both the UTC-7 military timezone and the ISO 8601
/// date-time separator. After a hyphenated numeric date, the ISO production
/// takes precedence and therefore requires a following time item.
fn gnu_rejects_iso_date_followed_by_standalone_t_timezone(input: &str) -> bool {
    let mut fields = input.split_ascii_whitespace();
    let Some(date) = fields.next() else {
        return false;
    };
    let Some(zone) = fields.next() else {
        return false;
    };
    if fields.next().is_some() || !zone.eq_ignore_ascii_case("t") {
        return false;
    }

    gnu_hyphenated_numeric_date(date)
}

fn gnu_hyphenated_numeric_date(input: &str) -> bool {
    let mut date_fields = input.split('-');
    let Some(year) = date_fields.next() else {
        return false;
    };
    let Some(month) = date_fields.next() else {
        return false;
    };
    let Some(day) = date_fields.next() else {
        return false;
    };

    date_fields.next().is_none()
        && [year, month, day]
            .into_iter()
            .all(|field| !field.is_empty() && field.bytes().all(|byte| byte.is_ascii_digit()))
}

fn parse_military_timezone_only(
    input: &str,
    reference_time: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let mut chars = input.chars();
    let tz_char = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    let offset_hours = military_timezone_offset_hours(tz_char)?;
    let naive_midnight = reference_time.date_naive().and_hms_opt(0, 0, 0)?;

    match offset_hours {
        Some(hours) => {
            let offset = FixedOffset::east_opt(hours * 3600)?;
            match offset.from_local_datetime(&naive_midnight) {
                chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
                    Some(dt.with_timezone(&Local))
                }
                chrono::LocalResult::None => None,
            }
        }
        None => match Local.from_local_datetime(&naive_midnight) {
            chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
            chrono::LocalResult::None => None,
        },
    }
}

/// Parse a GNU military timezone used as a standalone date-time item.
///
/// GNU's grammar accepts a zone token before or after other items, for example
/// `A day ago` and `2024-01-01 12:34 A`. `J` denotes local time; all other
/// military letters represent fixed UTC offsets.
fn parse_gnu_military_timezone(
    input: &str,
    reference_time: DateTime<Local>,
    normalized_extended_year: bool,
) -> Option<DateTime<Local>> {
    let fields = input.split_ascii_whitespace().collect::<Vec<_>>();
    if fields.len() < 2 {
        return None;
    }

    let (zone_index, zone_char, offset_hours) =
        fields.iter().enumerate().find_map(|(index, field)| {
            let mut chars = field.chars();
            let zone_char = chars.next()?;
            chars.next().is_none().then(|| {
                military_timezone_offset_hours(zone_char)
                    .map(|offset_hours| (index, zone_char, offset_hours))
            })?
        })?;

    // A standalone T/t immediately after a hyphenated date starts GNU's ISO
    // date-time production, which requires an explicit time field.
    if zone_char.eq_ignore_ascii_case(&'t')
        && zone_index > 0
        && gnu_hyphenated_numeric_date(fields[zone_index - 1])
    {
        return None;
    }

    let wall_time_input = fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| (index != zone_index).then_some(*field))
        .collect::<Vec<_>>()
        .join(" ");
    let wall_time =
        parse_datetime_gnu_compat_impl(&wall_time_input, reference_time, normalized_extended_year)
            .ok()?;

    let Some(offset_hours) = offset_hours else {
        return Some(wall_time);
    };
    let utc_naive = wall_time
        .naive_local()
        .checked_sub_signed(Duration::hours(i64::from(offset_hours)))?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(utc_naive, Utc).with_timezone(&Local))
}

/// GNU accepts at most one timezone item in a date string.
///
/// The specialized parsers below remove a recognized item before recursively
/// parsing the remaining date expression.  Count named, military, numeric,
/// and attached UTC-designator items first so a second timezone cannot be
/// accepted by that recursive parse.
fn gnu_has_multiple_timezones(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut named_count = 0;
    let mut index = 0;

    while index < bytes.len() {
        let name_len = GNU_NAMED_TIMEZONES
            .iter()
            .filter_map(|(name, _)| {
                let end = index.checked_add(name.len())?;
                (end <= bytes.len()
                    && bytes[index..end].eq_ignore_ascii_case(name.as_bytes())
                    && (index == 0 || !bytes[index - 1].is_ascii_alphabetic())
                    && (end == bytes.len() || !bytes[end].is_ascii_alphabetic()))
                .then_some(name.len())
            })
            .max();
        if let Some(name_len) = name_len {
            named_count += 1;
            index += name_len;
        } else {
            index += 1;
        }
    }

    let military_count = input
        .split_ascii_whitespace()
        .filter(|field| {
            let mut chars = field.chars();
            chars.next().is_some_and(|character| {
                chars.next().is_none() && military_timezone_offset_hours(character).is_some()
            })
        })
        .count();

    let numeric_count = gnu_numeric_timezone_item_count(input);
    let attached_utc_count = input
        .as_bytes()
        .windows(2)
        .filter(|window| window[0].is_ascii_digit() && matches!(window[1], b'Z' | b'z'))
        .count();

    named_count + military_count + numeric_count + attached_utc_count > 1
}

/// Count GNU numeric timezone items without confusing signed relative values
/// such as `-5 days` for timezone offsets.  A numeric offset can either be a
/// separate field or be directly attached to a clock, and the latter form can
/// contain more than one offset in an invalid date string.
fn gnu_numeric_timezone_item_count(input: &str) -> usize {
    let fields = input.split_ascii_whitespace().collect::<Vec<_>>();

    fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let standalone = is_gnu_numeric_timezone_offset(field)
                && !fields
                    .get(index + 1)
                    .is_some_and(|next| is_gnu_relative_time_unit(next));
            usize::from(standalone) + gnu_attached_numeric_timezone_count(field)
        })
        .sum()
}

fn is_gnu_relative_time_unit(input: &str) -> bool {
    matches!(
        input.to_ascii_lowercase().as_str(),
        "year"
            | "years"
            | "month"
            | "months"
            | "fortnight"
            | "fortnights"
            | "week"
            | "weeks"
            | "day"
            | "days"
            | "hour"
            | "hours"
            | "minute"
            | "minutes"
            | "min"
            | "mins"
            | "second"
            | "seconds"
            | "sec"
            | "secs"
    )
}

fn gnu_attached_numeric_timezone_count(field: &str) -> usize {
    let bytes = field.as_bytes();
    let mut count = 0;
    let mut index = 0;

    while index < bytes.len() {
        if matches!(bytes[index], b'+' | b'-')
            && gnu_numeric_timezone_follows_clock(field, index)
            && let Some(end) = gnu_numeric_timezone_offset_end(bytes, index)
        {
            count += 1;
            index = end;
        } else {
            index += 1;
        }
    }

    count
}

fn gnu_numeric_timezone_follows_clock(field: &str, sign_index: usize) -> bool {
    let prefix = &field[..sign_index];
    if prefix.contains(':') {
        return true;
    }

    let clock = if let Some((_, time)) = prefix.rsplit_once(['T', 't']) {
        time.split(['+', '-']).next().unwrap_or_default()
    } else {
        let first_sign = prefix.find(['+', '-']).unwrap_or(sign_index);
        &field[..first_sign]
    };
    (1..=2).contains(&clock.len()) && clock.bytes().all(|byte| byte.is_ascii_digit())
}

fn gnu_numeric_timezone_offset_end(bytes: &[u8], sign_index: usize) -> Option<usize> {
    let digits_start = sign_index.checked_add(1)?;
    let digits_end = bytes[digits_start..]
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .map_or(bytes.len(), |offset| digits_start + offset);
    let hours_len = digits_end.checked_sub(digits_start)?;
    if hours_len == 0 {
        return None;
    }

    if bytes.get(digits_end) == Some(&b':') {
        let minutes_end = digits_end.checked_add(3)?;
        return ((1..=2).contains(&hours_len)
            && bytes.get(digits_end + 1).is_some_and(u8::is_ascii_digit)
            && bytes.get(digits_end + 2).is_some_and(u8::is_ascii_digit))
        .then_some(minutes_end);
    }

    (1..=4).contains(&hours_len).then_some(digits_end)
}

fn is_gnu_numeric_timezone_offset(input: &str) -> bool {
    let Some(offset) = input.strip_prefix(['+', '-']) else {
        return false;
    };

    if let Some((hours, minutes)) = offset.split_once(':') {
        return (1..=2).contains(&hours.len())
            && minutes.len() == 2
            && hours.bytes().all(|byte| byte.is_ascii_digit())
            && minutes.bytes().all(|byte| byte.is_ascii_digit());
    }

    (1..=4).contains(&offset.len()) && offset.bytes().all(|byte| byte.is_ascii_digit())
}

const GNU_NAMED_TIMEZONES: &[(&str, i32)] = &[
    ("GMT", 0),
    ("UT", 0),
    ("UTC", 0),
    ("WET", 0),
    ("WEST", 3_600),
    ("BST", 3_600),
    ("ART", -10_800),
    ("BRT", -10_800),
    ("BRST", -7_200),
    ("NST", -12_600),
    ("NDT", -9_000),
    ("AST", -14_400),
    ("ADT", -10_800),
    ("CLT", -14_400),
    ("CLST", -10_800),
    ("EST", -18_000),
    ("EDT", -14_400),
    ("CST", -21_600),
    ("CDT", -18_000),
    ("MST", -25_200),
    ("MDT", -21_600),
    ("PST", -28_800),
    ("PDT", -25_200),
    ("AKST", -32_400),
    ("AKDT", -28_800),
    ("HST", -36_000),
    ("HAST", -36_000),
    ("HADT", -32_400),
    ("SST", -43_200),
    ("WAT", 3_600),
    ("CET", 3_600),
    ("CEST", 7_200),
    ("MET", 3_600),
    ("MEZ", 3_600),
    ("MEST", 7_200),
    ("MESZ", 7_200),
    ("EET", 7_200),
    ("EEST", 10_800),
    ("CAT", 7_200),
    ("SAST", 7_200),
    ("EAT", 10_800),
    ("MSK", 10_800),
    ("MSD", 14_400),
    ("IST", 19_800),
    ("SGT", 28_800),
    ("KST", 32_400),
    ("JST", 32_400),
    ("GST", 36_000),
    ("NZST", 43_200),
    ("NZDT", 46_800),
];

const GNU_DAYLIGHT_TIMEZONES: &[&str] = &[
    "WEST", "BST", "BRST", "NDT", "ADT", "CLST", "EDT", "CDT", "MDT", "PDT", "AKDT", "HADT",
    "CEST", "MEST", "MESZ", "EEST", "MSD", "NZDT",
];

fn parse_gnu_local_timezone(
    input: &str,
    reference_time: DateTime<Local>,
    normalized_extended_year: bool,
) -> Option<Option<DateTime<Local>>> {
    let candidates: Vec<(String, bool)> = (0..=3)
        .map(|quarter| reference_time + Duration::days(quarter * 90))
        .filter_map(|probe| local_timezone_info(probe.timestamp()))
        .filter(|(name, _)| {
            !name.is_empty()
                && name.bytes().all(|byte| byte.is_ascii_alphabetic())
                && !["GMT", "UT", "UTC"].contains(&name.as_str())
        })
        .collect();

    let bytes = input.as_bytes();
    let (start, end, zone_name) = candidates
        .iter()
        .flat_map(|(name, _)| {
            bytes
                .windows(name.len())
                .enumerate()
                .filter(move |(index, candidate)| {
                    candidate.eq_ignore_ascii_case(name.as_bytes())
                        && (*index == 0 || !bytes[*index - 1].is_ascii_alphabetic())
                        && (*index + name.len() == bytes.len()
                            || !bytes[*index + name.len()].is_ascii_alphabetic())
                })
                .map(move |(index, _)| (index, index + name.len(), name.as_str()))
        })
        .min_by_key(|(start, _, _)| *start)?;

    let suffix = &input[end..];
    let trimmed_suffix = suffix.trim_start();
    let has_dst_suffix = trimmed_suffix
        .get(..3)
        .is_some_and(|word| word.eq_ignore_ascii_case("DST"))
        && trimmed_suffix
            .as_bytes()
            .get(3)
            .is_none_or(|byte| !byte.is_ascii_alphabetic());
    let suffix_start = if has_dst_suffix {
        end + (suffix.len() - trimmed_suffix.len()) + 3
    } else {
        end
    };

    let wall_time_input = format!("{} {}", &input[..start], &input[suffix_start..]);
    let parsed = match parse_datetime_gnu_compat_impl(
        wall_time_input.trim(),
        reference_time,
        normalized_extended_year,
    ) {
        Ok(parsed) => parsed,
        Err(_) => return Some(None),
    };
    let (parsed_zone_name, parsed_is_dst) = match local_timezone_info(parsed.timestamp()) {
        Some(info) => info,
        None => return Some(None),
    };
    if has_dst_suffix {
        Some(parsed_is_dst.then_some(parsed))
    } else {
        Some(
            parsed_zone_name
                .eq_ignore_ascii_case(zone_name)
                .then_some(parsed),
        )
    }
}

#[cfg(target_os = "linux")]
fn local_timezone_info(timestamp: i64) -> Option<(String, bool)> {
    let timestamp: libc::time_t = timestamp;
    let mut local_tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let result = unsafe { libc::localtime_r(&timestamp, local_tm.as_mut_ptr()) };
    if result.is_null() {
        return None;
    }
    let local_tm = unsafe { local_tm.assume_init() };
    if local_tm.tm_zone.is_null() {
        return None;
    }
    let name = unsafe { CStr::from_ptr(local_tm.tm_zone) }
        .to_str()
        .ok()?
        .to_string();
    Some((name, local_tm.tm_isdst > 0))
}

#[cfg(not(target_os = "linux"))]
fn local_timezone_info(_timestamp: i64) -> Option<(String, bool)> {
    None
}

fn parse_gnu_named_timezone(
    input: &str,
    reference_time: DateTime<Local>,
    normalized_extended_year: bool,
) -> Option<DateTime<Local>> {
    let bytes = input.as_bytes();
    let (start, end, mut offset_seconds, zone_name) = GNU_NAMED_TIMEZONES
        .iter()
        .flat_map(|(name, offset)| {
            bytes
                .windows(name.len())
                .enumerate()
                .filter(move |(index, candidate)| {
                    candidate.eq_ignore_ascii_case(name.as_bytes())
                        && (*index == 0 || !bytes[*index - 1].is_ascii_alphabetic())
                        && (*index + name.len() == bytes.len()
                            || !bytes[*index + name.len()].is_ascii_alphabetic())
                })
                .map(move |(index, _)| (index, index + name.len(), *offset, *name))
        })
        .min_by_key(|(start, _, _, _)| *start)?;

    let mut suffix_start = end;
    let suffix = &input[end..];
    let trimmed_suffix = suffix.trim_start();
    if trimmed_suffix
        .get(..3)
        .is_some_and(|word| word.eq_ignore_ascii_case("DST"))
        && trimmed_suffix
            .as_bytes()
            .get(3)
            .is_none_or(|byte| !byte.is_ascii_alphabetic())
    {
        if GNU_DAYLIGHT_TIMEZONES.contains(&zone_name) {
            return None;
        }
        offset_seconds += 3_600;
        suffix_start = end + (suffix.len() - trimmed_suffix.len()) + 3;
    }

    let wall_time_input = format!("{} {}", &input[..start], &input[suffix_start..]);
    let wall_time_input = wall_time_input.trim();
    let naive = if wall_time_input.is_empty() {
        reference_time.date_naive().and_hms_opt(0, 0, 0)?
    } else {
        parse_datetime_gnu_compat_impl(wall_time_input, reference_time, normalized_extended_year)
            .ok()?
            .naive_local()
    };
    let offset = FixedOffset::east_opt(offset_seconds)?;
    offset
        .from_local_datetime(&naive)
        .earliest()
        .map(|date| date.with_timezone(&Local))
}

fn parse_embedded_timezone(input: &str) -> Option<DateTime<Local>> {
    let rest = input.strip_prefix("TZ=\"")?;
    let quote_idx = rest.find('"')?;
    let timezone_name = &rest[..quote_idx];
    let date_str = rest[quote_idx + 1..].trim();
    let naive = parse_embedded_timezone_datetime(date_str)?;

    if let Ok(timezone) = timezone_name.parse::<Tz>() {
        return timezone
            .from_local_datetime(&naive)
            .earliest()
            .map(|dt| dt.with_timezone(&Local));
    }

    let offset_hours = if timezone_name.starts_with("EST") {
        -5
    } else if timezone_name.starts_with("PST") {
        -8
    } else if timezone_name == "UTC0" || timezone_name == "GMT0" {
        0
    } else {
        return None;
    };
    FixedOffset::east_opt(offset_hours * 3600)?
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.with_timezone(&Local))
}

fn parse_embedded_timezone_datetime(input: &str) -> Option<NaiveDateTime> {
    for format in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(date) = NaiveDateTime::parse_from_str(input, format) {
            return Some(date);
        }
    }

    NaiveDate::parse_from_str(input, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)
}

fn parse_rfc5322_datetime(input: &str) -> Option<DateTime<Local>> {
    let date_time = if let Some((weekday, rest)) = input.split_once(',') {
        if parse_weekday_name(weekday.trim()).is_some() {
            rest.trim()
        } else {
            input
        }
    } else {
        input
    };

    for format in [
        "%d %b %Y %H:%M:%S %z",
        "%d %b %Y %H:%M %z",
        "%d %B %Y %H:%M:%S %z",
        "%d %B %Y %H:%M %z",
    ] {
        if let Ok(date) = DateTime::parse_from_str(date_time, format) {
            return Some(date.with_timezone(&Local));
        }
    }

    None
}

fn parse_compact_time_of_day(
    input: &str,
    reference_time: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let time = parse_gnu_compact_clock(input)?;
    let naive = reference_time.date_naive().and_time(time);

    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
        chrono::LocalResult::None => None,
    }
}

fn parse_gnu_compact_clock(input: &str) -> Option<NaiveTime> {
    let (hour_part, minute_part) = match input.len() {
        1 | 2 => (input, "0"),
        3 => input.split_at(1),
        4 => input.split_at(2),
        _ => return None,
    };

    let hour = hour_part.parse::<u32>().ok()?;
    let minute = minute_part.parse::<u32>().ok()?;
    chrono::NaiveTime::from_hms_opt(hour, minute, 0)
}

/// Parse a GNU time-of-day with a directly attached UTC designator, such as
/// `12Z` or `12:34:56.123z`.
fn parse_gnu_time_only_utc_designator(
    input: &str,
    reference_time: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let clock = input
        .strip_suffix('Z')
        .or_else(|| input.strip_suffix('z'))?;
    let time = parse_gnu_24_hour_clock(clock).or_else(|| parse_gnu_compact_clock(clock))?;
    let naive = reference_time.date_naive().and_time(time);

    Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc).with_timezone(&Local))
}

/// Parse a GNU date-only UTC form after its `Z`, `UTC`, or `GMT` suffix has
/// been normalized to `+0000`.
fn parse_gnu_utc_date(input: &str) -> Option<DateTime<Local>> {
    let date = input.strip_suffix("+0000")?;
    let naive = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)?;

    Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc).with_timezone(&Local))
}

#[derive(Clone, Copy)]
enum Meridian {
    Am,
    Pm,
}

/// Parse GNU's 12-hour clock item independently from the date item.
fn parse_gnu_meridian_datetime(
    input: &str,
    reference_time: DateTime<Local>,
    normalized_extended_year: bool,
) -> Option<DateTime<Local>> {
    let tokens: Vec<&str> = input.split_ascii_whitespace().collect();
    let (time, time_start, time_end) = find_gnu_meridian_time(&tokens)?;
    let date_input = tokens
        .iter()
        .enumerate()
        .filter(|(index, _)| *index < time_start || *index >= time_end)
        .map(|(_, token)| *token)
        .collect::<Vec<_>>()
        .join(" ");

    let date_time = if date_input.is_empty() {
        reference_time
    } else {
        parse_datetime_gnu_compat_impl(&date_input, reference_time, normalized_extended_year)
            .ok()?
    };
    let naive = date_time.date_naive().and_time(time);

    match date_time.timezone().from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
        chrono::LocalResult::None => None,
    }
}

fn find_gnu_meridian_time(tokens: &[&str]) -> Option<(NaiveTime, usize, usize)> {
    let mut result = None;

    for (index, token) in tokens.iter().enumerate() {
        if let Some((clock, meridian)) = split_gnu_meridian_suffix(token) {
            let time = parse_gnu_meridian_clock(clock, meridian)?;
            if result.replace((time, index, index + 1)).is_some() {
                return None;
            }
        }
    }
    if result.is_some() {
        return result;
    }

    for (index, pair) in tokens.windows(2).enumerate() {
        if let Some(meridian) = parse_gnu_meridian_word(pair[1]) {
            let time = parse_gnu_meridian_clock(pair[0], meridian)?;
            if result.replace((time, index, index + 2)).is_some() {
                return None;
            }
        }
    }

    result
}

fn split_gnu_meridian_suffix(input: &str) -> Option<(&str, Meridian)> {
    let lower = input.to_ascii_lowercase();
    for (suffix, meridian) in [
        ("a.m.", Meridian::Am),
        ("p.m.", Meridian::Pm),
        ("am", Meridian::Am),
        ("pm", Meridian::Pm),
    ] {
        if lower.ends_with(suffix) && input.len() > suffix.len() {
            return Some((&input[..input.len() - suffix.len()], meridian));
        }
    }
    None
}

fn parse_gnu_meridian_word(input: &str) -> Option<Meridian> {
    match input.to_ascii_lowercase().as_str() {
        "am" | "a.m." => Some(Meridian::Am),
        "pm" | "p.m." => Some(Meridian::Pm),
        _ => None,
    }
}

fn parse_gnu_meridian_clock(input: &str, meridian: Meridian) -> Option<NaiveTime> {
    let mut components = input.split(':');
    let hour = components.next()?.parse::<u32>().ok()?;
    let minute = components
        .next()
        .map_or(Some(0), |value| value.parse().ok())?;
    let seconds = components.next().unwrap_or("0");
    if components.next().is_some() || !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }

    let (second, nanoseconds) = if let Some((second, fraction)) = seconds.split_once('.') {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let mut nanos = fraction
            .bytes()
            .take(9)
            .fold(0u32, |value, digit| value * 10 + u32::from(digit - b'0'));
        for _ in fraction.len().min(9)..9 {
            nanos *= 10;
        }
        (second.parse::<u32>().ok()?, nanos)
    } else {
        (seconds.parse::<u32>().ok()?, 0)
    };
    if second > 59 {
        return None;
    }

    let hour = match meridian {
        Meridian::Am if hour == 12 => 0,
        Meridian::Am => hour,
        Meridian::Pm if hour == 12 => 12,
        Meridian::Pm => hour + 12,
    };
    NaiveTime::from_hms_nano_opt(hour, minute, second, nanoseconds)
}

fn parse_weekday_name(input: &str) -> Option<Weekday> {
    match input.trim_end_matches(',').to_ascii_lowercase().as_str() {
        "sunday" | "sun" => Some(Weekday::Sun),
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" | "tues" => Some(Weekday::Tue),
        "wednesday" | "wed" | "wednes" => Some(Weekday::Wed),
        "thursday" | "thu" | "thur" | "thurs" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        _ => None,
    }
}

fn parse_weekday_ordinal(input: &str) -> Option<i32> {
    match input.to_ascii_lowercase().as_str() {
        "last" => Some(-1),
        "this" => Some(0),
        "next" | "first" => Some(1),
        "third" => Some(3),
        "fourth" => Some(4),
        "fifth" => Some(5),
        "sixth" => Some(6),
        "seventh" => Some(7),
        "eighth" => Some(8),
        "ninth" => Some(9),
        "tenth" => Some(10),
        "eleventh" => Some(11),
        "twelfth" => Some(12),
        value if value.bytes().all(|byte| byte.is_ascii_digit()) => value.parse().ok(),
        _ => None,
    }
}

fn is_gnu_month_name(input: &str) -> bool {
    matches!(
        input.trim_matches(',').to_ascii_lowercase().as_str(),
        "january"
            | "jan"
            | "february"
            | "feb"
            | "march"
            | "mar"
            | "april"
            | "apr"
            | "may"
            | "june"
            | "jun"
            | "july"
            | "jul"
            | "august"
            | "aug"
            | "september"
            | "sep"
            | "sept"
            | "october"
            | "oct"
            | "november"
            | "nov"
            | "december"
            | "dec"
    )
}

fn is_gnu_date_number(input: &str) -> bool {
    let input = input.trim_matches(',');
    !input.is_empty() && input.bytes().all(|byte| byte.is_ascii_digit())
}

fn strip_weekday_from_explicit_date(input: &str) -> Option<String> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    let weekday_index = parts
        .iter()
        .position(|part| parse_weekday_name(part).is_some())?;
    let weekday_start = if weekday_index > 0 {
        let modifier = parts[weekday_index - 1];
        if parse_weekday_ordinal(modifier).is_some() {
            weekday_index - 1
        } else {
            weekday_index
        }
    } else {
        weekday_index
    };

    let remaining = parts
        .iter()
        .enumerate()
        .filter(|(index, _)| *index < weekday_start || *index > weekday_index)
        .map(|(_, part)| *part)
        .collect::<Vec<_>>();
    let has_explicit_iso_date = remaining.iter().any(|part| {
        let date = part
            .trim_matches(',')
            .split(['T', 't'])
            .next()
            .unwrap_or(part);
        NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok()
    });
    let has_explicit_word_month_date = remaining.windows(2).any(|parts| {
        let [first, second] = parts else {
            return false;
        };
        (is_gnu_date_number(first) && is_gnu_month_name(second))
            || (is_gnu_month_name(first) && is_gnu_date_number(second))
    });
    if !(has_explicit_iso_date || has_explicit_word_month_date) {
        return None;
    }

    Some(remaining.join(" "))
}

/// 解析包含星期几名称的表达式
fn parse_weekday_expression(
    input: &str,
    reference_time: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    let (ordinal, weekday) = match parts.as_slice() {
        [weekday] => (0, *weekday),
        [ordinal, weekday] => (parse_weekday_ordinal(ordinal)?, *weekday),
        _ => return None,
    };
    let target_weekday = parse_weekday_name(weekday)?;
    let current_weekday = reference_time.weekday();

    let current = current_weekday.num_days_from_sunday() as i32;
    let target = target_weekday.num_days_from_sunday() as i32;
    let ordinal_weeks = ordinal - i32::from(ordinal > 0 && current != target);
    let days_offset = ordinal_weeks * 7 + (target - current + 7) % 7;

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

/// Parse the GNU day-shift words when they are combined with one clock time.
///
/// GNU's grammar treats a day shift (for example, `tomorrow`) and a time of
/// day as separate items, independent of their order.  The generic fallback
/// parser does not compose these items, so handle this narrow grammar before
/// falling back to the individual relative-time paths.
fn parse_relative_day_with_explicit_time(
    input: &str,
    reference_time: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let mut day_offset = None;
    let mut time = None;

    for token in input.split_ascii_whitespace() {
        match token.to_ascii_lowercase().as_str() {
            "tomorrow" if day_offset.is_none() => day_offset = Some(1),
            "yesterday" if day_offset.is_none() => day_offset = Some(-1),
            "today" | "now" if day_offset.is_none() => day_offset = Some(0),
            _ if time.is_none() => {
                time = ["%H:%M:%S%.f", "%H:%M:%S", "%H:%M"]
                    .into_iter()
                    .find_map(|format| NaiveTime::parse_from_str(token, format).ok());
                time?;
            }
            _ => return None,
        }
    }

    let date = reference_time
        .date_naive()
        .checked_add_signed(Duration::days(day_offset?))?;
    match reference_time
        .timezone()
        .from_local_datetime(&date.and_time(time?))
    {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
        chrono::LocalResult::None => None,
    }
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
    use chrono::{Local, TimeZone, Timelike, Utc};

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
    fn test_parse_weekday_with_gnu_ordinals() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap(); // Thursday

        for (input, expected) in [
            (
                "first monday",
                NaiveDate::from_ymd_opt(2025, 7, 28).unwrap(),
            ),
            (
                "third monday",
                NaiveDate::from_ymd_opt(2025, 8, 11).unwrap(),
            ),
            ("5 monday", NaiveDate::from_ymd_opt(2025, 8, 25).unwrap()),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.date_naive(), expected, "input {input}");
            assert_eq!(parsed.hour(), 0, "input {input}");
        }

        let explicit = parse_datetime_gnu_compat("third monday 2024-01-01", ref_time).unwrap();
        assert_eq!(
            explicit.date_naive(),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
        );
    }

    #[test]
    fn test_explicit_date_ignores_weekday() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in ["2024-02-29 next Fri", "next Fri 2024-02-29"] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.date_naive(),
                NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
                "input {input}"
            );
            assert_eq!(parsed.hour(), 0, "input {input}");
            assert_eq!(parsed.minute(), 0, "input {input}");
            assert_eq!(parsed.second(), 0, "input {input}");
        }
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

        let hence = parse_datetime_gnu_compat("1 day hence", ref_time).unwrap();
        assert_eq!(hence.day(), 25);
    }

    #[test]
    fn test_parse_gnu_this_relative_unit_aliases() {
        let ref_time = Local.timestamp_opt(1_000_000, 123_456_789).unwrap();

        for input in [
            "this year",
            "this years",
            "this month",
            "this months",
            "this fortnight",
            "this fortnights",
            "this week",
            "this weeks",
            "this day",
            "this days",
            "this hour",
            "this hours",
            "this minute",
            "this minutes",
            "this min",
            "this mins",
            "this second",
            "this seconds",
            "this sec",
            "this secs",
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed, ref_time, "input {input}");
        }
    }

    #[test]
    fn test_parse_gnu_ordinal_relative_units() {
        let ref_time = Local.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();

        for (input, expected_date, expected_time) in [
            (
                "first fortnight",
                NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
                NaiveTime::MIN,
            ),
            (
                "third years",
                NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
                NaiveTime::MIN,
            ),
            (
                "fourth months",
                NaiveDate::from_ymd_opt(2024, 5, 1).unwrap(),
                NaiveTime::MIN,
            ),
            (
                "fifth weeks",
                NaiveDate::from_ymd_opt(2024, 2, 5).unwrap(),
                NaiveTime::MIN,
            ),
            (
                "sixth days",
                NaiveDate::from_ymd_opt(2024, 1, 7).unwrap(),
                NaiveTime::MIN,
            ),
            (
                "seventh hours",
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
            ),
            (
                "eighth minutes",
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 8, 0).unwrap(),
            ),
            (
                "ninth mins",
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 9, 0).unwrap(),
            ),
            (
                "tenth seconds",
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 10).unwrap(),
            ),
            (
                "eleventh secs",
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 11).unwrap(),
            ),
            (
                "twelfth day",
                NaiveDate::from_ymd_opt(2024, 1, 13).unwrap(),
                NaiveTime::MIN,
            ),
            (
                "third day 2024-01-01",
                NaiveDate::from_ymd_opt(2024, 1, 4).unwrap(),
                NaiveTime::MIN,
            ),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.date_naive(), expected_date, "input {input}");
            assert_eq!(parsed.time(), expected_time, "input {input}");
        }
    }

    #[test]
    fn test_parse_gnu_fractional_relative_units() {
        let ref_time = Local.timestamp_opt(1_000_000, 0).unwrap();

        for input in [
            "1.5 years",
            "1.5 months",
            "1.5 fortnights",
            "1.5 weeks",
            "1.5 days",
            "1.5 hours",
            "1.5 minutes",
            "1.0 days",
        ] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} must be rejected"
            );
        }

        for input in ["1.5 seconds", "1.5 sec", "1.5 secs"] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.timestamp(),
                ref_time.timestamp() + 1,
                "input {input}"
            );
            assert_eq!(
                parsed.timestamp_subsec_nanos(),
                500_000_000,
                "input {input}"
            );
        }
    }

    #[test]
    fn test_parse_gnu_fractional_second_lexemes() {
        let ref_time = Local.timestamp_opt(1_000_000, 0).unwrap();

        for input in [
            "1.2.3 seconds",
            "1..5 seconds",
            ".5 seconds",
            "1. seconds",
            "+.5 sec",
            "-.5 sec",
            "999999999999999999999 sec",
        ] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} must be rejected"
            );
        }

        let comma = parse_datetime_gnu_compat("1,5 seconds", ref_time).unwrap();
        assert_eq!(comma.timestamp(), ref_time.timestamp() + 1);
        assert_eq!(comma.timestamp_subsec_nanos(), 500_000_000);

        let positive = parse_datetime_gnu_compat("1.1234567891 seconds", ref_time).unwrap();
        assert_eq!(positive.timestamp(), ref_time.timestamp() + 1);
        assert_eq!(positive.timestamp_subsec_nanos(), 123_456_789);

        let negative = parse_datetime_gnu_compat("-1.1234567891 seconds", ref_time).unwrap();
        assert_eq!(negative.timestamp(), ref_time.timestamp() - 2);
        assert_eq!(negative.timestamp_subsec_nanos(), 876_543_210);
    }

    #[test]
    fn test_parse_gnu_dotted_date_and_timezone_words() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        let month = parse_datetime_gnu_compat("Feb. 29 2024", ref_time).unwrap();
        assert_eq!(
            month.date_naive(),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
        );

        let weekday = parse_datetime_gnu_compat("Mon. 2024-01-01", ref_time).unwrap();
        assert_eq!(
            weekday.date_naive(),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
        );

        let timezone = parse_datetime_gnu_compat("2024-01-01 00:00 P.S.T.", ref_time).unwrap();
        assert_eq!(timezone.with_timezone(&Utc).timestamp(), 1_704_096_000);

        for input in ["Sept. 1 2024", "1 day ago.", "2024-01-01 00:00 A."] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} must be rejected"
            );
        }
    }

    #[test]
    fn test_parse_relative_day_with_explicit_time() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 8, 0, 0).unwrap();

        for (input, expected_day, expected_hour, expected_minute, expected_second) in [
            ("tomorrow 12:34:56", 25, 12, 34, 56),
            ("yesterday 01:02:03", 23, 1, 2, 3),
            ("today 23:45:00", 24, 23, 45, 0),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.day(), expected_day, "input {input}");
            assert_eq!(parsed.hour(), expected_hour, "input {input}");
            assert_eq!(parsed.minute(), expected_minute, "input {input}");
            assert_eq!(parsed.second(), expected_second, "input {input}");
        }
    }

    #[test]
    fn test_parse_numeric_dates_with_explicit_time() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 8, 0, 0).unwrap();

        for input in ["2020/01/02 03:04:05", "20200102 03:04:05"] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.date_naive(),
                NaiveDate::from_ymd_opt(2020, 1, 2).unwrap()
            );
            assert_eq!(parsed.hour(), 3);
            assert_eq!(parsed.minute(), 4);
            assert_eq!(parsed.second(), 5);
        }
    }

    #[test]
    fn test_parse_military_timezone_after_datetime() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 8, 0, 0).unwrap();

        for (input, expected_utc) in [
            (
                "2024-01-01 12:34 A",
                Utc.with_ymd_and_hms(2024, 1, 1, 11, 34, 0).unwrap(),
            ),
            (
                "2024-01-01 12:34 N",
                Utc.with_ymd_and_hms(2024, 1, 1, 13, 34, 0).unwrap(),
            ),
            (
                "2024-01-01 12:34 Z",
                Utc.with_ymd_and_hms(2024, 1, 1, 12, 34, 0).unwrap(),
            ),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.timestamp(),
                expected_utc.timestamp(),
                "input {input}"
            );
        }
    }

    #[test]
    fn test_parse_fortnight_relative_to_explicit_date() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        let parsed = parse_datetime_gnu_compat("2024-01-01 2 fortnights ago", ref_time).unwrap();

        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(2023, 12, 4).unwrap()
        );
        assert_eq!(parsed.hour(), 0);
    }

    #[test]
    fn test_parse_multiple_calendar_relative_units_together() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        let parsed = parse_datetime_gnu_compat("2024-02-29 1 day 1 month", ref_time).unwrap();

        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(2024, 3, 30).unwrap()
        );
        assert_eq!(parsed.time(), NaiveTime::MIN);

        let parsed = parse_datetime_gnu_compat("2024-03-31 1 month 1 day ago", ref_time).unwrap();
        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(2024, 4, 30).unwrap()
        );
        assert_eq!(parsed.time(), NaiveTime::MIN);
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
    fn test_parse_military_timezone_only() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, offset_hours) in [("a", 1), ("Z", 0), ("n", -1), ("t", -7)] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            let expected = chrono::FixedOffset::east_opt(offset_hours * 3600)
                .unwrap()
                .with_ymd_and_hms(2025, 7, 24, 0, 0, 0)
                .unwrap()
                .with_timezone(&Local);
            assert_eq!(parsed.timestamp(), expected.timestamp(), "input {input}");
            assert_eq!(
                parsed.timestamp_subsec_nanos(),
                expected.timestamp_subsec_nanos(),
                "input {input}"
            );
        }

        let local = parse_datetime_gnu_compat("j", ref_time).unwrap();
        assert_eq!(local.date_naive(), ref_time.date_naive());
        assert_eq!(local.hour(), 0);
        assert_eq!(local.minute(), 0);
        assert_eq!(local.second(), 0);
    }

    #[test]
    fn test_parse_embedded_iana_timezone() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, expected_utc) in [
            (
                "TZ=\"America/Los_Angeles\" 2024-07-01 09:00",
                Utc.with_ymd_and_hms(2024, 7, 1, 16, 0, 0).unwrap(),
            ),
            (
                "TZ=\"America/Los_Angeles\" 2024-01-01 09:00",
                Utc.with_ymd_and_hms(2024, 1, 1, 17, 0, 0).unwrap(),
            ),
            (
                "TZ=\"America/Los_Angeles\" 2024-11-03 01:30",
                Utc.with_ymd_and_hms(2024, 11, 3, 8, 30, 0).unwrap(),
            ),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.timestamp(),
                expected_utc.timestamp(),
                "input {input}"
            );
        }

        assert!(
            parse_datetime_gnu_compat("TZ=\"America/Los_Angeles\" 2024-03-10 02:30", ref_time)
                .is_err()
        );
    }

    #[test]
    fn test_parse_multiple_bare_calendar_relative_units_together() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 34, 56).unwrap();
        let parsed = parse_datetime_gnu_compat("day month", ref_time).unwrap();

        assert_eq!(
            parsed.naive_local(),
            NaiveDate::from_ymd_opt(2025, 8, 25)
                .unwrap()
                .and_hms_opt(12, 34, 56)
                .unwrap()
        );
    }

    #[test]
    fn test_parse_bare_clock_relative_units_with_calendar_unit() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 34, 56).unwrap();

        for (input, expected) in [
            (
                "second month",
                NaiveDate::from_ymd_opt(2025, 8, 24)
                    .unwrap()
                    .and_hms_opt(12, 34, 57)
                    .unwrap(),
            ),
            (
                "sec month",
                NaiveDate::from_ymd_opt(2025, 8, 24)
                    .unwrap()
                    .and_hms_opt(12, 34, 57)
                    .unwrap(),
            ),
            (
                "minute month",
                NaiveDate::from_ymd_opt(2025, 8, 24)
                    .unwrap()
                    .and_hms_opt(12, 35, 56)
                    .unwrap(),
            ),
            (
                "hour month",
                NaiveDate::from_ymd_opt(2025, 8, 24)
                    .unwrap()
                    .and_hms_opt(13, 34, 56)
                    .unwrap(),
            ),
            (
                "1 second month",
                NaiveDate::from_ymd_opt(2025, 8, 24)
                    .unwrap()
                    .and_hms_opt(12, 34, 57)
                    .unwrap(),
            ),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.naive_local(), expected, "input {input}");
        }
    }

    #[test]
    fn test_parse_rfc5322_datetime() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let expected = Utc.with_ymd_and_hms(2024, 2, 29, 7, 4, 56).unwrap();

        for input in [
            "Thu, 29 Feb 2024 12:34:56 +0530",
            "Fri, 29 Feb 2024 12:34:56 +0530",
            "29 Feb 2024 12:34:56 +0530",
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.timestamp(), expected.timestamp(), "input {input}");
        }
    }

    #[test]
    fn test_weekday_is_ignored_for_explicit_word_month_dates() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, expected) in [
            (
                "Friday, 29 February 2024",
                Utc.with_ymd_and_hms(2024, 2, 29, 0, 0, 0).unwrap(),
            ),
            (
                "Fri, 29 Feb 2024",
                Utc.with_ymd_and_hms(2024, 2, 29, 0, 0, 0).unwrap(),
            ),
            (
                "Thu, 29 Feb 2024 12:34:56 UTC",
                Utc.with_ymd_and_hms(2024, 2, 29, 12, 34, 56).unwrap(),
            ),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.timestamp(), expected.timestamp(), "input {input}");
        }
    }

    #[test]
    fn test_parse_gnu_named_timezones() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, expected_utc) in [
            (
                "2024-01-01 12:00 EST",
                Utc.with_ymd_and_hms(2024, 1, 1, 17, 0, 0).unwrap(),
            ),
            (
                "NST 2024-01-01 12:00",
                Utc.with_ymd_and_hms(2024, 1, 1, 15, 30, 0).unwrap(),
            ),
            (
                "2024-01-01 12:00IST",
                Utc.with_ymd_and_hms(2024, 1, 1, 6, 30, 0).unwrap(),
            ),
            (
                "2024-01-01 12:00 NZDT",
                Utc.with_ymd_and_hms(2023, 12, 31, 23, 0, 0).unwrap(),
            ),
            (
                "2024-01-01 12:00 EST DST",
                Utc.with_ymd_and_hms(2024, 1, 1, 16, 0, 0).unwrap(),
            ),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.timestamp(),
                expected_utc.timestamp(),
                "input {input}"
            );
        }

        assert!(parse_datetime_gnu_compat("2024-01-01 12:00 EDT DST", ref_time).is_err());
    }

    #[test]
    fn test_rejects_multiple_gnu_timezone_items() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in [
            "UTC UTC",
            "UTC PST",
            "2024-01-01 UTC UTC",
            "2024-01-01 12:00 UTC PST",
            "A UTC",
            "12Z UTC",
            "2024-01-01 12:00 +0000 +0100",
            "2024-01-01 12:00+0000 +0100",
            "2024-01-01 12:00+0100 UTC",
        ] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} must be rejected"
            );
        }
    }

    #[test]
    fn test_named_timezone_with_signed_relative_day_is_not_a_second_timezone() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("2024-01-01 12:00 UTC -5 days", ref_time)
            .expect("GNU accepts a signed relative day after a named timezone");
        assert_eq!(
            parsed.with_timezone(&Utc),
            Utc.with_ymd_and_hms(2023, 12, 27, 12, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_parse_gnu_numeric_timezones() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, expected_utc) in [
            ("2024-01-01 12:00 +530", (2024, 1, 1, 6, 30, 0)),
            ("2024-01-01 12:00 -530", (2024, 1, 1, 17, 30, 0)),
            ("2024-01-01 12:00 +1260", (2023, 12, 31, 23, 0, 0)),
            ("2024-01-01 12:00 +2400", (2023, 12, 31, 12, 0, 0)),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            let (year, month, day, hour, minute, second) = expected_utc;
            let expected = Utc
                .with_ymd_and_hms(year, month, day, hour, minute, second)
                .unwrap();
            assert_eq!(parsed.timestamp(), expected.timestamp(), "input {input}");
        }

        for input in [
            "2024-02-29 12:00 +24:01",
            "2024-02-29 12:00 +2401",
            "2024-02-29 12:00 +25:00",
        ] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} must be rejected"
            );
        }
    }

    #[test]
    fn test_rejects_numeric_timezone_without_time_of_day() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in [
            "2024-02-29 +0",
            "2024-02-29 +00",
            "2024-02-29 +000",
            "2024-02-29 +0000",
            "2024-02-29+0000",
            "2024-02-29 +0530",
            "2024-02-29 -0500",
            "1/2/24 +0530",
        ] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} should fail"
            );
        }
    }

    #[test]
    fn test_parse_gnu_numeric_timezone_accepts_variable_width_offsets() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, expected_utc) in [
            ("2024-01-01 00:00 +01234", (2023, 12, 31, 11, 26, 0)),
            ("2024-01-01 00:00 +1:2", (2023, 12, 31, 22, 58, 0)),
            ("2024-01-01 00:00 -1:2", (2024, 1, 1, 1, 2, 0)),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            let (year, month, day, hour, minute, second) = expected_utc;
            let expected = Utc
                .with_ymd_and_hms(year, month, day, hour, minute, second)
                .unwrap();
            assert_eq!(parsed.timestamp(), expected.timestamp(), "input {input}");
        }
    }

    #[test]
    fn test_parse_gnu_extended_years() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let expected = Utc.with_ymd_and_hms(12345, 1, 1, 12, 34, 56).unwrap();

        for input in ["12345-01-01 12:34:56 UTC", "12345/01/01 12:34:56 UTC"] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.timestamp(), expected.timestamp(), "input {input}");
        }
        assert!(parse_datetime_gnu_compat("+12345-01-01 UTC", ref_time).is_err());
    }

    #[test]
    fn test_parse_full_month_name_with_comma() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("January 1, 2024 12:00 UTC", ref_time).unwrap();

        assert_eq!(parsed.timestamp(), 1_704_110_400);
    }

    #[test]
    fn test_ignores_gnu_parenthesized_comments() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in [
            "(comment) 2024-01-01 12:34 UTC",
            "2024-01-01 (comment) 12:34 UTC",
            "2024-01-01 12:34 UTC (comment)",
            "2024-01-01 ((nested) comment) 12:34 UTC",
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.timestamp(), 1_704_112_440, "input {input}");
        }
    }

    #[test]
    fn test_parse_gnu_meridian_times() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, expected_hour, expected_minute, expected_second, expected_nanos) in [
            ("2024-02-29 12pm UTC", 12, 0, 0, 0),
            ("2024-02-29 12:34pm UTC", 12, 34, 0, 0),
            ("2024-02-29 12:34:56.5pm UTC", 12, 34, 56, 500_000_000),
            ("12pm 2024-02-29 UTC", 12, 0, 0, 0),
            ("2024-02-29 1:2 p.m. UTC", 13, 2, 0, 0),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.date_naive(),
                NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
            );
            assert_eq!(parsed.hour(), expected_hour, "input {input}");
            assert_eq!(parsed.minute(), expected_minute, "input {input}");
            assert_eq!(parsed.second(), expected_second, "input {input}");
            assert_eq!(
                parsed.timestamp_subsec_nanos(),
                expected_nanos,
                "input {input}"
            );
        }
    }

    #[test]
    fn test_parse_iso_utc_designator_without_seconds() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, expected_hour, expected_minute) in [
            ("2024-02-29Z", 0, 0),
            ("2024-02-29T12:34Z", 12, 34),
            ("2024-02-29 12:34z", 12, 34),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.date_naive(),
                NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
                "input {input}"
            );
            assert_eq!(parsed.hour(), expected_hour, "input {input}");
            assert_eq!(parsed.minute(), expected_minute, "input {input}");
            assert_eq!(parsed.offset().local_minus_utc(), 0, "input {input}");
        }
    }

    #[test]
    fn test_rejects_iso_hour_followed_by_utc_designator() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in ["2024-02-29T00Z", "2024-02-29T12Z"] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} should fail"
            );
        }
    }

    #[test]
    fn test_parse_gnu_day_month_name_hyphen_date() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("17-JUN-1992", ref_time).unwrap();

        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(1992, 6, 17).unwrap()
        );
        assert_eq!(parsed.time(), NaiveTime::MIN);
    }

    #[test]
    fn test_parse_gnu_slash_date_prefers_month_day_two_digit_year() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("1/2/24", ref_time).unwrap();

        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()
        );
        assert_eq!(parsed.time(), NaiveTime::MIN);
    }

    #[test]
    fn test_parse_gnu_slash_date_keeps_single_digit_year_literal() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("1/2/1", ref_time).unwrap();

        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(1, 1, 2).unwrap()
        );
        assert_eq!(parsed.time(), NaiveTime::MIN);
    }

    #[test]
    fn test_rejects_short_year_first_slash_date() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        assert!(parse_datetime_gnu_compat("24/1/2", ref_time).is_err());
    }

    #[test]
    fn test_rejects_gnu_invalid_space_separated_numeric_date() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        assert!(parse_datetime_gnu_compat("2024 02 29", ref_time).is_err());
    }

    #[test]
    fn test_parse_gnu_month_name_comma_date() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("Jan 2, 2024", ref_time).unwrap();

        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()
        );
        assert_eq!(parsed.time(), NaiveTime::MIN);
    }

    #[test]
    fn test_parse_gnu_full_month_name_without_comma() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("January 1 1970", ref_time).unwrap();

        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
        );
        assert_eq!(parsed.time(), NaiveTime::MIN);
    }

    #[test]
    fn test_parse_gnu_date_without_year_uses_reference_year() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in ["01/02", "Jan 2", "2 Jan"] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
                "input {input}"
            );
            assert_eq!(parsed.time(), NaiveTime::MIN, "input {input}");
        }
    }

    #[test]
    fn test_parse_gnu_iso_hour_with_numeric_timezone() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("2024-02-29T12+05", ref_time).unwrap();

        assert_eq!(parsed.with_timezone(&Utc).timestamp(), 1_709_190_000);
        assert_eq!(
            parsed.with_timezone(&Utc).time(),
            NaiveTime::from_hms_opt(7, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_parse_gnu_iso_datetime_with_lowercase_t_separator() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("2024-02-29t12:34:56", ref_time).unwrap();

        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
        );
        assert_eq!(parsed.time(), NaiveTime::from_hms_opt(12, 34, 56).unwrap());
    }

    #[test]
    fn test_parse_gnu_hyphen_date_expands_two_digit_year() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, expected) in [
            ("24-2-29", NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()),
            ("68-1-2", NaiveDate::from_ymd_opt(2068, 1, 2).unwrap()),
            ("69-1-2", NaiveDate::from_ymd_opt(1969, 1, 2).unwrap()),
            (
                "24-2-29T12:34",
                NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
            ),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.date_naive(), expected, "input {input}");
            let expected_time = if input.contains('T') {
                NaiveTime::from_hms_opt(12, 34, 0).unwrap()
            } else {
                NaiveTime::MIN
            };
            assert_eq!(parsed.time(), expected_time, "input {input}");
        }
    }

    #[test]
    fn test_rejects_timezone_less_iso_hour() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in ["2024-02-29T1", "2024-02-29T12"] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} should fail"
            );
        }
    }

    #[test]
    fn test_rejects_iso_date_followed_by_standalone_t_timezone() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in ["2024-02-29 T", "2024-02-29 t"] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input} should fail"
            );
        }
    }

    #[test]
    fn test_parses_time_only_utc_designator() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for (input, expected) in [
            ("12Z", NaiveTime::from_hms_opt(12, 0, 0).unwrap()),
            ("001Z", NaiveTime::from_hms_opt(0, 1, 0).unwrap()),
            (
                "12:34:56.123Z",
                NaiveTime::from_hms_nano_opt(12, 34, 56, 123_000_000).unwrap(),
            ),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.with_timezone(&Utc).date_naive(),
                ref_time.date_naive()
            );
            assert_eq!(parsed.with_timezone(&Utc).time(), expected, "input {input}");
        }
    }

    #[test]
    fn test_parse_gnu_compact_time_with_date_in_either_order() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in ["2024-02-29 1234", "1234 2024-02-29"] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.date_naive(),
                NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
                "input {input}"
            );
            assert_eq!(
                parsed.time(),
                NaiveTime::from_hms_opt(12, 34, 0).unwrap(),
                "input {input}"
            );
        }
    }

    #[test]
    fn test_parse_gnu_explicit_time_before_iso_date() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("12:34 2024-02-29", ref_time).unwrap();

        assert_eq!(
            parsed.naive_local(),
            NaiveDate::from_ymd_opt(2024, 2, 29)
                .unwrap()
                .and_hms_opt(12, 34, 0)
                .unwrap()
        );
    }

    #[test]
    fn test_parse_gnu_time_only_with_fractional_seconds() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("12:34:56.123456789", ref_time).unwrap();

        assert_eq!(parsed.date_naive(), ref_time.date_naive());
        assert_eq!(
            parsed.time(),
            NaiveTime::from_hms_nano_opt(12, 34, 56, 123_456_789).unwrap()
        );
    }

    #[test]
    fn test_parse_gnu_bare_month_uses_calendar_arithmetic() {
        let ref_time = Local.with_ymd_and_hms(2024, 1, 31, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("month", ref_time).unwrap();

        assert_eq!(
            parsed.date_naive(),
            NaiveDate::from_ymd_opt(2024, 3, 2).unwrap()
        );
        assert_eq!(parsed.time(), NaiveTime::from_hms_opt(12, 0, 0).unwrap());
    }

    #[test]
    fn test_parse_gnu_relative_minute_and_second_aliases() {
        let ref_time = Local.timestamp_opt(1_000_000, 0).unwrap();
        let base = parse_datetime_gnu_compat("1970-01-01", ref_time).unwrap();

        for (input, offset_seconds) in [
            ("1970-01-01 1 min", 60),
            ("1970-01-01 1 mins", 60),
            ("1970-01-01 1 second", 1),
            ("1970-01-01 1 secs", 1),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.timestamp(),
                base.timestamp() + offset_seconds,
                "input {input}"
            );
        }
    }

    #[test]
    fn test_parse_gnu_military_timezone_with_relative_date() {
        let ref_time = Local.timestamp_opt(1_000_000, 0).unwrap();

        for input in ["a day ago", "day ago a"] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(
                parsed.timestamp(),
                ref_time.timestamp() - 90_000,
                "input {input}"
            );
        }
    }

    #[test]
    fn test_parse_gnu_relative_unit_with_weekday() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap(); // Thursday

        for (input, expected_date, expected_time) in [
            (
                "second monday",
                NaiveDate::from_ymd_opt(2025, 7, 28).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 1).unwrap(),
            ),
            (
                "day monday",
                NaiveDate::from_ymd_opt(2025, 7, 29).unwrap(),
                NaiveTime::MIN,
            ),
            (
                "monday 1 second",
                NaiveDate::from_ymd_opt(2025, 7, 28).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 1).unwrap(),
            ),
        ] {
            let parsed = parse_datetime_gnu_compat(input, ref_time).unwrap();
            assert_eq!(parsed.date_naive(), expected_date, "input {input}");
            assert_eq!(parsed.time(), expected_time, "input {input}");
        }
    }

    #[test]
    fn test_rejects_leap_second_input() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

        for input in [
            "2016-12-31 23:59:60 UTC",
            "2016-12-31 3:9:60 UTC",
            "2016-12-31T23:59:60+00:00",
            "Dec 31 2016 23:59:60 UTC",
        ] {
            assert!(
                parse_datetime_gnu_compat(input, ref_time).is_err(),
                "input {input}"
            );
        }
    }

    #[test]
    fn test_parse_comma_fractional_seconds() {
        let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();
        let parsed = parse_datetime_gnu_compat("2024-01-01 12:00:00,25 UTC", ref_time).unwrap();

        assert_eq!(parsed.timestamp(), 1_704_110_400);
        assert_eq!(parsed.timestamp_subsec_nanos(), 250_000_000);
    }

    #[test]
    fn test_normalizes_all_comma_fractional_seconds_in_one_pass() {
        assert_eq!(
            normalize_comma_fractional_seconds("1:2:3,4 5:6:7,8"),
            Some("1:2:3.4 5:6:7.8".to_string())
        );
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
    fn test_parse_double_dash_as_reference_date_midnight() {
        let reference = Local.with_ymd_and_hms(2024, 2, 29, 12, 34, 56).unwrap();

        let parsed = parse_datetime_gnu_compat("--", reference).unwrap();

        assert_eq!(parsed.date_naive(), reference.date_naive());
        assert_eq!(parsed.time(), NaiveTime::MIN);
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

    #[test]
    fn test_two_digit_year_expansion_path() {
        let value = NaiveDate::parse_from_str("01/01/00", "%m/%d/%y").unwrap();
        assert_eq!(value.year(), 2000);
        assert_eq!(
            expand_year_for_format(value, "%m/%d/%y").unwrap().year(),
            2000
        );

        let reference = Local.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(
            parse_datetime_gnu_compat("01/01/00", reference)
                .unwrap()
                .year(),
            2000
        );
    }
}
