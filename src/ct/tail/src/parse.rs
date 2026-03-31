/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use std::ffi::OsString;

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub struct TailObsoleteArgs {
    pub num: u64,
    pub plus: bool,
    pub lines: bool,
    pub follow: bool,
}

impl Default for TailObsoleteArgs {
    fn default() -> Self {
        Self {
            num: 10,
            plus: false,
            lines: true,
            follow: false,
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum TailParseError {
    OutOfRange,
    Overflow,
    Context,
    InvalidEncoding,
}
/// Parses obsolete syntax
/// tail -\[NUM\]\[bcl\]\[f\] and tail +\[NUM\]\[bcl\]\[f\] // spell-checker:disable-line
pub fn tail_parse_obsolete(src: &OsString) -> Option<Result<TailObsoleteArgs, TailParseError>> {
    let mut rest = match src.to_str() {
        Some(src) => src,
        None => return Some(Err(TailParseError::InvalidEncoding)),
    };
    let sign = if let Some(r) = rest.strip_prefix('-') {
        rest = r;
        '-'
    } else if let Some(r) = rest.strip_prefix('+') {
        rest = r;
        '+'
    } else {
        return None;
    };

    let end_num = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let has_num = !rest[..end_num].is_empty();
    let num: u64 = if has_num {
        if let Ok(num) = rest[..end_num].parse() {
            num
        } else {
            return Some(Err(TailParseError::OutOfRange));
        }
    } else {
        10
    };
    rest = &rest[end_num..];

    let mode = if let Some(r) = rest.strip_prefix('l') {
        rest = r;
        'l'
    } else if let Some(r) = rest.strip_prefix('c') {
        rest = r;
        'c'
    } else if let Some(r) = rest.strip_prefix('b') {
        rest = r;
        'b'
    } else {
        'l'
    };

    let follow = rest.contains('f');
    if !rest.chars().all(|f| f == 'f') {
        // GNU allows an arbitrary amount of following fs, but nothing else
        if sign == '-' && has_num {
            return Some(Err(TailParseError::Context));
        }
        return None;
    }

    let multiplier = if mode == 'b' { 512 } else { 1 };
    let num = match num.checked_mul(multiplier) {
        Some(n) => n,
        None => return Some(Err(TailParseError::Overflow)),
    };

    Some(Ok(TailObsoleteArgs {
        num,
        plus: sign == '+',
        lines: mode == 'l',
        follow,
    }))
}

