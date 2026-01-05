/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use ctcore::ct_parse_size::{parse_size_u64, ParseSizeError};
use std::ffi::OsString;

#[derive(PartialEq, Eq, Debug)]
pub enum ParseError {
    Syntax,
    Overflow,
}

/// Parses obsolete syntax
/// head -NUM\[kmzv\] // spell-checker:disable-line
pub fn parse_obsolete(src: &str) -> Option<Result<impl Iterator<Item = OsString>, ParseError>> {
    let mut chars = src.char_indices();
    if let Some((_, '-')) = chars.next() {
        let mut num_end = 0usize;
        let mut has_num = false;
        let mut last_char = 0 as char;
        for (n, c) in &mut chars {
            if c.is_ascii_digit() {
                has_num = true;
                num_end = n;
            } else {
                last_char = c;
                break;
            }
        }
        if has_num {
            process_num_block(&src[1..=num_end], last_char, &mut chars)
        } else {
            None
        }
    } else {
        None
    }
}

/// Processes the numeric block of the input string to generate the appropriate options.
fn process_num_block(
    src: &str,
    last_char: char,
    chars: &mut std::str::CharIndices,
) -> Option<Result<impl Iterator<Item = OsString>, ParseError>> {
    match src.parse::<usize>() {
        Ok(num) => {
            let mut quiet = false;
            let mut verbose = false;
            let mut zero_terminated = false;
            let mut multiplier = None;
            let mut c = last_char;
            loop {
                // note that here, we only match lower case 'k', 'c', and 'm'
                match c {
                    // we want to preserve order
                    // this also saves us 1 heap allocation
                    'q' => {
                        quiet = true;
                        verbose = false;
                    }
                    'v' => {
                        verbose = true;
                        quiet = false;
                    }
                    'z' => zero_terminated = true,
                    'c' => multiplier = Some(1),
                    'b' => multiplier = Some(512),
                    'k' => multiplier = Some(1024),
                    'm' => multiplier = Some(1024 * 1024),
                    '\0' => {}
                    _ => return Some(Err(ParseError::Syntax)),
                }
                if let Some((_, next)) = chars.next() {
                    c = next;
                } else {
                    break;
                }
            }
            let mut options = Vec::new();
            if quiet {
                options.push(OsString::from("-q"));
            }
            if verbose {
                options.push(OsString::from("-v"));
            }
            if zero_terminated {
                options.push(OsString::from("-z"));
            }
            if let Some(n) = multiplier {
                options.push(OsString::from("-c"));
                let num = match num.checked_mul(n) {
                    Some(n) => n,
                    None => return Some(Err(ParseError::Overflow)),
                };
                options.push(OsString::from(format!("{num}")));
            } else {
                options.push(OsString::from("-n"));
                options.push(OsString::from(format!("{num}")));
            }
            Some(Ok(options.into_iter()))
        }
        Err(_) => Some(Err(ParseError::Overflow)),
    }
}

/// Parses an -c or -n argument,
/// the bool specifies whether to read from the end
pub fn parse_num(src: &str) -> Result<(u64, bool), ParseSizeError> {
    let mut size_string = src.trim();
    let mut all_but_last = false;

    if let Some(c) = size_string.chars().next() {
        if c == '+' || c == '-' {
            // head: '+' is not documented (8.32 man pages)
            size_string = &size_string[1..];
            if c == '-' {
                all_but_last = true;
            }
        }
    } else {
        return Err(ParseSizeError::ParseFailure(src.to_string()));
    }

    // remove leading zeros so that size is interpreted as decimal, not octal
    let trimmed_string = size_string.trim_start_matches('0');
    if trimmed_string.is_empty() {
        Ok((0, all_but_last))
    } else {
        parse_size_u64(trimmed_string).map(|n| (n, all_but_last))
    }
}

