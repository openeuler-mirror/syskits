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

use super::long_double::{
    ExtendedBigDecimal, LongDoubleParseError, long_double_from_f64, parse_long_double,
};
use crate::{
    ct_error::set_ct_exit_code,
    ct_features::ct_format::num_parser::{ParseError, ParsedNumber},
    ct_show_warning,
};
use std::ffi::{CStr, OsStr};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;

use super::spec::classify_printf_locale_sequence;

unsafe extern "C" {
    fn mbrtowc(
        wide: *mut crate::libc::wchar_t,
        bytes: *const crate::libc::c_char,
        length: usize,
        state: *mut crate::libc::mbstate_t,
    ) -> usize;
}

/// 格式化参数
///
/// 这些变体各自仅被其相应的指令接受。例如，FormatArgument::Char 需要一个 %c 指令。
///
/// FormatArgument::Unparsed 变体包含一个可以解析为其他类型的字符串。
/// 这是由 printf 工具使用的。
#[derive(Clone, Debug)]
pub enum FormatArgument {
    Char(char),
    String(String),
    UnsignedInt(u64),
    SignedInt(i64),
    Float(f64),
    /// 特殊参数，会被强制转换为其他变体
    Unparsed(String),
    /// Linux命令行中的原始字节参数，由printf用于保留非UTF-8输入。
    Bytes(Vec<u8>),
}

/// 支持随机索引访问的参数游标
pub struct ArgCursor<'a> {
    args: &'a [FormatArgument],
    seq_index: usize,
    max_accessed: usize,
}

impl<'a> ArgCursor<'a> {
    pub fn new(args: &'a [FormatArgument]) -> Self {
        Self {
            args,
            seq_index: 0,
            max_accessed: 0,
        }
    }

    /// 根据是否有明确索引来提取参数，并记录最大读取位置
    fn fetch(&mut self, explicit_index: Option<usize>) -> Option<&'a FormatArgument> {
        let idx = match explicit_index {
            Some(i) => {
                if i > 0 {
                    i - 1
                } else {
                    return None;
                }
            }
            None => {
                let i = self.seq_index;
                self.seq_index += 1;
                i
            }
        };

        if idx + 1 > self.max_accessed {
            self.max_accessed = idx + 1;
        }

        self.args.get(idx)
    }

    pub fn get_char(&mut self, idx: Option<usize>) -> u8 {
        if let Some(arg) = self.fetch(idx) {
            match arg {
                FormatArgument::Unparsed(s) => {
                    let v = s.bytes().next();
                    v.unwrap_or(b'\0')
                }
                FormatArgument::Bytes(bytes) => bytes.first().copied().unwrap_or(b'\0'),
                FormatArgument::Char(c) => *c as u8,
                _ => b'\0',
            }
        } else {
            b'\0'
        }
    }

    pub fn get_u64(&mut self, idx: Option<usize>) -> u64 {
        if let Some(arg) = self.fetch(idx) {
            match arg {
                FormatArgument::Unparsed(s) => {
                    let v = ParsedNumber::parse_u64(s);
                    extract_value_with_overflow(v, s, u64::MAX)
                }
                FormatArgument::Bytes(bytes) => parse_bytes_u64(bytes),
                FormatArgument::UnsignedInt(n) => *n,
                _ => 0,
            }
        } else {
            0
        }
    }

    pub fn get_i64(&mut self, idx: Option<usize>) -> i64 {
        self.fetch(idx).map(parse_argument_i64).unwrap_or(0)
    }

    pub(crate) fn get_i64_with_source(&mut self, idx: Option<usize>) -> (i64, Vec<u8>) {
        let Some(arg) = self.fetch(idx) else {
            return (0, Vec::new());
        };

        (parse_argument_i64(arg), argument_source_bytes(arg))
    }

    pub fn get_f64(&mut self, idx: Option<usize>) -> f64 {
        if let Some(arg) = self.fetch(idx) {
            match arg {
                FormatArgument::Unparsed(s) => {
                    let v = ParsedNumber::parse_f64(s);
                    extract_value(v, s)
                }
                FormatArgument::Bytes(bytes) => parse_bytes_f64(bytes),
                FormatArgument::Float(n) => *n,
                _ => 0.0,
            }
        } else {
            0.0
        }
    }

    pub fn get_long_double(&mut self, idx: Option<usize>) -> ExtendedBigDecimal {
        if let Some(arg) = self.fetch(idx) {
            match arg {
                FormatArgument::Unparsed(s) => extract_long_double(parse_long_double(s), s),
                FormatArgument::Bytes(bytes) => parse_bytes_long_double(bytes),
                FormatArgument::Float(n) => long_double_from_f64(*n),
                _ => ExtendedBigDecimal::default(),
            }
        } else {
            ExtendedBigDecimal::default()
        }
    }

    pub fn get_str(&mut self, idx: Option<usize>) -> &'a str {
        match self.fetch(idx) {
            Some(FormatArgument::Unparsed(s) | FormatArgument::String(s)) => s,
            Some(FormatArgument::Bytes(bytes)) => std::str::from_utf8(bytes).unwrap_or(""),
            _ => "",
        }
    }

    pub fn get_bytes(&mut self, idx: Option<usize>) -> &'a [u8] {
        self.get_optional_bytes(idx).unwrap_or(b"")
    }

    pub fn get_optional_bytes(&mut self, idx: Option<usize>) -> Option<&'a [u8]> {
        match self.fetch(idx) {
            Some(FormatArgument::Unparsed(s) | FormatArgument::String(s)) => Some(s.as_bytes()),
            Some(FormatArgument::Bytes(bytes)) => Some(bytes),
            Some(_) => Some(b""),
            None => None,
        }
    }

    /// 返回当前批次中最多消耗了几个参数
    pub fn consumed_count(&self) -> usize {
        self.max_accessed
    }
}

