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

// spell-checker:ignore (vars) NANOS numstr
//! Parsing a duration from a string.
//!
//! Use the [`from_str`] function to parse a [`Duration`] from a string.

use std::time::Duration;

use crate::ct_display::Quotable;

/// Parse a duration from a string.
///
/// The string may contain only a number, like "123" or "4.5", or it
/// may contain a number with a unit specifier, like "123s" meaning
/// one hundred twenty three seconds or "4.5d" meaning four and a half
/// days. If no unit is specified, the unit is assumed to be seconds.
///
/// The only allowed suffixes are
///
/// * "s" for seconds,
/// * "m" for minutes,
/// * "h" for hours,
/// * "d" for days.
///
/// This function uses [`Duration::saturating_mul`] to compute the
/// number of seconds, so it does not overflow. If overflow would have
/// occurred, [`Duration::MAX`] is returned instead.
///
/// # Errors
///
/// This function returns an error if the input string is empty, the
/// input is not a valid number, or the unit specifier is invalid or
/// unknown.
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
/// use ctcore::ct_parse_time::from_str;
/// assert_eq!(from_str("123"), Ok(Duration::from_secs(123)));
/// assert_eq!(from_str("2d"), Ok(Duration::from_secs(60 * 60 * 24 * 2)));
/// ```
pub fn from_str(string: &str) -> Result<Duration, String> {
    // let len = string.len();
    // if len == 0 {
    //     return Err("empty string".to_owned());
    // }
    // let slice = match string.get(..len - 1) {
    //     Some(s) => s,
    //     None => return Err(format!("invalid time interval {}", string.quote())),
    // };
    // let (numstr, times) = match string.chars().next_back().unwrap() {
    //     's' => (slice, 1),
    //     'm' => (slice, 60),
    //     'h' => (slice, 60 * 60),
    //     'd' => (slice, 60 * 60 * 24),
    //     val if !val.is_alphabetic() => (string, 1),
    //     _ => {
    //         if string == "inf" || string == "infinity" {
    //             ("inf", 1)
    //         } else {
    //             return Err(format!("invalid time interval {}", string.quote()));
    //         }
    //     }
    // };
    // let num = numstr
    //     .parse::<f64>()
    //     .map_err(|e| format!("invalid time interval {}: {}", string.quote(), e))?;
    //
    // if num < 0. {
    //     return Err(format!("invalid time interval {}", string.quote()));
    // }
    //
    // const NANOS_PER_SEC: u32 = 1_000_000_000;
    // let whole_secs = num.trunc();
    // let nanos = (num.fract() * (NANOS_PER_SEC as f64)).trunc();
    // let duration = Duration::new(whole_secs as u64, nanos as u32);
    // Ok(duration.saturating_mul(times))

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

