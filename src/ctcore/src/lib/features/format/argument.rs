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

use crate::{
    ct_error::set_exit_code,
    features::format::num_parser::{ParseError, ParsedNumber},
    quoting_style::{escape_name, Quotes, QuotingStyle},
    show_error, show_warning,
};
use os_display::Quotable;
use std::ffi::OsStr;

/// An argument for formatting
///
/// Each of these variants is only accepted by their respective directives. For
/// example, [`FormatArgument::Char`] requires a `%c` directive.
///
/// The [`FormatArgument::Unparsed`] variant contains a string that can be
/// parsed into other types. This is used by the `printf` utility.
#[derive(Clone, Debug)]
pub enum FormatArgument {
    Char(char),
    String(String),
    UnsignedInt(u64),
    SignedInt(i64),
    Float(f64),
    /// Special argument that gets coerced into the other variants
    Unparsed(String),
}

pub trait ArgumentIter<'a>: Iterator<Item = &'a FormatArgument> {
    fn get_char(&mut self) -> u8;
    fn get_i64(&mut self) -> i64;
    fn get_u64(&mut self) -> u64;
    fn get_f64(&mut self) -> f64;
    fn get_str(&mut self) -> &'a str;
}

impl<'a, T: Iterator<Item = &'a FormatArgument>> ArgumentIter<'a> for T {
    fn get_char(&mut self) -> u8 {
        if let Some(next) = self.next() {
            match next {
                FormatArgument::Unparsed(s) => {
                    // s.bytes().next().unwrap_or(b'\0')
                    let v = s.bytes().next();
                    v.unwrap_or(b'\0')
                }
                FormatArgument::Char(c) => *c as u8,
                _ => b'\0',
            }
        } else {
            b'\0'
        }
    }

    fn get_u64(&mut self) -> u64 {
        if let Some(next) = self.next() {
            match next {
                FormatArgument::Unparsed(s) => {
                    let v = ParsedNumber::parse_u64(s);
                    extract_value(v, s)
                }
                FormatArgument::UnsignedInt(n) => *n,
                _ => 0,
            }
        } else {
            0
        }
    }

    fn get_i64(&mut self) -> i64 {
        let result = self.next();
        if let Some(next) = result {
            match next {
                FormatArgument::Unparsed(s) => {
                    let v = ParsedNumber::parse_i64(s);
                    extract_value(v, s)
                }
                FormatArgument::SignedInt(n) => *n,
                _ => 0,
            }
        } else {
            0
        }
    }

    fn get_f64(&mut self) -> f64 {
        let result = self.next();
        if let Some(next) = result {
            match next {
                FormatArgument::Unparsed(s) => {
                    let v = ParsedNumber::parse_f64(s);
                    extract_value(v, s)
                }
                FormatArgument::Float(n) => *n,
                _ => 0.0,
            }
        } else {
            0.0
        }
    }

    fn get_str(&mut self) -> &'a str {
        // match self.next() {
        //     Some(FormatArgument::Unparsed(s) | FormatArgument::String(s)) => s,
        //     _ => "",
        // }
        let result = self.next();
        if let Some(FormatArgument::Unparsed(s) | FormatArgument::String(s)) = result {
            s
        } else {
            ""
        }
    }
}

// 该函数接收两个通用参数： T 和 ParseError<'_, T>。该函数用于从解析结果中提取值，并处理可能出现的解析错误。
// 函数首先检查解析结果 (p) 是否为 OK，即解析是否成功。如果是，则返回解析后的值 (v)。
// 如果解析结果为 Err，表示解析过程中出现错误，函数会将退出代码设为 1（表示出现错误），然后继续处理错误。
fn extract_value<T: Default>(p: Result<T, ParseError<'_, T>>, input: &str) -> T {
    match p {
        Ok(v) => v,
        Err(e) => {
            set_exit_code(1);
            let input = escape_name(
                OsStr::new(input),
                &QuotingStyle::C {
                    quotes: Quotes::None,
                },
            );
            match e {
                ParseError::CtOverflow => {
                    show_error!("{}: Numerical result out of range", input.quote());
                    Default::default()
                }
                ParseError::CtNotNumeric => {
                    show_error!("{}: expected a numeric value", input.quote());
                    Default::default()
                }
                ParseError::CtPartialMatch(v, rest) => {
                    if input.starts_with('\'') {
                        show_warning!(
                            "{}: character(s) following character constant have been ignored",
                            &rest,
                        );
                    } else {
                        show_error!("{}: value not completely converted", input.quote());
                    }
                    v
                }
            }

            // if let ParseError::CtOverflow = e {
            //     show_error!("{}: Numerical result out of range", input.quote());
            //     Default::default()
            // } else if let ParseError::CtNotNumeric = e {
            //     show_error!("{}: expected a numeric value", input.quote());
            //     Default::default()
            // } else if let ParseError::CtPartialMatch(v, rest) = e {
            //     if input.starts_with('\'') {
            //         show_warning!(
            //             "{}: character(s) following character constant have been ignored",
            //             &rest,
            //         );
            //     } else {
            //         show_error!("{}: value not completely converted", input.quote());
            //     }
            //     v
            // }
        }
    }
}