fn parse_argument_i64(arg: &FormatArgument) -> i64 {
    match arg {
        FormatArgument::Unparsed(s) => {
            let v = ParsedNumber::parse_i64(s);
            extract_value_with_overflow(v, s, signed_overflow_value(s))
        }
        FormatArgument::Bytes(bytes) => parse_bytes_i64(bytes),
        FormatArgument::SignedInt(n) => *n,
        _ => 0,
    }
}

fn argument_source_bytes(arg: &FormatArgument) -> Vec<u8> {
    match arg {
        FormatArgument::Unparsed(s) | FormatArgument::String(s) => s.as_bytes().to_vec(),
        FormatArgument::Bytes(bytes) => bytes.clone(),
        FormatArgument::Char(c) => c.to_string().into_bytes(),
        FormatArgument::UnsignedInt(n) => n.to_string().into_bytes(),
        FormatArgument::SignedInt(n) => n.to_string().into_bytes(),
        FormatArgument::Float(n) => n.to_string().into_bytes(),
    }
}

fn parse_bytes_u64(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    if let Some(value) = parse_bytes_character_constant(bytes) {
        return value;
    }
    match std::str::from_utf8(bytes) {
        Ok(input) => extract_value_with_overflow(ParsedNumber::parse_u64(input), input, u64::MAX),
        Err(_) => invalid_numeric_bytes(bytes),
    }
}

fn parse_bytes_i64(bytes: &[u8]) -> i64 {
    if bytes.is_empty() {
        return 0;
    }
    if let Some(value) = parse_bytes_character_constant(bytes) {
        return value as i64;
    }
    match std::str::from_utf8(bytes) {
        Ok(input) => extract_value_with_overflow(
            ParsedNumber::parse_i64(input),
            input,
            signed_overflow_value(input),
        ),
        Err(_) => invalid_numeric_bytes(bytes),
    }
}

fn parse_bytes_f64(bytes: &[u8]) -> f64 {
    if let Some(value) = parse_bytes_character_constant(bytes) {
        return value as f64;
    }
    match std::str::from_utf8(bytes) {
        Ok(input) => extract_value(ParsedNumber::parse_f64(input), input),
        Err(_) => invalid_numeric_bytes(bytes),
    }
}

fn parse_bytes_long_double(bytes: &[u8]) -> ExtendedBigDecimal {
    if let Some(value) = parse_bytes_character_constant(bytes) {
        return ExtendedBigDecimal::from_u64(value);
    }
    match std::str::from_utf8(bytes) {
        Ok(input) => extract_long_double(parse_long_double(input), input),
        Err(_) => invalid_numeric_bytes(bytes),
    }
}

