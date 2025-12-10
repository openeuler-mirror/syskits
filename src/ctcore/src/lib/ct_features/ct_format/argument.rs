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
    ct_error::set_ct_exit_code,
    ct_features::ct_format::num_parser::{ParseError, ParsedNumber},
    ct_quoting_style::{escape_name, CtQuotes, CtQuotingStyle},
    ct_show_error, ct_show_warning,
};
use os_display::Quotable;
use std::ffi::OsStr;

/// 格式化参数
///
/// 这些变体各自仅被其相应的指令接受。例如，FormatArgument::Char 需要一个 %c 指令。
///
/// FormatArgument::Unparsed 变体包含一个可以解析为其他类型的字符串。
/// 这是由 printf 工具使用的。
///
///
#[derive(Clone, Debug)]
pub enum FormatArgument {
    Char(char),
    String(String),
    UnsignedInt(u64),
    SignedInt(i64),
    Float(f64),
    /// 特殊参数，会被强制转换为其他变体
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
            set_ct_exit_code(1);
            let input = escape_name(
                OsStr::new(input),
                &CtQuotingStyle::C {
                    quotes: CtQuotes::None,
                },
            );
            match e {
                ParseError::CtOverflow => {
                    ct_show_error!("{}: Numerical result out of range", input.quote());
                    Default::default()
                }
                ParseError::CtNotNumeric => {
                    ct_show_error!("{}: expected a numeric value", input.quote());
                    Default::default()
                }
                ParseError::CtPartialMatch(v, rest) => {
                    if input.starts_with('\'') {
                        ct_show_warning!(
                            "{}: character(s) following character constant have been ignored",
                            &rest,
                        );
                    } else {
                        ct_show_error!("{}: value not completely converted", input.quote());
                    }
                    v
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(test)]
    mod tests {
        use super::*;

        struct MockArgumentIter<'a> {
            args: &'a [FormatArgument],
            index: usize,
        }

        impl<'a> MockArgumentIter<'a> {
            fn new(args: &'a [FormatArgument]) -> Self {
                MockArgumentIter { args, index: 0 }
            }
        }

        impl<'a> Iterator for MockArgumentIter<'a> {
            type Item = &'a FormatArgument;

            fn next(&mut self) -> Option<Self::Item> {
                if self.index < self.args.len() {
                    let arg = &self.args[self.index];
                    self.index += 1;
                    Some(arg)
                } else {
                    None
                }
            }
        }

        #[test]
        fn test_argument_iter_get_char() {
            let args = vec![
                FormatArgument::Char('A'),
                FormatArgument::Unparsed("BCD".to_string()),
                FormatArgument::String("EFG".to_string()),
                FormatArgument::SignedInt(42),
            ];
            let mut iter = MockArgumentIter::new(&args);

            assert_eq!(iter.get_char(), b'A');
            assert_eq!(iter.get_char(), b'B');
            assert_eq!(iter.get_char(), 0); // 对于String，只应考虑第一个字符
            assert_eq!(iter.get_char(), 0); // 对于 SignedInt 类型，应默认返回 '\0'
        }

        #[test]
        fn test_argument_iter_get_i64() {
            let args = vec![
                FormatArgument::SignedInt(-42),
                FormatArgument::Unparsed("123".to_string()),
                FormatArgument::UnsignedInt(456),
                FormatArgument::String("789".to_string()),
            ];
            let mut iter = MockArgumentIter::new(&args);

            assert_eq!(iter.get_i64(), -42);
            assert_eq!(iter.get_i64(), 123);
            assert_eq!(iter.get_i64(), 0); // 对于UnsignedInt，它应该自动转换为i64
            assert_eq!(iter.get_i64(), 0); // 对于 String 类型，应默认返回 0
        }

        #[test]
        fn test_argument_iter_get_u64() {
            let args = vec![
                FormatArgument::UnsignedInt(123),
                FormatArgument::Unparsed("456".to_string()),
                FormatArgument::SignedInt(-789),
                FormatArgument::String("101112".to_string()),
            ];
            let mut iter = MockArgumentIter::new(&args);

            assert_eq!(iter.get_u64(), 123);
            assert_eq!(iter.get_u64(), 456); //对于 Unparsed 类型，应将字符串解析为 u64
            assert_eq!(iter.get_u64(), 0); //  对于 SignedInt 类型，应默认返回 0。
            assert_eq!(iter.get_u64(), 0); // 对于 String 类型，应默认返回 0。
        }

        #[test]
        fn test_argument_iter_get_f64() {
            let args = vec![
                FormatArgument::Float(3.14),
                FormatArgument::Unparsed("2.718".to_string()),
                FormatArgument::SignedInt(-42),
                FormatArgument::String("1.618".to_string()),
            ];
            let mut iter = MockArgumentIter::new(&args);

            assert_eq!(iter.get_f64(), 3.14);
            assert_eq!(iter.get_f64(), 2.718); //对于 Unparsed 类型，应将字符串解析为 u64
            assert_eq!(iter.get_f64(), 0.0); // 对于 SignedInt 类型，应默认返回 0。
            assert_eq!(iter.get_f64(), 0.0); // 对于 String 类型，应默认返回 0。
        }

        #[test]
        fn test_argument_iter_get_str() {
            let args = vec![
                FormatArgument::String("abc".to_string()),
                FormatArgument::Unparsed("def".to_string()),
                FormatArgument::Char('g'),
                FormatArgument::SignedInt(-42),
            ];
            let mut iter = MockArgumentIter::new(&args);

            assert_eq!(iter.get_str(), "abc");
            assert_eq!(iter.get_str(), "def");
            assert_eq!(iter.get_str(), ""); //对于 Char类型，应该返回一个空的字符串
            assert_eq!(iter.get_str(), ""); //对于 SignedInt 类型，应该返回一个空的字符串
        }
    }

    #[test]
    fn test_extract_value_ok() {
        let result: Result<u32, ParseError<u32>> = Ok(42);
        assert_eq!(extract_value(result, "input"), 42);
    }

    #[test]
    fn test_extract_value_overflow() {
        let result: Result<u32, ParseError<u32>> = Err(ParseError::CtOverflow);
        assert_eq!(extract_value(result, "input"), 0); // 默认值
    }

    #[test]
    fn test_extract_value_not_numeric() {
        let result: Result<u32, ParseError<u32>> = Err(ParseError::CtNotNumeric);
        assert_eq!(extract_value(result, "input"), 0); // 默认值
    }

    #[test]
    fn test_extract_value_partial_match() {
        let result: Result<u32, ParseError<u32>> = Err(ParseError::CtPartialMatch(5, "rest"));
        assert_eq!(extract_value(result, "input"), 5);
    }
    #[test]
    fn test_extract_value_unexpected_error() {
        let result: Result<u32, ParseError<u32>> = Err(ParseError::CtOverflow);

        assert_eq!(extract_value(result, "input"), 0);
    }
}
