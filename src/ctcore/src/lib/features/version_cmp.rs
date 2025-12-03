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
use std::cmp::Ordering;

/// Compares the non-digit parts of a version.
/// Special cases: ~ are before everything else, even ends ("a~" < "a")
/// Letters are before non-letters
fn version_non_digit_cmp(a: &str, b: &str) -> Ordering {
    let mut a_chars = a.chars();
    let mut b_chars = b.chars();
    loop {
        match (a_chars.next(), b_chars.next()) {
            (Some(c1), Some(c2)) if c1 == c2 => {}
            (None, None) => return Ordering::Equal,
            (_, Some('~')) => return Ordering::Greater,
            (Some('~'), _) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c1), Some(c2)) if c1.is_ascii_alphabetic() && !c2.is_ascii_alphabetic() => {
                return Ordering::Less
            }
            (Some(c1), Some(c2)) if !c1.is_ascii_alphabetic() && c2.is_ascii_alphabetic() => {
                return Ordering::Greater
            }
            (Some(c1), Some(c2)) => return c1.cmp(&c2),
        }
    }
}

/// Remove file endings matching the regex (\.[A-Za-z~][A-Za-z0-9~]*)*$
fn remove_file_ending(a: &str) -> &str {
    let mut ending_start = None;
    let mut prev_was_dot = false;
    for (idx, char) in a.char_indices() {
        if char == '.' {
            if ending_start.is_none() || prev_was_dot {
                ending_start = Some(idx);
            }
            prev_was_dot = true;
        } else if prev_was_dot {
            prev_was_dot = false;
            if !char.is_ascii_alphabetic() && char != '~' {
                ending_start = None;
            }
        } else if !char.is_ascii_alphanumeric() && char != '~' {
            ending_start = None;
        }
    }
    if prev_was_dot {
        ending_start = None;
    }
    if let Some(ending_start) = ending_start {
        &a[..ending_start]
    } else {
        a
    }
}

pub fn version_cmp(mut a: &str, mut b: &str) -> Ordering {
    let str_cmp = a.cmp(b);
    if str_cmp == Ordering::Equal {
        return str_cmp;
    }

    // Special cases:
    // 1. Empty strings
    match (a.is_empty(), b.is_empty()) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (true, true) => unreachable!(),
        (false, false) => {}
    }
    // 2. Dots
    match (a == ".", b == ".") {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (true, true) => unreachable!(),
        (false, false) => {}
    }
    // 3. Two Dots
    match (a == "..", b == "..") {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (true, true) => unreachable!(),
        (false, false) => {}
    }
    // 4. Strings starting with a dot
    match (a.starts_with('.'), b.starts_with('.')) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (true, true) => {
            // Strip the leading dot for later comparisons
            a = &a[1..];
            b = &b[1..];
        }
        _ => {}
    }

    // Try to strip file extensions
    let (mut a, mut b) = match (remove_file_ending(a), remove_file_ending(b)) {
        (a_stripped, b_stripped) if a_stripped == b_stripped => {
            // If both would be the same after stripping file extensions, don't strip them.
            (a, b)
        }
        stripped => stripped,
    };

    // 1. Compare leading non-numerical part
    // 2. Compare leading numerical part
    // 3. Repeat
    while !a.is_empty() || !b.is_empty() {
        let a_numerical_start = a.find(|c: char| c.is_ascii_digit()).unwrap_or(a.len());
        let b_numerical_start = b.find(|c: char| c.is_ascii_digit()).unwrap_or(b.len());

        let a_str = &a[..a_numerical_start];
        let b_str = &b[..b_numerical_start];

        match version_non_digit_cmp(a_str, b_str) {
            Ordering::Equal => {}
            ord => return ord,
        }

        a = &a[a_numerical_start..];
        b = &b[a_numerical_start..];

        let a_numerical_end = a.find(|c: char| !c.is_ascii_digit()).unwrap_or(a.len());
        let b_numerical_end = b.find(|c: char| !c.is_ascii_digit()).unwrap_or(b.len());

        let a_str = a[..a_numerical_end].trim_start_matches('0');
        let b_str = b[..b_numerical_end].trim_start_matches('0');

        match a_str.len().cmp(&b_str.len()) {
            Ordering::Equal => {}
            ord => return ord,
        }

        match a_str.cmp(b_str) {
            Ordering::Equal => {}
            ord => return ord,
        }

        a = &a[a_numerical_end..];
        b = &b[b_numerical_end..];
    }

    Ordering::Equal
}