fn extract_long_double(
    parsed: Result<ExtendedBigDecimal, LongDoubleParseError<'_>>,
    input: &str,
) -> ExtendedBigDecimal {
    match parsed {
        Ok(value) => value,
        Err(error) => {
            let input_escaped = quote_numeric_argument(OsStr::new(input));
            match error {
                LongDoubleParseError::OutOfRange(value) => {
                    set_ct_exit_code(1);
                    show_numeric_error(&input_escaped, b"Numerical result out of range");
                    value
                }
                LongDoubleParseError::NotNumeric => {
                    set_ct_exit_code(1);
                    show_numeric_error(&input_escaped, b"expected a numeric value");
                    ExtendedBigDecimal::default()
                }
                LongDoubleParseError::PartialMatch(value, rest) => {
                    if input.starts_with('\'') || input.starts_with('"') {
                        if std::env::var_os("POSIXLY_CORRECT").is_none() {
                            ct_show_warning!(
                                "{}: character(s) following character constant have been ignored",
                                rest,
                            );
                        }
                    } else {
                        set_ct_exit_code(1);
                        show_numeric_error(&input_escaped, b"value not completely converted");
                    }
                    value
                }
            }
        }
    }
}

fn parse_bytes_character_constant(bytes: &[u8]) -> Option<u64> {
    let rest = bytes
        .strip_prefix(b"\'")
        .or_else(|| bytes.strip_prefix(b"\""))?;
    let first = *rest.first()?;
    let mut value = u64::from(first);
    let mut consumed = 1;

    if rest.len() > 1 {
        unsafe {
            let mut state: crate::libc::mbstate_t = std::mem::zeroed();
            let mut wide: crate::libc::wchar_t = 0;
            let length = mbrtowc(&mut wide, rest.as_ptr().cast(), rest.len(), &mut state);
            if length != usize::MAX && length != usize::MAX - 1 && length != 0 {
                value = wide as u64;
                consumed = length;
            }
        }
    }

    let trailing = &rest[consumed..];
    if !trailing.is_empty() && std::env::var_os("POSIXLY_CORRECT").is_none() {
        let warning = character_constant_warning(crate::ct_util_name(), trailing);
        let _ = std::io::stderr().lock().write_all(&warning);
    }
    Some(value)
}

fn character_constant_warning(utility_name: &str, trailing: &[u8]) -> Vec<u8> {
    let mut warning = Vec::with_capacity(utility_name.len() + trailing.len() + 80);
    warning.extend_from_slice(utility_name.as_bytes());
    warning.extend_from_slice(b": warning: ");
    warning.extend_from_slice(trailing);
    warning.extend_from_slice(b": character(s) following character constant have been ignored\n");
    warning
}

fn invalid_numeric_bytes<T: Default>(bytes: &[u8]) -> T {
    set_ct_exit_code(1);
    let escaped = quote_numeric_argument(OsStr::from_bytes(bytes));
    show_numeric_error(&escaped, b"expected a numeric value");
    T::default()
}

fn quote_numeric_argument(input: &OsStr) -> Vec<u8> {
    let (left_quote, right_quote) = numeric_locale_quote_marks();
    quote_numeric_bytes_with_classifier(
        input.as_bytes(),
        left_quote,
        right_quote,
        classify_printf_locale_sequence,
    )
}

