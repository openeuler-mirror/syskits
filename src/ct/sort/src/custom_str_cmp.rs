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

//! 自定义字符串比较。
//!
//! 目标是在不转换字符串的情况下（即不分配新字符串）比较字符串。)

use std::cmp::Ordering;

fn custom_filter_char(
    c: char,
    is_ignore_non_printing: bool,
    is_ignore_non_dictionary: bool,
) -> bool {
    if is_ignore_non_dictionary && !(c.is_ascii_alphanumeric() || c.is_ascii_whitespace()) {
        return false;
    }
    if is_ignore_non_printing && (c.is_ascii_control() || !c.is_ascii()) {
        return false;
    }
    true
}

fn custom_cmp_chars(a: char, b: char, is_ignore_case: bool) -> Ordering {
    match is_ignore_case {
        true => a.to_ascii_uppercase().cmp(&b.to_ascii_uppercase()),
        false => a.cmp(&b),
    }
}

pub fn custom_cmp_str(
    a: &str,
    b: &str,
    is_ignore_non_printing: bool,
    is_ignore_non_dictionary: bool,
    ignore_case: bool,
) -> Ordering {
    if !(ignore_case || is_ignore_non_dictionary || is_ignore_non_printing) {
        // 没有自定义设置。返回默认的 strcmp，速度更快。
        return a.cmp(b);
    }
    let mut a_chars = a
        .chars()
        .filter(|&c| custom_filter_char(c, is_ignore_non_printing, is_ignore_non_dictionary));
    let mut b_chars = b
        .chars()
        .filter(|&c| custom_filter_char(c, is_ignore_non_printing, is_ignore_non_dictionary));
    loop {
        let a_char = a_chars.next();
        let b_char = b_chars.next();
        match (a_char, b_char) {
            (None, None) => return Ordering::Equal,
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (Some(a_char), Some(b_char)) => {
                let ordering = custom_cmp_chars(a_char, b_char, ignore_case);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
        }
    }
}