fn quote_numeric_bytes_with_classifier<F>(
    input: &[u8],
    left_quote: &[u8],
    right_quote: &[u8],
    mut classify: F,
) -> Vec<u8>
where
    F: FnMut(&[u8]) -> (usize, bool),
{
    let mut quoted = Vec::with_capacity(input.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(left_quote);

    let mut index = 0;
    while index < input.len() {
        if !right_quote.is_empty() && input[index..].starts_with(right_quote) {
            quoted.push(b'\\');
            quoted.extend_from_slice(right_quote);
            index += right_quote.len();
            continue;
        }

        let byte = input[index];
        if byte.is_ascii() {
            match byte {
                b'\x07' => quoted.extend_from_slice(b"\\a"),
                b'\x08' => quoted.extend_from_slice(b"\\b"),
                b'\t' => quoted.extend_from_slice(b"\\t"),
                b'\n' => quoted.extend_from_slice(b"\\n"),
                b'\x0b' => quoted.extend_from_slice(b"\\v"),
                b'\x0c' => quoted.extend_from_slice(b"\\f"),
                b'\r' => quoted.extend_from_slice(b"\\r"),
                b'\\' => quoted.extend_from_slice(b"\\\\"),
                0x00..=0x1f | 0x7f => push_octal_byte_to_vec(&mut quoted, byte),
                _ => quoted.push(byte),
            }
            index += 1;
            continue;
        }

        let (length, printable) = classify(&input[index..]);
        let length = length.clamp(1, input.len() - index);
        if printable {
            quoted.extend_from_slice(&input[index..index + length]);
        } else {
            for byte in &input[index..index + length] {
                push_octal_byte_to_vec(&mut quoted, *byte);
            }
        }
        index += length;
    }

    quoted.extend_from_slice(right_quote);
    quoted
}

fn numeric_locale_quote_marks() -> (&'static [u8], &'static [u8]) {
    let codeset = unsafe { crate::libc::nl_langinfo(crate::libc::CODESET) };
    if codeset.is_null() {
        return (b"'", b"'");
    }
    numeric_locale_quote_marks_for_codeset(unsafe { CStr::from_ptr(codeset) }.to_bytes())
}

fn numeric_locale_quote_marks_for_codeset(codeset: &[u8]) -> (&'static [u8], &'static [u8]) {
    if codeset.eq_ignore_ascii_case(b"UTF-8") || codeset.eq_ignore_ascii_case(b"UTF8") {
        (b"\xe2\x80\x98", b"\xe2\x80\x99")
    } else if codeset.eq_ignore_ascii_case(b"GB18030") {
        // Match the fallback byte strings emitted by GNU quotearg.c.
        (b"\xa1\x07e", b"\xa1\xaf")
    } else {
        (b"'", b"'")
    }
}

fn push_octal_byte_to_vec(escaped: &mut Vec<u8>, byte: u8) {
    escaped.push(b'\\');
    escaped.push(b'0' + (byte >> 6));
    escaped.push(b'0' + ((byte >> 3) & 7));
    escaped.push(b'0' + (byte & 7));
}

fn show_numeric_error(quoted: &[u8], message: &[u8]) {
    let utility_name = crate::ct_util_name();
    let mut diagnostic = Vec::with_capacity(utility_name.len() + quoted.len() + message.len() + 4);
    diagnostic.extend_from_slice(utility_name.as_bytes());
    diagnostic.extend_from_slice(b": ");
    diagnostic.extend_from_slice(quoted);
    diagnostic.extend_from_slice(b": ");
    diagnostic.extend_from_slice(message);
    diagnostic.push(b'\n');
    let _ = std::io::stderr().lock().write_all(&diagnostic);
}

// 该函数接收两个通用参数： T 和 ParseError<'_, T>。该函数用于从解析结果中提取值，并处理可能出现的解析错误。
// 函数首先检查解析结果 (p) 是否为 OK，即解析是否成功。如果是，则返回解析后的值 (v)。
// 如果解析结果为 Err，表示解析过程中出现错误，函数会将退出代码设为 1（表示出现错误），然后继续处理错误。
fn extract_value<T: Default>(p: Result<T, ParseError<'_, T>>, input: &str) -> T {
    extract_value_with_overflow(p, input, T::default())
}

fn extract_value_with_overflow<T: Default>(
    p: Result<T, ParseError<'_, T>>,
    input: &str,
    overflow_value: T,
) -> T {
    match p {
        Ok(v) => v,
        Err(e) => {
            let input_escaped = quote_numeric_argument(OsStr::new(input));
            match e {
                ParseError::CtOverflow => {
                    set_ct_exit_code(1);
                    show_numeric_error(&input_escaped, b"Numerical result out of range");
                    overflow_value
                }
                ParseError::CtNotNumeric => {
                    set_ct_exit_code(1);
                    show_numeric_error(&input_escaped, b"expected a numeric value");
                    Default::default()
                }
                ParseError::CtPartialMatch(v, rest) => {
                    if input.starts_with('\'') || input.starts_with('"') {
                        if std::env::var_os("POSIXLY_CORRECT").is_none() {
                            ct_show_warning!(
                                "{}: character(s) following character constant have been ignored",
                                &rest,
                            );
                        }
                    } else {
                        set_ct_exit_code(1);
                        show_numeric_error(&input_escaped, b"value not completely converted");
                    }
                    v
                }
            }
        }
    }
}

fn signed_overflow_value(input: &str) -> i64 {
    if input.starts_with('-') {
        i64::MIN
    } else {
        i64::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod argument_tests {
        use super::*;

        struct MockArgumentIter<'a> {
            args: &'a [FormatArgument],
            index: usize,
        }

        impl<'a> MockArgumentIter<'a> {
            fn new(args: &'a [FormatArgument]) -> Self {
                MockArgumentIter { args, index: 0 }
            }

            fn get_char(&mut self) -> u8 {
                if let Some(arg) = self.next() {
                    match arg {
                        FormatArgument::Unparsed(s) => s.bytes().next().unwrap_or(b'\0'),
                        FormatArgument::Char(c) => *c as u8,
                        _ => b'\0',
                    }
                } else {
                    b'\0'
                }
            }

            fn get_i64(&mut self) -> i64 {
                if let Some(arg) = self.next() {
                    match arg {
                        FormatArgument::Unparsed(s) => s.parse::<i64>().unwrap_or(0),
                        FormatArgument::SignedInt(n) => *n,
                        _ => 0,
                    }
                } else {
                    0
                }
            }

            fn get_u64(&mut self) -> u64 {
                if let Some(arg) = self.next() {
                    match arg {
                        FormatArgument::Unparsed(s) => s.parse::<u64>().unwrap_or(0),
                        FormatArgument::UnsignedInt(n) => *n,
                        _ => 0,
                    }
                } else {
                    0
                }
            }

            fn get_f64(&mut self) -> f64 {
                if let Some(arg) = self.next() {
                    match arg {
                        FormatArgument::Unparsed(s) => s.parse::<f64>().unwrap_or(0.0),
                        FormatArgument::Float(n) => *n,
                        _ => 0.0,
                    }
                } else {
                    0.0
                }
            }

            fn get_str(&mut self) -> &'a str {
                if let Some(FormatArgument::Unparsed(s) | FormatArgument::String(s)) = self.next() {
                    s
                } else {
                    ""
                }
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
                FormatArgument::Float(std::f64::consts::PI),
                FormatArgument::Unparsed(std::f64::consts::E.to_string()),
                FormatArgument::SignedInt(-42),
                FormatArgument::String("1.618".to_string()),
            ];
            let mut iter = MockArgumentIter::new(&args);

            assert_eq!(iter.get_f64(), std::f64::consts::PI);
            assert_eq!(iter.get_f64(), std::f64::consts::E); //对于 Unparsed 类型，应将字符串解析为 u64
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

    #[test]
    fn empty_byte_integer_arguments_are_zero_without_an_error_status() {
        crate::ct_error::set_ct_exit_code(0);

        assert_eq!(parse_bytes_i64(b""), 0);
        assert_eq!(parse_bytes_u64(b""), 0);
        assert_eq!(crate::ct_error::get_ct_exit_code(), 0);
    }

    #[test]
    fn numeric_diagnostic_quotes_keep_spaces_unescaped() {
        assert_eq!(quote_numeric_argument(OsStr::new(" ")), b"' '");
        assert_eq!(quote_numeric_argument(OsStr::new("a b")), b"'a b'");
    }

    #[test]
    fn numeric_diagnostic_preserves_printable_single_byte_locale_bytes() {
        assert_eq!(
            quote_numeric_bytes_with_classifier(b"\xa01", b"'", b"'", |_| (1, true)),
            b"'\xa01'"
        );
    }

    #[test]
    fn numeric_diagnostic_classifies_multibyte_locale_sequences() {
        assert_eq!(
            quote_numeric_bytes_with_classifier(b"\xc2\x81\xff", b"'", b"'", |bytes| {
                if bytes.starts_with(b"\xc2\x81") {
                    (2, true)
                } else {
                    (1, false)
                }
            }),
            b"'\xc2\x81\\377'"
        );
    }

    #[test]
    fn numeric_diagnostic_uses_codeset_specific_quote_marks() {
        assert_eq!(
            numeric_locale_quote_marks_for_codeset(b"UTF-8"),
            (&b"\xe2\x80\x98"[..], &b"\xe2\x80\x99"[..])
        );
        assert_eq!(
            numeric_locale_quote_marks_for_codeset(b"ISO-8859-1"),
            (&b"'"[..], &b"'"[..])
        );
    }

    #[test]
    fn character_constant_warning_preserves_trailing_bytes() {
        assert_eq!(
            character_constant_warning("printf", &[0xa9]),
            b"printf: warning: \xa9: character(s) following character constant have been ignored\n"
        );
    }
}
