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

//! unexpand命令通常用于将行首的空格转换为制表符，这样可以使得文本在显示时按照固定的列对齐，尤其是在处理纯文本表格时。

extern crate rust_i18n;
use rust_i18n::t;
use std::error::Error;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, ArgAction, ArgMatches, Command, builder::OsStringValueParser, crate_version};
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write, stdout};
use std::path::Path;
use std::str::from_utf8;
use sys_locale::get_locale;

use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo, set_ct_exit_code, strip_errno};
use ctcore::ct_posix::{GnuGetoptCommandExt, posixly_correct};
use ctcore::ct_quoting_style::escape_shell_bytes_with_classifier;
use std::ffi::{CStr, OsStr, OsString};

const UNEXPAND_DEFAULT_TABSTOP: usize = 8;
const UNEXPAND_INPUT_LINE_TOO_LONG: &str = "input line is too long";
const UNEXPAND_UTF8_LOCALES: [&CStr; 3] = [c"C.UTF8", c"en_US.UTF8", c"en_GB.UTF8"];

unsafe extern "C" {
    fn iswblank(wide: ctcore::libc::c_uint) -> ctcore::libc::c_int;
    fn iswcntrl(wide: ctcore::libc::c_uint) -> ctcore::libc::c_int;
    fn iswprint(wide: ctcore::libc::c_uint) -> ctcore::libc::c_int;
    fn mbrtowc(
        wide: *mut ctcore::libc::wchar_t,
        bytes: *const ctcore::libc::c_char,
        length: usize,
        state: *mut ctcore::libc::mbstate_t,
    ) -> usize;
    fn wcwidth(wide: ctcore::libc::wchar_t) -> ctcore::libc::c_int;
}

#[derive(Debug, PartialEq)]
enum UnexpandParseError {
    InvalidCharacter(String),
    InvalidCharacterBytes(Vec<u8>),
    SpecifierNotAtStartOfNumber(String, String),
    SpecifierNotAtStartOfNumberBytes(String, Vec<u8>),
    Multiple(Vec<UnexpandParseError>),
    SpecifierOnlyAllowedWithLastValue(String),
    SpecifierMutuallyExclusive,
    TabSizeCannotBeZero,
    TabStopTooLarge(String),
    TabStopValueTooLarge,
    TabSizesMustBeAscending,
}

impl Error for UnexpandParseError {}

impl CTError for UnexpandParseError {
    fn diagnostic_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        let diagnostic = match self {
            Self::InvalidCharacter(text) => {
                let mut diagnostic = b"tab size contains invalid character(s): ".to_vec();
                diagnostic.extend_from_slice(&unexpand_quote_diagnostic_argument(text.as_bytes()));
                diagnostic
            }
            Self::InvalidCharacterBytes(bytes) => {
                let mut diagnostic = b"tab size contains invalid character(s): ".to_vec();
                diagnostic.extend_from_slice(&unexpand_quote_diagnostic_argument(bytes));
                diagnostic
            }
            Self::SpecifierNotAtStartOfNumber(specifier, text) => {
                let mut diagnostic = Vec::with_capacity(specifier.len() + text.len() + 48);
                diagnostic.push(b'\'');
                diagnostic.extend_from_slice(specifier.as_bytes());
                diagnostic.push(b'\'');
                diagnostic.extend_from_slice(b" specifier not at start of number: ");
                diagnostic.extend_from_slice(&unexpand_quote_diagnostic_argument(text.as_bytes()));
                diagnostic
            }
            Self::SpecifierNotAtStartOfNumberBytes(specifier, bytes) => {
                let mut diagnostic = Vec::with_capacity(specifier.len() + bytes.len() + 48);
                diagnostic.push(b'\'');
                diagnostic.extend_from_slice(specifier.as_bytes());
                diagnostic.push(b'\'');
                diagnostic.extend_from_slice(b" specifier not at start of number: ");
                diagnostic.extend_from_slice(&unexpand_quote_diagnostic_argument(bytes));
                diagnostic
            }
            Self::Multiple(errors) => unexpand_join_diagnostic_bytes(errors),
            Self::TabStopTooLarge(text) => {
                let mut diagnostic = b"tab stop is too large ".to_vec();
                diagnostic.extend_from_slice(&unexpand_quote_diagnostic_argument(text.as_bytes()));
                diagnostic
            }
            _ => return std::borrow::Cow::Owned(self.to_string().into_bytes()),
        };
        std::borrow::Cow::Owned(diagnostic)
    }
}

impl fmt::Display for UnexpandParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::InvalidCharacter(s) => {
                write!(f, "tab size contains invalid character(s): {}", s.quote())
            }
            Self::InvalidCharacterBytes(bytes) => write!(
                f,
                "tab size contains invalid character(s): {}",
                String::from_utf8_lossy(&unexpand_quote_diagnostic_argument(bytes))
            ),
            Self::SpecifierNotAtStartOfNumber(specifier, s) => write!(
                f,
                "{} specifier not at start of number: {}",
                specifier.quote(),
                s.quote()
            ),
            Self::SpecifierNotAtStartOfNumberBytes(specifier, bytes) => write!(
                f,
                "{} specifier not at start of number: {}",
                specifier.quote(),
                String::from_utf8_lossy(&unexpand_quote_diagnostic_argument(bytes))
            ),
            Self::Multiple(errors) => {
                for (index, error) in errors.iter().enumerate() {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    error.fmt(f)?;
                }
                Ok(())
            }
            Self::SpecifierOnlyAllowedWithLastValue(specifier) => write!(
                f,
                "{} specifier only allowed with the last value",
                specifier.quote()
            ),
            Self::SpecifierMutuallyExclusive => {
                write!(f, "'/' specifier is mutually exclusive with '+'")
            }
            Self::TabSizeCannotBeZero => write!(f, "tab size cannot be 0"),
            Self::TabStopTooLarge(s) => write!(f, "tab stop is too large {}", s.quote()),
            Self::TabStopValueTooLarge => write!(f, "tab stop value is too large"),
            Self::TabSizesMustBeAscending => write!(f, "tab sizes must be ascending"),
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
enum RemainingMode {
    None,
    Slash,
    Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnexpandTabstopMode {
    None,
    Slash,
    Plus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnexpandRow {
    pub row_index: usize,
    pub line: String,
    pub has_tabs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnexpandSemantic {
    pub tabstop_mode: UnexpandTabstopMode,
    pub tabstops: Vec<usize>,
    pub all_blanks: bool,
    pub assume_utf8: bool,
    pub rows: Vec<UnexpandRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct UnexpandRunOutcome {
    stderr_text: String,
    exit_code: i32,
}

fn unexpand_tabstop_mode(remaining_mode: RemainingMode) -> UnexpandTabstopMode {
    match remaining_mode {
        RemainingMode::None => UnexpandTabstopMode::None,
        RemainingMode::Slash => UnexpandTabstopMode::Slash,
        RemainingMode::Plus => UnexpandTabstopMode::Plus,
    }
}

fn unexpand_rows_from_output(output: &str) -> Vec<UnexpandRow> {
    output
        .split_terminator('\n')
        .enumerate()
        .map(|(index, line)| UnexpandRow {
            row_index: index + 1,
            line: line.to_string(),
            has_tabs: line.contains('\t'),
        })
        .collect()
}

/// 判断字节是否为空格、水平制表符或逗号。
fn is_space_or_comma(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b',')
}

#[cfg(test)]
fn unexpand_tabstops_parse(
    s: &str,
    from_short_tabs: bool,
) -> Result<(RemainingMode, Vec<usize>), UnexpandParseError> {
    let (mode, tabstops) = unexpand_tabstops_parse_inner(OsStr::new(s), from_short_tabs, false)?;
    let explicit_end = if mode == RemainingMode::None {
        tabstops.len()
    } else {
        tabstops.len() - 1
    };
    unexpand_validate_tabstops(&tabstops[..explicit_end])?;
    Ok((mode, tabstops))
}

fn unexpand_tabstops_parse_inner(
    s: &OsStr,
    from_short_tabs: bool,
    preserve_single_extension: bool,
) -> Result<(RemainingMode, Vec<usize>), UnexpandParseError> {
    let bytes = s.as_encoded_bytes();
    let first = bytes
        .iter()
        .position(|byte| !is_space_or_comma(*byte))
        .unwrap_or(bytes.len());
    let bytes = &bytes[first..];
    if bytes.is_empty() {
        return Ok((RemainingMode::None, vec![UNEXPAND_DEFAULT_TABSTOP]));
    }

    let mut numbers = Vec::new();
    let mut current_mode = RemainingMode::None;
    let mut slash_extension = None;
    let mut plus_extension = None;
    let mut errors = Vec::new();
    let mut have_number = false;
    let mut number = 0usize;
    let mut number_start = 0usize;
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            byte if is_space_or_comma(byte) => {
                if have_number {
                    unexpand_record_tabstop(
                        number,
                        current_mode,
                        &mut numbers,
                        &mut slash_extension,
                        &mut plus_extension,
                        &mut errors,
                    );
                    have_number = false;
                }
                index += 1;
            }
            b'/' => {
                if have_number {
                    errors.push(unexpand_specifier_not_at_start_error("/", &bytes[index..]));
                }
                current_mode = RemainingMode::Slash;
                index += 1;
            }
            b'+' => {
                if have_number {
                    errors.push(unexpand_specifier_not_at_start_error("+", &bytes[index..]));
                }
                current_mode = RemainingMode::Plus;
                index += 1;
            }
            byte @ b'0'..=b'9' => {
                if !have_number {
                    have_number = true;
                    number = 0;
                    number_start = index;
                }

                let digit = usize::from(byte - b'0');
                if let Some(value) = number
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(digit))
                {
                    number = value;
                    index += 1;
                } else {
                    let number_end = bytes[number_start..]
                        .iter()
                        .position(|byte| !byte.is_ascii_digit())
                        .map_or(bytes.len(), |offset| number_start + offset);
                    let overflowing_number = from_utf8(&bytes[number_start..number_end])
                        .expect("tab-stop number contains only ASCII digits");
                    errors.push(if from_short_tabs {
                        UnexpandParseError::TabStopValueTooLarge
                    } else {
                        UnexpandParseError::TabStopTooLarge(overflowing_number.to_string())
                    });
                    have_number = false;
                    index = number_end;
                }
            }
            _ => {
                errors.push(unexpand_invalid_character_error(&bytes[index..]));
                break;
            }
        }
    }

    if have_number && errors.is_empty() {
        unexpand_record_tabstop(
            number,
            current_mode,
            &mut numbers,
            &mut slash_extension,
            &mut plus_extension,
            &mut errors,
        );
    }

    if !errors.is_empty() {
        return Err(unexpand_combine_parse_errors(errors));
    }

    if slash_extension.is_some() && plus_extension.is_some() {
        return Err(UnexpandParseError::SpecifierMutuallyExclusive);
    }

    let extension = slash_extension
        .map(|tabstop| (RemainingMode::Slash, tabstop))
        .or_else(|| plus_extension.map(|tabstop| (RemainingMode::Plus, tabstop)));

    if let Some((remaining_mode, tabstop)) = extension {
        if numbers.is_empty() {
            return Ok(if preserve_single_extension {
                (remaining_mode, vec![tabstop])
            } else {
                (RemainingMode::None, vec![tabstop])
            });
        }
        numbers.push(tabstop);
        return Ok((remaining_mode, numbers));
    }

    if numbers.is_empty() && preserve_single_extension && current_mode != RemainingMode::None {
        return Ok((RemainingMode::None, vec![]));
    }

    if numbers.is_empty() {
        return Ok((RemainingMode::None, vec![UNEXPAND_DEFAULT_TABSTOP]));
    }

    Ok((RemainingMode::None, numbers))
}

fn unexpand_record_tabstop(
    number: usize,
    mode: RemainingMode,
    numbers: &mut Vec<usize>,
    slash_extension: &mut Option<usize>,
    plus_extension: &mut Option<usize>,
    errors: &mut Vec<UnexpandParseError>,
) {
    match mode {
        RemainingMode::None => numbers.push(number),
        RemainingMode::Slash => {
            if slash_extension.is_some() {
                errors.push(UnexpandParseError::SpecifierOnlyAllowedWithLastValue(
                    "/".to_string(),
                ));
            }
            *slash_extension = (number != 0).then_some(number);
        }
        RemainingMode::Plus => {
            if plus_extension.is_some() {
                errors.push(UnexpandParseError::SpecifierOnlyAllowedWithLastValue(
                    "+".to_string(),
                ));
            }
            *plus_extension = (number != 0).then_some(number);
        }
    }
}

fn unexpand_combine_parse_errors(mut errors: Vec<UnexpandParseError>) -> UnexpandParseError {
    if errors.len() == 1 {
        errors.pop().expect("one parse error")
    } else {
        UnexpandParseError::Multiple(errors)
    }
}

fn unexpand_validate_tabstops(tabstops: &[usize]) -> Result<(), UnexpandParseError> {
    let mut previous = None;
    for tabstop in tabstops {
        if *tabstop == 0 {
            return Err(UnexpandParseError::TabSizeCannotBeZero);
        }
        if previous.is_some_and(|previous| previous >= *tabstop) {
            return Err(UnexpandParseError::TabSizesMustBeAscending);
        }
        previous = Some(*tabstop);
    }
    Ok(())
}

fn unexpand_invalid_character_error(bytes: &[u8]) -> UnexpandParseError {
    match from_utf8(bytes) {
        Ok(text) => UnexpandParseError::InvalidCharacter(text.to_string()),
        Err(_) => UnexpandParseError::InvalidCharacterBytes(bytes.to_vec()),
    }
}

fn unexpand_specifier_not_at_start_error(specifier: &str, bytes: &[u8]) -> UnexpandParseError {
    match from_utf8(bytes) {
        Ok(text) => {
            UnexpandParseError::SpecifierNotAtStartOfNumber(specifier.to_string(), text.to_string())
        }
        Err(_) => UnexpandParseError::SpecifierNotAtStartOfNumberBytes(
            specifier.to_string(),
            bytes.to_vec(),
        ),
    }
}

fn unexpand_quote_diagnostic_argument(bytes: &[u8]) -> Vec<u8> {
    let (left_quote, right_quote) = unexpand_diagnostic_quote_marks();
    unexpand_quote_diagnostic_argument_with_quote_marks(bytes, left_quote, right_quote)
}

fn unexpand_quote_diagnostic_argument_with_quote_marks(
    bytes: &[u8],
    left_quote: &[u8],
    right_quote: &[u8],
) -> Vec<u8> {
    let mut quoted = Vec::with_capacity(bytes.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(left_quote);

    let mut index = 0;
    while index < bytes.len() {
        if !right_quote.is_empty() && bytes[index..].starts_with(right_quote) {
            quoted.push(b'\\');
            quoted.extend_from_slice(right_quote);
            index += right_quote.len();
            continue;
        }

        let byte = bytes[index];
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
                0x00..=0x1f | 0x7f => unexpand_push_octal_byte(&mut quoted, byte),
                _ => quoted.push(byte),
            }
            index += 1;
            continue;
        }

        let (length, printable) = unexpand_classify_locale_sequence(&bytes[index..]);
        let length = length.clamp(1, bytes.len() - index);
        if printable {
            quoted.extend_from_slice(&bytes[index..index + length]);
        } else {
            for byte in &bytes[index..index + length] {
                unexpand_push_octal_byte(&mut quoted, *byte);
            }
        }
        index += length;
    }

    quoted.extend_from_slice(right_quote);
    quoted
}

fn unexpand_join_diagnostic_bytes(errors: &[UnexpandParseError]) -> Vec<u8> {
    let utility_name = ctcore::ct_util_name();
    let mut diagnostic = Vec::new();
    for (index, error) in errors.iter().enumerate() {
        if index > 0 {
            diagnostic.push(b'\n');
            diagnostic.extend_from_slice(utility_name.as_bytes());
            diagnostic.extend_from_slice(b": ");
        }
        diagnostic.extend_from_slice(error.diagnostic_bytes().as_ref());
    }
    diagnostic
}

fn unexpand_diagnostic_quote_marks() -> (&'static [u8], &'static [u8]) {
    let codeset = unsafe { ctcore::libc::nl_langinfo(ctcore::libc::CODESET) };
    if codeset.is_null() {
        return (b"'", b"'");
    }

    let codeset = unsafe { CStr::from_ptr(codeset) }.to_bytes();
    if codeset.eq_ignore_ascii_case(b"UTF-8") || codeset.eq_ignore_ascii_case(b"UTF8") {
        (b"\xe2\x80\x98", b"\xe2\x80\x99")
    } else if codeset.eq_ignore_ascii_case(b"GB18030") {
        (b"\xa1\x07e", b"\xa1\xaf")
    } else {
        (b"'", b"'")
    }
}

fn unexpand_classify_locale_sequence(remaining: &[u8]) -> (usize, bool) {
    unsafe {
        let mut state: ctcore::libc::mbstate_t = std::mem::zeroed();
        let mut wide = 0 as ctcore::libc::wchar_t;
        let length = mbrtowc(
            &mut wide,
            remaining.as_ptr().cast(),
            remaining.len(),
            &mut state,
        );
        if length == usize::MAX {
            return (1, false);
        }
        if length == usize::MAX - 1 {
            return (remaining.len(), false);
        }

        let length = if length == 0 { 1 } else { length };
        (length, iswprint(wide as ctcore::libc::c_uint) != 0)
    }
}

fn unexpand_push_octal_byte(output: &mut Vec<u8>, byte: u8) {
    output.push(b'\\');
    output.push(b'0' + (byte >> 6));
    output.push(b'0' + ((byte >> 3) & 0o7));
    output.push(b'0' + (byte & 0o7));
}

mod unexpand_flags {
    pub const FILE: &str = "file";
    pub const ALL: &str = "all";
    pub const FIRST_ONLY: &str = "first-only";
    pub const TABS: &str = "tabs";
    pub const NO_UTF8: &str = "no-utf8";
    pub const SHORT_TABS: &str = "short-tabs";
}

#[derive(Clone)]
struct UnexpandFlags {
    files: Vec<OsString>,
    tabstops: Vec<usize>,
    remaining_mode: RemainingMode,
    is_a_flag: bool,
    is_u_flag: bool,
}

impl UnexpandFlags {
    fn new(matches: &clap::ArgMatches) -> Result<Self, UnexpandParseError> {
        let (remaining_mode, tabstops) = Self::parse_tabstops(matches)?;

        let is_a_flag = Self::parse_a_flag(matches);
        let is_u_flag = Self::parse_u_flag(matches);
        let files = Self::parse_files(matches);

        Ok(Self {
            files,
            tabstops,
            remaining_mode,
            is_a_flag,
            is_u_flag,
        })
    }

    fn parse_u_flag(matches: &ArgMatches) -> bool {
        !matches.get_flag(unexpand_flags::NO_UTF8)
    }

    fn parse_files(matches: &ArgMatches) -> Vec<OsString> {
        if let Some(v) = matches.get_many::<OsString>(unexpand_flags::FILE) {
            v.cloned().collect()
        } else {
            vec![OsString::from("-")]
        }
    }

    fn parse_a_flag(matches: &ArgMatches) -> bool {
        (matches.get_count(unexpand_flags::ALL) > 0 || matches.contains_id(unexpand_flags::TABS))
            && matches.get_count(unexpand_flags::FIRST_ONLY) == 0
    }

    fn parse_tabstops(
        matches: &ArgMatches,
    ) -> Result<(RemainingMode, Vec<usize>), UnexpandParseError> {
        let from_short_tabs = matches.get_flag(unexpand_flags::SHORT_TABS);
        if let Some(s) = matches.get_many::<OsString>(unexpand_flags::TABS) {
            let mut tabstops = Vec::new();
            let mut extension = None;

            for input in s {
                if input
                    .as_encoded_bytes()
                    .iter()
                    .all(|byte| is_space_or_comma(*byte))
                {
                    continue;
                }

                if let Some(mode) =
                    unexpand_reused_zero_extension_mode(input, extension.map(|(mode, _)| mode))
                {
                    let specifier = match mode {
                        RemainingMode::Slash => "/",
                        RemainingMode::Plus => "+",
                        RemainingMode::None => unreachable!("zero extension has a prefix"),
                    };
                    return Err(UnexpandParseError::SpecifierOnlyAllowedWithLastValue(
                        specifier.to_string(),
                    ));
                }

                let (mode, mut option_tabstops) =
                    unexpand_tabstops_parse_inner(input, from_short_tabs, true)?;
                let option_extension = if mode == RemainingMode::None {
                    None
                } else {
                    option_tabstops.pop()
                };

                tabstops.extend(option_tabstops);

                if let Some(tabstop) = option_extension {
                    if let Some((existing_mode, _)) = extension {
                        return Err(if existing_mode == mode {
                            let specifier = match mode {
                                RemainingMode::Slash => "/",
                                RemainingMode::Plus => "+",
                                RemainingMode::None => unreachable!(),
                            };
                            UnexpandParseError::SpecifierOnlyAllowedWithLastValue(
                                specifier.to_string(),
                            )
                        } else {
                            UnexpandParseError::SpecifierMutuallyExclusive
                        });
                    }
                    extension = Some((mode, tabstop));
                }
            }

            if let Some((mode, tabstop)) = extension {
                unexpand_validate_tabstops(&tabstops)?;
                if tabstops.is_empty() {
                    return Ok((RemainingMode::None, vec![tabstop]));
                }
                tabstops.push(tabstop);
                return Ok((mode, tabstops));
            }

            if tabstops.is_empty() {
                return Ok((RemainingMode::None, vec![UNEXPAND_DEFAULT_TABSTOP]));
            }

            unexpand_validate_tabstops(&tabstops)?;
            return Ok((RemainingMode::None, tabstops));
        }
        Ok((RemainingMode::None, vec![UNEXPAND_DEFAULT_TABSTOP]))
    }
}

/// GNU records a nonzero extension before assigning the next value.  Therefore
/// a later `/0` or `+0` still triggers the duplicate-extension diagnostic.
fn unexpand_reused_zero_extension_mode(
    input: &OsStr,
    existing_mode: Option<RemainingMode>,
) -> Option<RemainingMode> {
    let bytes = input.as_encoded_bytes();
    let mut mode = RemainingMode::None;
    let mut have_number = false;
    let mut number = 0usize;

    let check_number = |mode: RemainingMode, number: usize| {
        (number == 0 && Some(mode) == existing_mode).then_some(mode)
    };

    for byte in bytes {
        match *byte {
            byte if is_space_or_comma(byte) => {
                if have_number {
                    if let Some(mode) = check_number(mode, number) {
                        return Some(mode);
                    }
                    have_number = false;
                }
            }
            b'/' | b'+' if have_number => {
                // GNU reports the misplaced prefix and does not commit this value.
                return None;
            }
            b'/' => mode = RemainingMode::Slash,
            b'+' => mode = RemainingMode::Plus,
            byte @ b'0'..=b'9' => {
                if !have_number {
                    number = 0;
                }
                have_number = true;
                number = number
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(usize::from(byte - b'0')))?;
            }
            _ => return None,
        }
    }

    have_number.then(|| check_number(mode, number)).flatten()
}

/// 判断字节是否为ASCII数字或逗号。
#[cfg(test)]
fn is_ascii_digit_or_comma(c: u8) -> bool {
    c.is_ascii_digit() || c == b','
}

fn unexpand_is_all_option(argument: &[u8]) -> bool {
    argument.len() > 2 && b"--all".starts_with(argument)
}

fn unexpand_is_tabs_option(argument: &[u8]) -> bool {
    let option = argument
        .split(|byte| *byte == b'=')
        .next()
        .expect("split always returns the first field");
    option.len() > 2 && b"--tabs".starts_with(option)
}

fn unexpand_short_option_from_bytes(bytes: &[u8]) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        OsString::from_vec(bytes.to_vec())
    }

    #[cfg(not(unix))]
    {
        OsString::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

#[cfg(test)]
fn is_digit_or_comma(c: char) -> bool {
    c.is_ascii() && is_ascii_digit_or_comma(c as u8)
}

/// 预处理命令行参数并展开快捷方式。例如，"-7"会被扩展为"--tabs=7 --first-only"，
/// 而"-1,3"会扩展为"--tabs=1 --tabs=3 --first-only"。
/// 但是，如果提供了"-a"、"--all"或"-t"/"--tabs"选项，则不会包含"--first-only"。
fn expand_shortcuts_os(args: &[OsString], posix_mode: bool) -> Vec<OsString> {
    let mut processed_args = Vec::with_capacity(args.len() + 3);
    let Some((program_name, args)) = args.split_first() else {
        return processed_args;
    };
    processed_args.push(program_name.clone());
    let mut is_all_arg_provided = false;
    let mut is_tabs_arg_provided = false;
    let mut is_has_shortcuts = false;
    let mut options_ended = false;
    let mut generated_options_index = None;
    let mut pending_short_tab = Vec::new();
    let mut tabs_value_expected = false;

    for argument in args {
        let bytes = argument.as_encoded_bytes();
        if tabs_value_expected {
            processed_args.push(argument.clone());
            tabs_value_expected = false;
            continue;
        }

        if !options_ended && bytes == b"--" {
            generated_options_index = Some(processed_args.len());
            options_ended = true;
            processed_args.push(argument.clone());
            continue;
        }

        if !options_ended && posix_mode && (bytes == b"-" || !bytes.starts_with(b"-")) {
            generated_options_index = Some(processed_args.len());
            options_ended = true;
            processed_args.push(argument.clone());
            continue;
        }

        if !options_ended
            && let Some(short_options) = bytes
                .strip_prefix(b"-")
                .filter(|options| !options.is_empty() && !options.starts_with(b"-"))
        {
            let mut option_index = 0;
            while option_index < short_options.len() {
                match short_options[option_index] {
                    b'0'..=b'9' => {
                        pending_short_tab.push(short_options[option_index]);
                        is_has_shortcuts = true;
                    }
                    b',' => {
                        if !pending_short_tab.is_empty() {
                            let value = from_utf8(&pending_short_tab)
                                .expect("tab shortcut only contains ASCII digits");
                            processed_args.push(OsString::from(format!("--tabs={value}")));
                            pending_short_tab.clear();
                        }
                    }
                    b'a' => {
                        processed_args.push(OsString::from("-a"));
                        is_all_arg_provided = true;
                    }
                    b't' => {
                        is_tabs_arg_provided = true;
                        tabs_value_expected = option_index + 1 == short_options.len();
                        processed_args.push(unexpand_short_option_from_bytes(
                            &[&b"-"[..], &short_options[option_index..]].concat(),
                        ));
                        break;
                    }
                    _ => {
                        if option_index == 0 {
                            processed_args.push(argument.clone());
                        } else {
                            processed_args.push(unexpand_short_option_from_bytes(
                                &[&b"-"[..], &short_options[option_index..]].concat(),
                            ));
                        }
                        break;
                    }
                }
                option_index += 1;
            }
            continue;
        }

        processed_args.push(argument.clone());
        if !options_ended && (unexpand_is_all_option(bytes) || bytes == b"-a") {
            is_all_arg_provided = true;
        }
        if !options_ended && unexpand_is_tabs_option(bytes) {
            is_tabs_arg_provided = true;
            tabs_value_expected = !bytes.contains(&b'=');
        }
    }

    let generated_options_index = generated_options_index.unwrap_or(processed_args.len());
    if !pending_short_tab.is_empty() {
        let value = from_utf8(&pending_short_tab).expect("tab shortcut only contains ASCII digits");
        processed_args.insert(
            generated_options_index,
            OsString::from(format!("--tabs={value}")),
        );
    }

    if is_has_shortcuts {
        let mut shortcuts = Vec::with_capacity(2);
        if !is_all_arg_provided && !is_tabs_arg_provided {
            shortcuts.push(OsString::from("--first-only"));
        }
        shortcuts.push(OsString::from("--short-tabs"));
        let insertion_index = generated_options_index + usize::from(!pending_short_tab.is_empty());
        processed_args.splice(insertion_index..insertion_index, shortcuts);
    }

    processed_args
}

#[cfg(test)]
fn expand_shortcuts(args: &[String]) -> Vec<String> {
    let mut command_args = vec![OsString::from("unexpand")];
    command_args.extend(args.iter().cloned().map(OsString::from));
    expand_shortcuts_os(&command_args, false)
        .into_iter()
        .skip(1)
        .map(|argument| {
            argument
                .into_string()
                .expect("test arguments are valid UTF-8")
        })
        .collect()
}

pub fn unexpand_main(args: impl ctcore::Args) -> CTResult<()> {
    unexpand_configure_sigpipe();
    unexpand_initialize_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect::<Vec<_>>();

    let posix_mode = posixly_correct();
    let matches = ct_app_with_posix_mode(posix_mode)
        .try_get_matches_from(expand_shortcuts_os(&args, posix_mode))?;

    unexpand(&UnexpandFlags::new(&matches)?)
}

#[cfg(target_os = "linux")]
fn unexpand_configure_sigpipe() {
    unexpand_restore_default_sigpipe_if_needed(ctcore::ct_sigpipe_was_default(), || {
        let _ = ctcore::ct_signals::enable_pipe_errors();
    });
}

#[cfg(not(target_os = "linux"))]
fn unexpand_configure_sigpipe() {}

#[cfg(target_os = "linux")]
fn unexpand_restore_default_sigpipe_if_needed(
    inherited_sigpipe_was_default: bool,
    restore_default: impl FnOnce(),
) {
    if inherited_sigpipe_was_default {
        restore_default();
    }
}

fn unexpand_initialize_locale() {
    let empty_locale = b"\0";
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, empty_locale.as_ptr().cast());
    }
}

fn unexpand_uses_utf8_locale() -> bool {
    let locale = unsafe { ctcore::libc::setlocale(ctcore::libc::LC_CTYPE, std::ptr::null()) };
    if locale.is_null() {
        return false;
    }

    unexpand_uses_utf8_locale_name(unsafe { CStr::from_ptr(locale) }.to_bytes())
}

fn unexpand_uses_utf8_locale_name(locale: &[u8]) -> bool {
    locale
        .windows(b"utf8".len())
        .any(|part| part.eq_ignore_ascii_case(b"utf8"))
        || locale
            .windows(b"utf-8".len())
            .any(|part| part.eq_ignore_ascii_case(b"utf-8"))
}

fn unexpand_find_utf8_locale(mut set_locale: impl FnMut(&CStr) -> bool) -> bool {
    UNEXPAND_UTF8_LOCALES
        .iter()
        .any(|locale| set_locale(locale))
}

fn unexpand_set_utf8_locale() -> std::io::Result<()> {
    if unexpand_find_utf8_locale(|locale| unsafe {
        !ctcore::libc::setlocale(ctcore::libc::LC_ALL, locale.as_ptr()).is_null()
    }) {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn ct_app() -> Command {
    ct_app_with_posix_mode(posixly_correct())
}

fn ct_app_with_posix_mode(posix_mode: bool) -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("unexpand.about");
    let usage_description = t!("unexpand.usage");
    let args = vec![
        Arg::new(unexpand_flags::FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(unexpand_flags::ALL)
            .short('a')
            .long(unexpand_flags::ALL)
            .help(t!("unexpand.clap.all"))
            .action(ArgAction::Count),
        Arg::new(unexpand_flags::FIRST_ONLY)
            .long(unexpand_flags::FIRST_ONLY)
            .help(t!("unexpand.clap.first_only"))
            .action(ArgAction::Count),
        Arg::new(unexpand_flags::TABS)
            .short('t')
            .long(unexpand_flags::TABS)
            .help(
                "use comma separated LIST of tab positions or have tabs N characters \
                apart instead of 8 (enables -a)",
            )
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_name("N, LIST"),
        Arg::new(unexpand_flags::NO_UTF8)
            .short('U')
            .long(unexpand_flags::NO_UTF8)
            .help(t!("unexpand.clap.no_utf8"))
            .action(ArgAction::SetTrue),
        Arg::new(unexpand_flags::SHORT_TABS)
            .long(unexpand_flags::SHORT_TABS)
            .hide(true)
            .action(ArgAction::SetTrue),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
        .gnu_getopt_with_mode(posix_mode)
}

fn unexpand_open(
    path: &OsStr,
    last_input_errno: &mut Option<i32>,
    stdin_was_closed: bool,
) -> CTResult<BufReader<Box<dyn Read + 'static>>> {
    let file_buf;
    let filename = Path::new(path);
    if filename.is_dir() {
        *last_input_errno = Some(ctcore::libc::EISDIR);
        Err(Box::new(CtSimpleError {
            code: 1,
            message: format!("{}: Is a directory", unexpand_quote_path(path)),
        }))
    } else if path == OsStr::new("-") {
        if stdin_was_closed {
            *last_input_errno = Some(ctcore::libc::EBADF);
            return Err(Box::new(unexpand_stdin_read_error()));
        }
        Ok(BufReader::new(ctcore::ct_io::stdin_reader_box()))
    } else {
        file_buf = File::open(filename).map_err(|error| {
            *last_input_errno = error.raw_os_error();
            error.map_err_context(|| unexpand_quote_path(path)) as Box<dyn CTError>
        })?;
        Ok(BufReader::new(Box::new(file_buf) as Box<dyn Read>))
    }
}

fn unexpand_stdin_read_error() -> CtSimpleError {
    CtSimpleError {
        code: 1,
        message: "-: Bad file descriptor".to_string(),
    }
}

fn unexpand_bom_mismatch_message(
    first_file_has_bom: bool,
    last_input_errno: Option<i32>,
) -> String {
    let errno = first_file_has_bom
        .then_some(ctcore::libc::ENOENT)
        .or(last_input_errno);
    match errno {
        Some(errno) => format!(
            "unexpand: combination of files with and without BOM header: {}\n",
            strip_errno(&std::io::Error::from_raw_os_error(errno))
        ),
        None => "unexpand: combination of files with and without BOM header\n".to_string(),
    }
}

fn unexpand_quote_path(path: &OsStr) -> String {
    let bytes = path.as_encoded_bytes();
    let quoted = escape_shell_bytes_with_classifier(bytes, |remaining| unsafe {
        let mut state: ctcore::libc::mbstate_t = std::mem::zeroed();
        let mut wide = 0 as ctcore::libc::wchar_t;
        let length = mbrtowc(
            &mut wide,
            remaining.as_ptr().cast(),
            remaining.len(),
            &mut state,
        );
        if length == usize::MAX {
            return (1, false);
        }
        if length == usize::MAX - 1 {
            return (remaining.len(), false);
        }

        let length = if length == 0 { 1 } else { length };
        let is_utf8 = from_utf8(&remaining[..length]).is_ok();
        (
            length,
            is_utf8 && iswprint(wide as ctcore::libc::c_uint) != 0,
        )
    });

    String::from_utf8(quoted).expect("shell-escaped paths are valid UTF-8")
}

#[cfg(test)]
fn unexpand_next_tabstop(tabstops: &[usize], col: usize) -> Option<usize> {
    match tabstops.len() {
        1 => Some(tabstops[0] - col % tabstops[0]),
        _ => {
            // 查找下一个较大的标签。
            // 如果列表中没有下一个更大的标签，那么当前的"tab"将被替换为一个空格。
            tabstops.iter().find(|&&t| t > col).map(|t| t - col)
        }
    }
}

#[cfg(test)]
fn unexpand_write_tabs<W: Write>(
    out: &mut W,
    tabstops: &[usize],
    mut s_col: usize,
    col: usize,
    is_prevtab: bool,
    is_init: bool,
    is_amode: bool,
) {
    // 这个条件语句确保以下行为：我们从不将非空白字符前的一个空格转换为制表符，除非这个空格位于行首。
    let is_ai = is_init || is_amode;
    if (is_ai && !is_prevtab && col > s_col + 1)
        || (col > s_col && (is_init || is_ai && is_prevtab))
    {
        while let Some(nts) = unexpand_next_tabstop(tabstops, s_col) {
            if col < s_col + nts {
                break;
            }

            ctcore::ct_crash_if_err!(1, out.write_all(b"\t"));
            s_col += nts;
        }
    }

    while col > s_col {
        ctcore::ct_crash_if_err!(1, out.write_all(b" "));
        s_col += 1;
    }
}

fn unexpand_next_tab_column(
    tabstops: &[usize],
    remaining_mode: RemainingMode,
    col: usize,
    tab_index: &mut usize,
) -> (usize, bool) {
    let total = tabstops.len();

    if total == 0 {
        return (
            col + (UNEXPAND_DEFAULT_TABSTOP - col % UNEXPAND_DEFAULT_TABSTOP),
            false,
        );
    }

    match remaining_mode {
        RemainingMode::None => {
            if total == 1 {
                let size = tabstops[0];
                return (col + (size - col % size), false);
            }
            while *tab_index < total {
                let tab = tabstops[*tab_index];
                if col < tab {
                    return (tab, false);
                }
                *tab_index += 1;
            }
            (0, true)
        }
        RemainingMode::Slash => {
            if total == 1 {
                let size = tabstops[0];
                return (col + (size - col % size), false);
            }
            let last_index = total - 1;
            while *tab_index < last_index {
                let tab = tabstops[*tab_index];
                if col < tab {
                    return (tab, false);
                }
                *tab_index += 1;
            }
            let size = tabstops[last_index];
            (col + (size - col % size), false)
        }
        RemainingMode::Plus => {
            if total == 1 {
                let size = tabstops[0];
                return (col + (size - col % size), false);
            }
            let last_index = total - 1;
            while *tab_index < last_index {
                let tab = tabstops[*tab_index];
                if col < tab {
                    return (tab, false);
                }
                *tab_index += 1;
            }
            let step = tabstops[last_index];
            let end_tab = tabstops[last_index - 1];
            let offset = col.saturating_sub(end_tab);
            (col + (step - (offset % step)), false)
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
enum UnexpandCharType {
    Backspace,
    Space,
    Tab,
    Other,
}

#[cfg(test)]
fn unexpand_next_char_info(
    is_u_flag: bool,
    buf: &[u8],
    byte: usize,
) -> (UnexpandCharType, usize, usize) {
    if !is_u_flag {
        let c = buf[byte];
        return unexpand_char_info_from_char(char::from(c));
    }

    match unexpand_utf8_first_char(&buf[byte..]) {
        Some(Ok((ch, _))) => unexpand_char_info_from_char(ch),
        Some(Err(_)) | None => (UnexpandCharType::Other, 1, 1),
    }
}

fn unexpand_char_info_from_char(ch: char) -> (UnexpandCharType, usize, usize) {
    let c_type = if ch == '\t' {
        UnexpandCharType::Tab
    } else if ch == '\x08' {
        UnexpandCharType::Backspace
    } else if is_blank_char(ch) {
        UnexpandCharType::Space
    } else {
        UnexpandCharType::Other
    };
    let c_width = if matches!(c_type, UnexpandCharType::Tab | UnexpandCharType::Backspace) {
        0
    } else {
        unexpand_char_width(ch)
    };
    (c_type, c_width, ch.len_utf8())
}

/// Return the first complete UTF-8 character, the number of bytes read before
/// a conversion error, or None when EOF is reached in an incomplete sequence.
fn unexpand_utf8_first_char(buf: &[u8]) -> Option<Result<(char, usize), usize>> {
    for length in 1..=buf.len().min(4) {
        match from_utf8(&buf[..length]) {
            Ok(text) => {
                if let Some(ch) = text.chars().next() {
                    return Some(Ok((ch, ch.len_utf8())));
                }
            }
            Err(error) if error.error_len().is_some() => return Some(Err(length)),
            Err(_) => {}
        }
    }
    None
}

fn is_blank_char(ch: char) -> bool {
    unsafe { iswblank(ch as ctcore::libc::c_uint) != 0 }
}

fn unexpand_char_width(ch: char) -> usize {
    let width = unsafe { wcwidth(ch as ctcore::libc::wchar_t) };
    if width >= 0 {
        width as usize
    } else if unsafe { iswcntrl(ch as ctcore::libc::c_uint) } != 0 {
        0
    } else {
        1
    }
}

/// GNU's BOM probe keeps an incomplete EF or EF BB prefix in mbfile's buffer.
/// The following valid character then supplies its classification and width.
fn unexpand_initial_non_bom_prefix_len(buf: &[u8]) -> usize {
    match buf {
        [0xef, second, ..] if *second != 0xbb => 1,
        [0xef, 0xbb, third, ..] if *third != 0xbf => 2,
        _ => 0,
    }
}

/// Reproduce GNU mbfile's recovery after mbrtoc32 consumes bytes past a
/// malformed sequence.  Bytes already buffered by mbfile prefix the next
/// character and inherit that character's classification and display width.
fn unexpand_mbfile_next_char_info(
    is_u_flag: bool,
    buf: &[u8],
    byte: usize,
    buffered_prefix_len: &mut usize,
) -> Option<(UnexpandCharType, usize, usize)> {
    let prefix_len = *buffered_prefix_len;
    let char_byte = byte.checked_add(prefix_len)?;
    let remaining = buf.get(char_byte..)?;

    if remaining.is_empty() {
        if prefix_len == 0 {
            return None;
        }
        *buffered_prefix_len = 0;
        return Some((UnexpandCharType::Other, 1, prefix_len));
    }

    if !is_u_flag {
        let c = *remaining.first()?;
        if c.is_ascii() {
            *buffered_prefix_len = 0;
            let (c_type, c_width, n_bytes) = unexpand_char_info_from_char(char::from(c));
            return Some((c_type, c_width, prefix_len + n_bytes));
        }

        *buffered_prefix_len = prefix_len;
        return Some((UnexpandCharType::Other, 1, 1));
    }

    match unexpand_utf8_first_char(remaining) {
        Some(Ok((ch, n_bytes))) => {
            *buffered_prefix_len = 0;
            let (c_type, c_width, _) = unexpand_char_info_from_char(ch);
            Some((c_type, c_width, prefix_len + n_bytes))
        }
        Some(Err(read_len)) => {
            *buffered_prefix_len = prefix_len + read_len - 1;
            Some((UnexpandCharType::Other, 1, 1))
        }
        None => {
            *buffered_prefix_len = 0;
            Some((UnexpandCharType::Other, 1, prefix_len + remaining.len()))
        }
    }
}

fn unexpand_incomplete_utf8_suffix_len(buf: &[u8]) -> usize {
    let start = buf.len().saturating_sub(4);

    for byte in (start..buf.len()).rev() {
        let suffix = &buf[byte..];
        if let Err(err) = from_utf8(suffix)
            && err.valid_up_to() == 0
            && err.error_len().is_none()
        {
            return suffix.len();
        }
    }

    0
}

#[derive(Default)]
struct UnexpandLineState {
    column: usize,
    tab_index: usize,
    one_blank_before_tab_stop: bool,
    prev_blank: bool,
    pending: Vec<Vec<u8>>,
    convert: bool,
    mbfile_buffered_prefix_len: usize,
    is_file_start: bool,
}

impl UnexpandLineState {
    fn new() -> Self {
        let mut state = Self::default();
        state.reset();
        state.is_file_start = true;
        state
    }

    fn reset(&mut self) {
        self.column = 0;
        self.tab_index = 0;
        self.one_blank_before_tab_stop = false;
        self.prev_blank = true;
        self.pending.clear();
        self.convert = true;
        self.mbfile_buffered_prefix_len = 0;
    }

    fn flush_pending<W: Write>(&mut self, output: &mut W) -> std::io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }

        if self.pending.len() > 1 && self.one_blank_before_tab_stop {
            self.pending[0] = vec![b'\t'];
        }
        for blank in &self.pending {
            output.write_all(blank)?;
        }
        self.pending.clear();
        self.one_blank_before_tab_stop = false;
        Ok(())
    }

    fn finish_line<W: Write>(&mut self, output: &mut W) -> std::io::Result<()> {
        self.flush_pending(output)?;
        self.reset();
        Ok(())
    }
}

#[allow(clippy::cognitive_complexity)]
fn unexpand_line_with_state<W: Write>(
    buf: &[u8],
    output: &mut W,
    flags: &UnexpandFlags,
    tabstops: &[usize],
    remaining_mode: RemainingMode,
    state: &mut UnexpandLineState,
    line_complete: bool,
) -> std::io::Result<()> {
    let mut byte = 0;
    let convert_entire_line = flags.is_a_flag;
    if state.is_file_start {
        state.mbfile_buffered_prefix_len = unexpand_initial_non_bom_prefix_len(buf);
        state.is_file_start = false;
    }
    let mut reached_mbfile_eof = false;

    while byte < buf.len() {
        let Some((c_type, c_width, n_bytes)) = unexpand_mbfile_next_char_info(
            flags.is_u_flag,
            buf,
            byte,
            &mut state.mbfile_buffered_prefix_len,
        ) else {
            reached_mbfile_eof = true;
            break;
        };
        let mut emit_tab = false;

        if state.convert {
            let blank = matches!(c_type, UnexpandCharType::Space | UnexpandCharType::Tab);
            if blank {
                let (next_tab_column, last_tab) = unexpand_next_tab_column(
                    tabstops,
                    remaining_mode,
                    state.column,
                    &mut state.tab_index,
                );
                if last_tab {
                    state.convert = false;
                }
                if state.convert {
                    if next_tab_column < state.column {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            UNEXPAND_INPUT_LINE_TOO_LONG,
                        ));
                    }
                    if c_type == UnexpandCharType::Tab {
                        state.column = next_tab_column;
                        if !state.pending.is_empty() {
                            state.pending[0] = vec![b'\t'];
                        }
                    } else {
                        let next_column = state.column.saturating_add(c_width);
                        if next_column < state.column {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                UNEXPAND_INPUT_LINE_TOO_LONG,
                            ));
                        }
                        state.column = next_column;

                        if !(state.prev_blank && state.column == next_tab_column) {
                            if state.column == next_tab_column {
                                state.one_blank_before_tab_stop = true;
                            }
                            state.pending.push(buf[byte..byte + n_bytes].to_vec());
                            state.prev_blank = true;
                            byte += n_bytes;
                            continue;
                        }

                        emit_tab = true;
                        if !state.pending.is_empty() {
                            state.pending[0] = vec![b'\t'];
                        }
                    }

                    state.pending.truncate(if state.one_blank_before_tab_stop {
                        1
                    } else {
                        0
                    });
                }
            } else if c_type == UnexpandCharType::Backspace {
                state.column = state.column.saturating_sub(1);
                state.tab_index = state.tab_index.saturating_sub(1);
            } else {
                let orig = state.column;
                state.column = state.column.saturating_add(c_width);
                if state.column < orig {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        UNEXPAND_INPUT_LINE_TOO_LONG,
                    ));
                }
            }

            state.flush_pending(output)?;

            state.prev_blank = matches!(c_type, UnexpandCharType::Space | UnexpandCharType::Tab);
            state.convert = state.convert && (convert_entire_line || state.prev_blank);
        }

        if emit_tab {
            output.write_all(b"\t")?;
        } else {
            output.write_all(&buf[byte..byte + n_bytes])?;
        }
        byte += n_bytes;
    }

    if line_complete || reached_mbfile_eof {
        state.finish_line(output)?;
    }

    output.flush()?;
    Ok(())
}

fn report_unexpand_error<F: FnMut(&str)>(
    stderr_text: &mut String,
    exit_code: &mut i32,
    code: i32,
    message: String,
    emit_stderr: &mut F,
) {
    stderr_text.push_str(&message);
    *exit_code = (*exit_code).max(code);
    emit_stderr(&message);
}

fn report_unexpand_ct_error<F: FnMut(&str)>(
    stderr_text: &mut String,
    exit_code: &mut i32,
    err: &dyn CTError,
    emit_stderr: &mut F,
) {
    report_unexpand_error(
        stderr_text,
        exit_code,
        err.code(),
        format!("unexpand: {err}\n"),
        emit_stderr,
    );
}

fn report_unexpand_io_error<F: FnMut(&str)>(
    stderr_text: &mut String,
    exit_code: &mut i32,
    err: &std::io::Error,
    emit_stderr: &mut F,
) {
    report_unexpand_error(
        stderr_text,
        exit_code,
        1,
        format!("unexpand: {err}\n"),
        emit_stderr,
    );
}

fn unexpand_output_error(error: std::io::Error) -> Box<dyn CTError> {
    unexpand_output_error_with_stdout_state(error, ctcore::ct_stdout_was_closed())
}

fn unexpand_output_error_with_stdout_state(
    error: std::io::Error,
    stdout_was_closed: bool,
) -> Box<dyn CTError> {
    if error.kind() == std::io::ErrorKind::InvalidData
        && error.to_string() == UNEXPAND_INPUT_LINE_TOO_LONG
    {
        CtSimpleError::new(1, UNEXPAND_INPUT_LINE_TOO_LONG)
    } else {
        let error = if stdout_was_closed {
            std::io::Error::from_raw_os_error(ctcore::libc::EBADF)
        } else {
            error
        };
        CtSimpleError::new(1, format!("write error: {}", strip_errno(&error)))
    }
}

#[allow(clippy::cognitive_complexity)]
#[cfg(test)]
fn unexpand_line<W: Write>(
    buf: &mut Vec<u8>,
    output: &mut W,
    flags: &UnexpandFlags,
    tabstops: &[usize],
    remaining_mode: RemainingMode,
) -> std::io::Result<()> {
    let mut state = UnexpandLineState::new();
    unexpand_line_with_state(
        buf,
        output,
        flags,
        tabstops,
        remaining_mode,
        &mut state,
        true,
    )?;
    buf.truncate(0);
    Ok(())
}

fn unexpand(flags: &UnexpandFlags) -> CTResult<()> {
    let mut output = BufWriter::new(stdout());
    let mut emit_stderr = |message: &str| eprint!("{message}");
    let outcome = unexpand_to_writer(flags, &mut output, &mut emit_stderr)?;
    output.flush().map_err(unexpand_output_error)?;

    if outcome.exit_code != 0 {
        set_ct_exit_code(outcome.exit_code);
    }
    Ok(())
}

fn unexpand_exe<W: Write>(
    flags: &UnexpandFlags,
    output: &mut W,
) -> Result<UnexpandRunOutcome, Box<dyn CTError>> {
    let mut discard_stderr = |_message: &str| {};
    unexpand_to_writer(flags, output, &mut discard_stderr)
}

fn unexpand_to_writer<W: Write, F: FnMut(&str)>(
    flags: &UnexpandFlags,
    output: &mut W,
    emit_stderr: &mut F,
) -> Result<UnexpandRunOutcome, Box<dyn CTError>> {
    let tabstops = &flags.tabstops[..];
    let remaining_mode = flags.remaining_mode;
    let mut active_flags = flags.clone();
    active_flags.is_u_flag &= unexpand_uses_utf8_locale();
    let using_utf_locale = active_flags.is_u_flag;
    let mut data_buf = Vec::new();
    let mut is_first_file = true;
    let mut first_file_has_bom = false;
    let mut last_input_errno = None;
    let stdin_was_closed = ctcore::ct_stdin_was_closed();
    let mut read_stdin = false;
    let mut stderr_text = String::new();
    let mut exit_code = 0;
    let mut line_state = UnexpandLineState::new();

    'files: for file in &flags.files {
        read_stdin |= file == OsStr::new("-");
        let mut fh = match unexpand_open(file, &mut last_input_errno, stdin_was_closed) {
            Ok(reader) => reader,
            Err(err) => {
                report_unexpand_ct_error(
                    &mut stderr_text,
                    &mut exit_code,
                    err.as_ref(),
                    emit_stderr,
                );
                continue;
            }
        };
        let mut is_first_chunk = true;
        let mut utf8_carry = Vec::new();

        loop {
            data_buf.clear();
            data_buf.append(&mut utf8_carry);
            // 使用 take 限制单次读取的上限，防止在无换行符的无限流中陷入死循环
            let mut chunk_reader = (&mut fh).take(65536);
            let n = match chunk_reader.read_until(b'\n', &mut data_buf) {
                Ok(size) => size,
                Err(e) => {
                    last_input_errno = e.raw_os_error();
                    report_unexpand_io_error(&mut stderr_text, &mut exit_code, &e, emit_stderr);
                    break;
                }
            };

            if n == 0 && data_buf.is_empty() {
                break;
            }

            if is_first_chunk {
                let file_has_bom = data_buf.starts_with(&[0xEF, 0xBB, 0xBF]);
                line_state.is_file_start = !file_has_bom;
                line_state.mbfile_buffered_prefix_len = 0;
                if !is_first_file && !using_utf_locale && file_has_bom != first_file_has_bom {
                    report_unexpand_error(
                        &mut stderr_text,
                        &mut exit_code,
                        1,
                        unexpand_bom_mismatch_message(first_file_has_bom, last_input_errno),
                        emit_stderr,
                    );
                    break 'files;
                }

                if file_has_bom {
                    if is_first_file && !first_file_has_bom {
                        if !using_utf_locale {
                            unexpand_set_utf8_locale().map_err(|error| {
                                let message = if error.raw_os_error() == Some(0) {
                                    "cannot set UTF-8 locale".to_string()
                                } else {
                                    format!("cannot set UTF-8 locale: {}", strip_errno(&error))
                                };
                                CtSimpleError::new(1, message)
                            })?;
                        }
                        output
                            .write_all(&[0xEF, 0xBB, 0xBF])
                            .map_err(unexpand_output_error)?;
                        first_file_has_bom = true;
                        // GNU switches a C locale to UTF-8 for a BOM-prefixed first file.
                        active_flags.is_u_flag = flags.is_u_flag;
                    }
                    data_buf.drain(0..3);
                }
                is_first_chunk = false;
            }

            if data_buf.is_empty() {
                continue;
            }

            let line_complete = data_buf.last() == Some(&b'\n');
            if active_flags.is_u_flag && n != 0 && !line_complete {
                let suffix_len = unexpand_incomplete_utf8_suffix_len(&data_buf);
                if suffix_len != 0 {
                    utf8_carry = data_buf.split_off(data_buf.len() - suffix_len);
                }
            }

            if data_buf.is_empty() {
                continue;
            }

            unexpand_line_with_state(
                &data_buf,
                output,
                &active_flags,
                tabstops,
                remaining_mode,
                &mut line_state,
                line_complete,
            )
            .map_err(unexpand_output_error)?;
        }
        is_first_file = false;
    }
    line_state
        .finish_line(output)
        .map_err(unexpand_output_error)?;

    if read_stdin && stdin_was_closed {
        let error = unexpand_stdin_read_error();
        report_unexpand_ct_error(&mut stderr_text, &mut exit_code, &error, emit_stderr);
    }

    Ok(UnexpandRunOutcome {
        stderr_text,
        exit_code,
    })
}

pub fn unexpand_native_semantic(args: impl ctcore::Args) -> CTResult<UnexpandSemantic> {
    unexpand_initialize_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect::<Vec<_>>();
    let posix_mode = posixly_correct();
    let matches = ct_app_with_posix_mode(posix_mode)
        .try_get_matches_from(expand_shortcuts_os(&args, posix_mode))?;
    let flags = UnexpandFlags::new(&matches)?;
    let mut classic_output = Vec::new();
    let outcome = unexpand_exe(&flags, &mut classic_output)?;
    let classic_text = String::from_utf8_lossy(&classic_output).into_owned();

    Ok(UnexpandSemantic {
        tabstop_mode: unexpand_tabstop_mode(flags.remaining_mode),
        tabstops: flags.tabstops.clone(),
        all_blanks: flags.is_a_flag,
        assume_utf8: flags.is_u_flag,
        rows: unexpand_rows_from_output(&classic_text),
        classic_text,
        stderr_text: outcome.stderr_text,
        exit_code: outcome.exit_code,
    })
}

#[derive(Default)]
pub struct Unexpand;
impl Tool for Unexpand {
    fn name(&self) -> &'static str {
        "unexpand"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        unexpand_main(args.iter().cloned()).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use crate::is_digit_or_comma;
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn test_tool_implementation() {
        let tool = Unexpand;

        // Test name method
        assert_eq!(tool.name(), "unexpand");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("unexpand"));

        // Test execute method with help flag (should work)
        let args: Vec<OsString> = vec![OsString::from("unexpand"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    #[cfg(test)]
    mod unexpand_tests {
        use std::fs::write;
        use std::io::Write;

        use tempfile::{NamedTempFile, tempdir};

        use super::*;

        #[test]
        fn test_unexpand_selects_first_available_utf8_locale() {
            let mut attempted = Vec::new();
            let found = unexpand_find_utf8_locale(|locale| {
                attempted.push(locale.to_bytes().to_vec());
                locale.to_bytes() == b"en_US.UTF8"
            });

            assert!(found);
            assert_eq!(attempted, vec![b"C.UTF8".to_vec(), b"en_US.UTF8".to_vec()]);
        }

        struct FullWriter;

        impl Write for FullWriter {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from_raw_os_error(ctcore::libc::ENOSPC))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        #[test]
        fn test_unexpand_exe_with_single_file() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), b"    Hello\tWorld\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().as_os_str().to_os_string()],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "\tHello\tWorld\n");
        }

        #[test]
        fn test_unexpand_exe_reports_gnu_style_write_error() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), b"        x\n").unwrap();
            let flags = UnexpandFlags {
                files: vec![file.path().as_os_str().to_os_string()],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let error = match unexpand_exe(&flags, &mut FullWriter) {
                Ok(_) => panic!("unexpand unexpectedly wrote to a full output"),
                Err(error) => error,
            };

            assert_eq!(format!("{error}"), "write error: No space left on device");
        }

        #[test]
        fn test_unexpand_closed_stdout_reports_bad_file_descriptor() {
            let error = std::io::Error::from_raw_os_error(ctcore::libc::ENOSPC);

            let error = unexpand_output_error_with_stdout_state(error, true);

            assert_eq!(format!("{error}"), "write error: Bad file descriptor");
        }

        #[test]
        fn test_unexpand_exe_with_multiple_files() {
            let dir = tempdir().unwrap();
            let file1_path = dir.path().join("file1.txt");
            let file2_path = dir.path().join("file2.txt");

            write(&file1_path, b"    Hello\n").unwrap();
            write(&file2_path, b"\tWorld\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![
                    file1_path.as_os_str().to_os_string(),
                    file2_path.as_os_str().to_os_string(),
                ],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "\tHello\n\tWorld\n");
        }

        #[test]
        fn test_unexpand_exe_keeps_line_state_across_unterminated_files() {
            let dir = tempdir().unwrap();
            let first_path = dir.path().join("first.txt");
            let second_path = dir.path().join("second.txt");
            write(&first_path, b"X").unwrap();
            write(&second_path, b"        Y\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![
                    first_path.as_os_str().to_os_string(),
                    second_path.as_os_str().to_os_string(),
                ],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            assert_eq!(output, b"X        Y\n");
        }

        #[test]
        fn test_unexpand_exe_rejects_mixed_bom_files_in_a_non_utf8_locale() {
            let dir = tempdir().unwrap();
            let bom_path = dir.path().join("bom.txt");
            let plain_path = dir.path().join("plain.txt");
            write(&bom_path, b"\xEF\xBB\xBF        first\n").unwrap();
            write(&plain_path, b"        second\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![
                    bom_path.as_os_str().to_os_string(),
                    plain_path.as_os_str().to_os_string(),
                ],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            let outcome = unexpand_exe(&flags, &mut output).unwrap();

            assert_eq!(outcome.exit_code, 1);
            assert_eq!(output, b"\xEF\xBB\xBF\tfirst\n");
            assert!(
                outcome
                    .stderr_text
                    .starts_with("unexpand: combination of files with and without BOM header")
            );
        }

        #[test]
        fn test_unexpand_exe_keeps_open_errno_for_bom_after_plain_file() {
            let dir = tempdir().unwrap();
            let missing_path = dir.path().join("missing.txt");
            let plain_path = dir.path().join("plain.txt");
            let bom_path = dir.path().join("bom.txt");
            write(&plain_path, b"        plain\n").unwrap();
            write(&bom_path, b"\xEF\xBB\xBF        bom\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![
                    missing_path.as_os_str().to_os_string(),
                    plain_path.as_os_str().to_os_string(),
                    bom_path.as_os_str().to_os_string(),
                ],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            let outcome = unexpand_exe(&flags, &mut output).unwrap();

            assert_eq!(output, b"\tplain\n");
            assert_eq!(outcome.exit_code, 1);
            assert_eq!(
                outcome.stderr_text,
                format!(
                    "unexpand: {}: No such file or directory\nunexpand: combination of files with and without BOM header: No such file or directory\n",
                    missing_path.display()
                )
            );
        }

        #[test]
        fn test_unexpand_exe_with_utf8_characters() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), "    Hello 世界\n".as_bytes()).unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().as_os_str().to_os_string()],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: true,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "\tHello 世界\n");
        }

        #[test]
        fn test_unexpand_exe_with_backspaces() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), b"Hello\n\nWorld\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().as_os_str().to_os_string()],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "Hello\n\nWorld\n");
        }

        #[test]
        fn test_unexpand_exe_with_no_files() {
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "");
        }

        #[test]
        fn test_unexpand_exe_with_long_line_across_chunks_keeps_column_state() {
            let file = NamedTempFile::new().unwrap();
            let mut input = vec![b' '; 70_000];
            input.extend_from_slice(b"X\n");
            write(file.path(), &input).unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().as_os_str().to_os_string()],
                tabstops: vec![3],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let mut expected = vec![b'\t'; 70_000 / 3];
            expected.extend(std::iter::repeat_n(b' ', 70_000 % 3));
            expected.extend_from_slice(b"X\n");

            assert_eq!(output, expected);
        }

        #[test]
        fn test_unexpand_exe_keeps_utf8_character_intact_across_chunks() {
            let file = NamedTempFile::new().unwrap();
            let mut input = b"\xEF\xBB\xBF".to_vec();
            input.extend(std::iter::repeat_n(b' ', 65_531));
            input.extend_from_slice("你      x\n".as_bytes());
            write(file.path(), &input).unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().as_os_str().to_os_string()],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: true,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let mut expected = b"\xEF\xBB\xBF".to_vec();
            expected.extend(std::iter::repeat_n(b'\t', 8_191));
            expected.extend_from_slice(b"   ");
            expected.extend_from_slice("你\t   x\n".as_bytes());

            assert_eq!(output, expected);
        }

        #[test]
        fn test_unexpand_native_semantic_collects_rows_and_metadata() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), b"    alpha\n        beta gamma\n").unwrap();

            let semantic = unexpand_native_semantic(
                vec![
                    OsString::from("unexpand"),
                    OsString::from("-t"),
                    OsString::from("4"),
                    file.path().as_os_str().to_os_string(),
                ]
                .into_iter(),
            )
            .unwrap();

            assert_eq!(semantic.tabstop_mode, UnexpandTabstopMode::None);
            assert_eq!(semantic.tabstops, vec![4]);
            assert!(semantic.all_blanks);
            assert!(semantic.assume_utf8);
            assert_eq!(semantic.classic_text, "\talpha\n\t\tbeta gamma\n");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
            assert_eq!(
                semantic.rows,
                vec![
                    UnexpandRow {
                        row_index: 1,
                        line: "\talpha".into(),
                        has_tabs: true,
                    },
                    UnexpandRow {
                        row_index: 2,
                        line: "\t\tbeta gamma".into(),
                        has_tabs: true,
                    },
                ]
            );
        }

        #[test]
        fn test_unexpand_native_semantic_preserves_directory_error() {
            let temp_dir = tempdir().unwrap();
            let input_dir = temp_dir.path().join("input");
            std::fs::create_dir(&input_dir).unwrap();

            let semantic = unexpand_native_semantic(
                vec![OsString::from("unexpand"), input_dir.as_os_str().into()].into_iter(),
            )
            .unwrap();

            assert!(semantic.rows.is_empty());
            assert_eq!(semantic.classic_text, "");
            assert_eq!(
                semantic.stderr_text,
                format!("unexpand: {}: Is a directory\n", input_dir.display())
            );
            assert_eq!(semantic.exit_code, 1);
        }

        #[test]
        fn test_unexpand_native_semantic_quotes_missing_path_with_space() {
            let temp_dir = tempdir().unwrap();
            let input_path = temp_dir.path().join("missing path");

            let semantic = unexpand_native_semantic(
                vec![
                    OsString::from("unexpand"),
                    input_path.clone().into_os_string(),
                ]
                .into_iter(),
            )
            .unwrap();

            assert_eq!(semantic.classic_text, "");
            assert_eq!(
                semantic.stderr_text,
                format!(
                    "unexpand: '{}': No such file or directory\n",
                    input_path.display()
                )
            );
            assert_eq!(semantic.exit_code, 1);
        }

        #[cfg(unix)]
        #[test]
        fn test_unexpand_native_semantic_reads_non_utf8_path() {
            use std::os::unix::ffi::OsStringExt;

            let temp_dir = tempdir().unwrap();
            let input_path = temp_dir
                .path()
                .join(OsString::from_vec(b"\xFFinput".to_vec()));
            write(&input_path, b"        data\n").unwrap();

            let semantic = unexpand_native_semantic(
                vec![OsString::from("unexpand"), input_path.into_os_string()].into_iter(),
            )
            .unwrap();

            assert_eq!(semantic.classic_text, "\tdata\n");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
        }
    }

    #[cfg(test)]
    mod unexpand_line_tests {
        use std::io::Cursor;

        use super::*;

        #[test]
        fn test_unexpand_line_with_spaces_and_tabs() {
            let mut buf = b"    \tHello".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[4], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "\t\tHello".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_mixed_characters() {
            let mut buf = b"Hello\tWorld".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello\tWorld".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_utf8_characters() {
            let mut buf = "Hello 世界".as_bytes().to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: true,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello 世界".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_matches_gnu_invalid_utf8_tab_boundary() {
            let mut buf = [&b"\xe2\x82"[..], &b"        x\n"[..]].concat();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: true,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None)
                .expect("invalid UTF-8 input must be processed");

            assert_eq!(output.into_inner(), b"\xe2\tx\n");
        }

        #[test]
        fn test_unexpand_line_preserves_invalid_utf8_lead_before_tab_boundary() {
            let mut buf = [&b"\xef"[..], &b"       x\n"[..]].concat();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: true,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None)
                .expect("invalid UTF-8 input must be processed");

            assert_eq!(output.into_inner(), b"\xef       x\n");
        }

        #[test]
        fn test_unexpand_line_collapses_partial_bom_prefix_as_gnu_blank() {
            let mut buf = [&b"\xef\xbb"[..], &b"        x\n"[..]].concat();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: true,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None)
                .expect("partial BOM prefix must be processed");

            assert_eq!(output.into_inner(), b"\tx\n");
        }

        #[test]
        fn test_unexpand_line_preserves_mbfile_invalid_sequence_before_tab() {
            let mut buf = b"\xe2\x82 \t".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: true,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None)
                .expect("invalid UTF-8 input must be processed");

            assert_eq!(output.into_inner(), b"\xe2\x82 \t");
        }

        #[test]
        fn test_unexpand_line_keeps_incomplete_utf8_at_eof() {
            let mut buf = b"\xef".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: true,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None)
                .expect("incomplete UTF-8 at EOF must be preserved");

            assert_eq!(output.into_inner(), b"\xef");
        }

        #[test]
        fn test_unexpand_line_with_backspace() {
            let mut buf = b"Hello\n\nWorld".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None).unwrap();
            // println!("{:?}", output);
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello\n\nWorld".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_lastcol_limit() {
            let mut buf = b"Hello\tWorld".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None).unwrap();
            assert_eq!(output.into_inner(), b"Hello\tWorld");
        }

        #[test]
        fn test_unexpand_line_with_no_tabstops() {
            let mut buf = b"Hello World".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello World".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_initial_whitespace() {
            let mut buf = b"   Hello".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[4], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "   Hello".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_multiple_tabstops() {
            let mut buf = b"       Hello".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4, 8],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[4, 8], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "\t   Hello".to_string()
            );
        }
    }

    #[cfg(test)]
    mod next_char_info_tests {
        use super::*;

        #[test]
        fn test_incomplete_utf8_suffix_len() {
            assert_eq!(unexpand_incomplete_utf8_suffix_len(b"text\xE4"), 1);
            assert_eq!(unexpand_incomplete_utf8_suffix_len(b"text\xE4\xBD"), 2);
            assert_eq!(unexpand_incomplete_utf8_suffix_len("text你".as_bytes()), 0);
            assert_eq!(unexpand_incomplete_utf8_suffix_len(b"text\xE4x"), 0);
        }

        #[test]
        fn test_next_char_info_with_utf8() {
            let buf = "Hello, é!".as_bytes();
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(true, buf, 7);
            assert_eq!(ctype, UnexpandCharType::Other);
            assert_eq!(cwidth, 1);
            assert_eq!(nbytes, 2);
        }

        #[test]
        fn test_next_char_info_gives_non_control_soft_hyphen_width_one() {
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(true, "\u{00AD}".as_bytes(), 0);

            assert_eq!(ctype, UnexpandCharType::Other);
            assert_eq!(cwidth, 1);
            assert_eq!(nbytes, 2);
        }

        #[test]
        fn test_next_char_info_with_ascii_space() {
            let buf = "Hello world".as_bytes();
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(false, buf, 5);
            assert_eq!(ctype, UnexpandCharType::Space);
            assert_eq!(cwidth, 1);
            assert_eq!(nbytes, 1);
        }

        #[test]
        fn test_next_char_info_gives_ascii_control_zero_width_in_byte_mode() {
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(false, b"\r", 0);

            assert_eq!(ctype, UnexpandCharType::Other);
            assert_eq!(cwidth, 0);
            assert_eq!(nbytes, 1);
        }

        #[test]
        fn test_next_char_info_with_ascii_tab() {
            let buf = "Hello\tworld".as_bytes();
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(false, buf, 5);
            assert_eq!(ctype, UnexpandCharType::Tab);
            assert_eq!(cwidth, 0);
            assert_eq!(nbytes, 1);
        }

        #[test]
        fn test_next_char_info_with_backspace() {
            let buf = "Hello\x08world".as_bytes();
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(false, buf, 5);
            assert_eq!(ctype, UnexpandCharType::Backspace);
            assert_eq!(cwidth, 0);
            assert_eq!(nbytes, 1);
        }

        #[test]
        fn test_next_char_info_with_invalid_utf8() {
            let buf = [0xff, 0xfe, 0xfd];
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(true, &buf, 0);
            assert_eq!(ctype, UnexpandCharType::Other);
            assert_eq!(cwidth, 1);
            assert_eq!(nbytes, 1);
        }
    }

    #[cfg(test)]
    mod write_tabs_tests {
        use std::io::Cursor;

        use super::*;

        #[test]
        fn test_unexpand_write_tabs_single_tabstop() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 0, 8, false, true, false);
            assert_eq!(output.into_inner(), b"\t\t");
        }

        #[test]
        fn test_unexpand_write_tabs_multiple_tabstops() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4, 8], 0, 12, false, true, false);
            assert_eq!(output.into_inner(), b"\t\t    ");
        }

        #[test]
        fn test_unexpand_write_tabs_no_tabstops() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[], 0, 8, false, true, false);
            assert_eq!(output.into_inner(), b"        ");
        }

        #[test]
        fn test_unexpand_write_tabs_with_prevtab() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 0, 8, true, true, false);
            assert_eq!(output.into_inner(), b"\t\t");
        }

        #[test]
        fn test_unexpand_write_tabs_with_amode() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 0, 8, false, false, true);
            assert_eq!(output.into_inner(), b"\t\t");
        }

        #[test]
        fn test_unexpand_write_tabs_no_init_no_amode() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 0, 8, false, false, false);
            assert_eq!(output.into_inner(), b"        ");
        }

        #[test]
        fn test_unexpand_write_tabs_col_less_than_scol() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 8, 4, false, true, false);
            assert_eq!(output.into_inner(), b"");
        }

        #[test]
        fn test_unexpand_write_tabs_col_equals_scol() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 4, 4, false, true, false);
            assert_eq!(output.into_inner(), b"");
        }

        #[test]
        fn test_unexpand_write_tabs_col_greater_than_scol() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 2, 4, false, true, false);
            assert_eq!(output.into_inner(), b"\t");
        }
    }

    #[cfg(test)]
    mod next_tabstop_tests {
        use super::*;

        #[test]
        fn test_single_tabstop_before_column() {
            let tabstops = vec![8];
            let col = 3;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(5));
        }

        #[test]
        fn test_single_tabstop_at_column() {
            let tabstops = vec![8];
            let col = 8;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(8));
        }

        #[test]
        fn test_single_tabstop_after_column() {
            let tabstops = vec![8];
            let col = 9;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(7));
        }

        #[test]
        fn test_multiple_tabstops_before_column() {
            let tabstops = vec![4, 8, 12];
            let col = 3;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(1));
        }

        #[test]
        fn test_multiple_tabstops_between_columns() {
            let tabstops = vec![4, 8, 12];
            let col = 5;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(3));
        }

        #[test]
        fn test_multiple_tabstops_at_column() {
            let tabstops = vec![4, 8, 12];
            let col = 8;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(4));
        }

        #[test]
        fn test_multiple_tabstops_after_last() {
            let tabstops = vec![4, 8, 12];
            let col = 13;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), None);
        }

        #[test]
        fn test_empty_tabstops() {
            let tabstops = vec![];
            let col = 5;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), None);
        }

        #[test]
        fn test_column_equal_to_tabstop() {
            let tabstops = vec![4, 8, 12];
            let col = 4;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(4));
        }

        #[test]
        fn test_column_greater_than_all_tabstops() {
            let tabstops = vec![4, 8, 12];
            let col = 15;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), None);
        }

        #[test]
        fn test_column_zero() {
            let tabstops = vec![4, 8, 12];
            let col = 0;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(4));
        }

        #[test]
        fn test_large_column_value() {
            let tabstops = vec![4, 8, 12];
            let col = 100;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), None);
        }
    }

    #[cfg(test)]
    mod unexpand_open_tests {
        use std::fs::File;
        use std::io::{Read, Write};

        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_unexpand_open_with_file() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile.txt");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Test content").unwrap();

            let mut last_input_errno = None;
            let result = unexpand_open(file_path.as_os_str(), &mut last_input_errno, false);
            assert!(result.is_ok());

            let mut reader = result.unwrap();
            let mut content = String::new();
            reader.read_to_string(&mut content).unwrap();
            assert_eq!(content, "Test content\n");
        }

        #[test]
        fn test_unexpand_open_with_stdin() {
            // This test is a bit tricky because it involves stdin,
            // so we won't actually test reading from stdin here
            let mut last_input_errno = None;
            let result = unexpand_open(OsStr::new("-"), &mut last_input_errno, false);
            assert!(result.is_ok());
        }

        #[test]
        fn test_unexpand_open_reports_closed_stdin_as_bad_file_descriptor() {
            let mut last_input_errno = None;

            let error = match unexpand_open(OsStr::new("-"), &mut last_input_errno, true) {
                Ok(_) => panic!("closed stdin must be rejected"),
                Err(error) => error,
            };

            assert_eq!(error.to_string(), "-: Bad file descriptor");
            assert_eq!(last_input_errno, Some(ctcore::libc::EBADF));
        }

        #[cfg(unix)]
        #[test]
        fn test_unexpand_quote_path_escapes_non_utf8_bytes() {
            use std::os::unix::ffi::OsStrExt;

            assert_eq!(
                unexpand_quote_path(OsStr::from_bytes(b"missing-\xFF")),
                "'missing-'$'\\377'"
            );
        }
    }

    #[cfg(test)]
    mod expand_shortcuts_tests {
        use super::*;

        #[test]
        fn test_expand_shortcuts_no_shortcuts() {
            let args = vec![
                "--all".to_string(),
                "file1".to_string(),
                "file2".to_string(),
            ];
            let expected = vec![
                "--all".to_string(),
                "file1".to_string(),
                "file2".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_tabs_shortcut() {
            let args = vec!["-4,8,12".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "file1".to_string(),
                "--tabs=12".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_does_not_add_first_only_with_explicit_tabs() {
            let args = vec!["-4".to_string(), "-t".to_string(), "3".to_string()];
            let expected = vec![
                "-t".to_string(),
                "3".to_string(),
                "--tabs=4".to_string(),
                "--short-tabs".to_string(),
            ];

            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_multiple_shortcuts() {
            let args = vec![
                "-4,8".to_string(),
                "-12,16".to_string(),
                "file1".to_string(),
            ];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=812".to_string(),
                "file1".to_string(),
                "--tabs=16".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_accumulate_terminal_tab_value_across_options() {
            let args = vec![
                "-4,8".to_string(),
                "--tabs=16".to_string(),
                "-9".to_string(),
            ];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=16".to_string(),
                "--tabs=89".to_string(),
                "--short-tabs".to_string(),
            ];

            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_handles_numeric_short_option_cluster() {
            let args = vec!["-a4".to_string()];
            let expected = vec![
                "-a".to_string(),
                "--tabs=4".to_string(),
                "--short-tabs".to_string(),
            ];

            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_preserves_all_option_abbreviation() {
            let args = vec!["-4".to_string(), "--al".to_string()];
            let expected = vec![
                "--al".to_string(),
                "--tabs=4".to_string(),
                "--short-tabs".to_string(),
            ];

            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_all_flag() {
            let args = vec!["-4,8".to_string(), "--all".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--all".to_string(),
                "file1".to_string(),
                "--tabs=8".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_a_flag() {
            let args = vec!["-4,8".to_string(), "-a".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "-a".to_string(),
                "file1".to_string(),
                "--tabs=8".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_empty_input() {
            let args: Vec<String> = vec![];
            let expected: Vec<String> = vec![];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_mixed_args() {
            let args = vec![
                "--all".to_string(),
                "-4,8".to_string(),
                "--some-flag".to_string(),
                "file1".to_string(),
                "-12".to_string(),
            ];
            let expected = vec![
                "--all".to_string(),
                "--tabs=4".to_string(),
                "--some-flag".to_string(),
                "file1".to_string(),
                "--tabs=812".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_handles_comma_and_all_short_options() {
            let args = vec!["-4,a".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "-a".to_string(),
                "file1".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_leading_dash() {
            let args = vec!["file1".to_string(), "-4".to_string()];
            let expected = vec![
                "file1".to_string(),
                "--tabs=4".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_repeated_tabs_shortcut() {
            let args = vec!["-4,8,8,12".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "--tabs=8".to_string(),
                "file1".to_string(),
                "--tabs=12".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_only_shortcuts() {
            let args = vec!["-4,8,12".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "--tabs=12".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_only_files() {
            let args = vec!["file1".to_string(), "file2".to_string()];
            let expected = vec!["file1".to_string(), "file2".to_string()];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_empty_tabs_shortcut() {
            let args = vec!["-4,,8".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "file1".to_string(),
                "--tabs=8".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_tabs_and_other_flags() {
            let args = vec![
                "-4,8".to_string(),
                "--no-utf8".to_string(),
                "file1".to_string(),
            ];
            let expected = vec![
                "--tabs=4".to_string(),
                "--no-utf8".to_string(),
                "file1".to_string(),
                "--tabs=8".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_only_flags() {
            let args = vec!["--all".to_string(), "--no-utf8".to_string()];
            let expected = vec!["--all".to_string(), "--no-utf8".to_string()];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_preserves_numeric_filename_after_double_dash() {
            let args = vec!["-4".to_string(), "--".to_string(), "-4".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
                "--".to_string(),
                "-4".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_inserts_legacy_options_before_posix_operands() {
            let args = [
                OsString::from("unexpand"),
                OsString::from("-4"),
                OsString::from("input"),
                OsString::from("-8"),
            ];
            let expected = [
                OsString::from("unexpand"),
                OsString::from("--tabs=4"),
                OsString::from("--first-only"),
                OsString::from("--short-tabs"),
                OsString::from("input"),
                OsString::from("-8"),
            ];

            assert_eq!(expand_shortcuts_os(&args, true), expected);
        }

        #[test]
        fn test_expand_shortcuts_keeps_tabs_value_before_posix_operand_detection() {
            let args = [
                OsString::from("unexpand"),
                OsString::from("-t"),
                OsString::from("4"),
                OsString::from("-4"),
            ];
            let expected = [
                OsString::from("unexpand"),
                OsString::from("-t"),
                OsString::from("4"),
                OsString::from("--tabs=4"),
                OsString::from("--short-tabs"),
            ];

            assert_eq!(expand_shortcuts_os(&args, true), expected);

            let reverse_args = [
                OsString::from("unexpand"),
                OsString::from("-4"),
                OsString::from("-t"),
                OsString::from("4"),
            ];
            let reverse_expected = [
                OsString::from("unexpand"),
                OsString::from("-t"),
                OsString::from("4"),
                OsString::from("--tabs=4"),
                OsString::from("--short-tabs"),
            ];

            assert_eq!(expand_shortcuts_os(&reverse_args, true), reverse_expected);
        }

        #[test]
        fn test_expand_shortcuts_preserves_stdin_file_marker() {
            let args = vec!["regular".to_string(), "-".to_string()];
            assert_eq!(expand_shortcuts(&args), args);
        }
    }

    #[cfg(test)]
    mod is_digit_or_comma_tests {
        use super::*;

        #[test]
        fn test_is_digit_or_comma() {
            assert!(is_digit_or_comma('0'));
            assert!(is_digit_or_comma('1'));
            assert!(is_digit_or_comma('2'));
            assert!(is_digit_or_comma('3'));
            assert!(is_digit_or_comma('4'));
            assert!(is_digit_or_comma('5'));
            assert!(is_digit_or_comma('6'));
            assert!(is_digit_or_comma('7'));
            assert!(is_digit_or_comma('8'));
            assert!(is_digit_or_comma('9'));
            assert!(is_digit_or_comma(','));
            assert!(!is_digit_or_comma('a'));
            assert!(!is_digit_or_comma('A'));
            assert!(!is_digit_or_comma('!'));
            assert!(!is_digit_or_comma('('));
            assert!(!is_digit_or_comma(')'));
        }
    }

    #[cfg(test)]
    mod unexpand_flags_tests {
        use super::*;

        #[test]
        fn test_unexpand_flags_new_default() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.tabstops, vec![UNEXPAND_DEFAULT_TABSTOP]);
            assert_eq!(flags.remaining_mode, RemainingMode::None);
            assert_eq!(flags.files, vec![OsString::from("-")]);
            assert!(!flags.is_a_flag);
            assert!(flags.is_u_flag);
        }

        #[test]
        fn test_unexpand_flags_new_with_tabs() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,8,12"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.tabstops, vec![4, 8, 12]);
            assert_eq!(flags.remaining_mode, RemainingMode::None);
        }

        #[test]
        fn test_unexpand_flags_keeps_extension_across_tabs_options() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "-t", "/3", "-t", "4,8"]);
            let flags = UnexpandFlags::new(&matches).unwrap();

            assert_eq!(flags.tabstops, vec![4, 8, 3]);
            assert_eq!(flags.remaining_mode, RemainingMode::Slash);
            assert!(flags.is_a_flag);
        }

        #[test]
        fn test_unexpand_flags_accepts_zero_extension_size() {
            let app = ct_app();

            let default_tabs = app.clone().get_matches_from(vec!["unexpand", "-t", "/0"]);
            let flags = UnexpandFlags::new(&default_tabs).unwrap();
            assert_eq!(flags.tabstops, vec![UNEXPAND_DEFAULT_TABSTOP]);
            assert_eq!(flags.remaining_mode, RemainingMode::None);

            let explicit_tabs = app.get_matches_from(vec!["unexpand", "-t", "4,8,+0"]);
            let flags = UnexpandFlags::new(&explicit_tabs).unwrap();
            assert_eq!(flags.tabstops, vec![4, 8]);
            assert_eq!(flags.remaining_mode, RemainingMode::None);

            let cross_option_tabs =
                ct_app().get_matches_from(vec!["unexpand", "-t", "/0", "-t", "4,8"]);
            let flags = UnexpandFlags::new(&cross_option_tabs).unwrap();
            assert_eq!(flags.tabstops, vec![4, 8]);
            assert_eq!(flags.remaining_mode, RemainingMode::None);
        }

        #[test]
        fn test_unexpand_flags_allows_extension_after_zero_extension() {
            let app = ct_app();

            let default_tabs = app
                .clone()
                .get_matches_from(vec!["unexpand", "-t", "/0,+3"]);
            let flags = UnexpandFlags::new(&default_tabs).unwrap();
            assert_eq!(flags.tabstops, vec![3]);
            assert_eq!(flags.remaining_mode, RemainingMode::None);

            let explicit_tabs = app.get_matches_from(vec!["unexpand", "-t", "4,/3,+0"]);
            let flags = UnexpandFlags::new(&explicit_tabs).unwrap();
            assert_eq!(flags.tabstops, vec![4, 3]);
            assert_eq!(flags.remaining_mode, RemainingMode::Slash);
        }

        #[test]
        fn test_unexpand_flags_rejects_zero_slash_extension_after_nonzero_slash_extension() {
            let matches = ct_app().get_matches_from(vec!["unexpand", "-t", "/3", "-t", "/0"]);

            assert!(matches!(
                UnexpandFlags::new(&matches),
                Err(UnexpandParseError::SpecifierOnlyAllowedWithLastValue(specifier)) if specifier == "/"
            ));
        }

        #[test]
        fn test_unexpand_flags_rejects_zero_plus_extension_after_nonzero_plus_extension() {
            let matches = ct_app().get_matches_from(vec!["unexpand", "-t", "+3", "-t", "+0"]);

            assert!(matches!(
                UnexpandFlags::new(&matches),
                Err(UnexpandParseError::SpecifierOnlyAllowedWithLastValue(specifier)) if specifier == "+"
            ));
        }

        #[test]
        fn test_unexpand_flags_uses_last_extension_prefix_before_number() {
            let app = ct_app();

            let slash_tabs = app
                .clone()
                .get_matches_from(vec!["unexpand", "-t", "4,+/3"]);
            let flags = UnexpandFlags::new(&slash_tabs).unwrap();
            assert_eq!(flags.tabstops, vec![4, 3]);
            assert_eq!(flags.remaining_mode, RemainingMode::Slash);

            let plus_tabs = app.get_matches_from(vec!["unexpand", "-t", "4,/+3"]);
            let flags = UnexpandFlags::new(&plus_tabs).unwrap();
            assert_eq!(flags.tabstops, vec![4, 3]);
            assert_eq!(flags.remaining_mode, RemainingMode::Plus);
        }

        #[test]
        fn test_unexpand_flags_rejects_distinct_nonzero_extension_modes() {
            let matches = ct_app().get_matches_from(vec!["unexpand", "-t", "4,/3,+4"]);

            assert!(matches!(
                UnexpandFlags::new(&matches),
                Err(UnexpandParseError::SpecifierMutuallyExclusive)
            ));
        }

        #[test]
        fn test_unexpand_flags_new_with_all_flag() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--all"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert!(flags.is_a_flag);
        }

        #[test]
        fn test_unexpand_flags_new_with_first_only_flag() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--first-only"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert!(!flags.is_a_flag);
        }

        #[test]
        fn test_unexpand_flags_accept_repeated_boolean_options() {
            let repeated_all = ct_app()
                .try_get_matches_from(["unexpand", "-a", "--all"])
                .expect("GNU accepts repeated --all options");
            assert!(UnexpandFlags::new(&repeated_all).unwrap().is_a_flag);

            let repeated_first_only = ct_app()
                .try_get_matches_from(["unexpand", "--first-only", "--first-only"])
                .expect("GNU accepts repeated --first-only options");
            assert!(!UnexpandFlags::new(&repeated_first_only).unwrap().is_a_flag);
        }

        #[test]
        fn test_unexpand_flags_new_with_no_utf8_flag() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--no-utf8"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert!(!flags.is_u_flag);
        }

        #[test]
        fn test_unexpand_flags_new_with_files() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "file1", "file2"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(
                flags.files,
                vec![OsString::from("file1"), OsString::from("file2")]
            );
        }

        #[test]
        fn test_unexpand_flags_new_with_invalid_tabstops() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,x,12"]);
            let result = UnexpandFlags::new(&matches);
            assert!(result.is_err());
            assert_eq!(
                result.err(),
                Some(UnexpandParseError::InvalidCharacter("x,12".to_string()))
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_unexpand_flags_preserves_non_utf8_tabstop_diagnostic_bytes() {
            use std::os::unix::ffi::OsStringExt;

            let matches = ct_app()
                .try_get_matches_from([
                    OsString::from("unexpand"),
                    OsString::from("-t"),
                    OsString::from_vec(vec![0xff]),
                ])
                .expect("raw tab-stop bytes must reach the unexpand parser");
            let error = match UnexpandFlags::new(&matches) {
                Err(error) => error,
                Ok(_) => panic!("non-UTF-8 tab-stop must be rejected"),
            };

            assert!(matches!(
                error.diagnostic_bytes().as_ref(),
                b"tab size contains invalid character(s): '\\377'"
                    | b"tab size contains invalid character(s): \xe2\x80\x98\\377\xe2\x80\x99"
            ));
        }

        #[test]
        fn test_unexpand_flags_new_with_zero_tabstops() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,0,12"]);
            let result = UnexpandFlags::new(&matches);
            assert!(result.is_err());
            assert_eq!(result.err(), Some(UnexpandParseError::TabSizeCannotBeZero));
        }

        #[test]
        fn test_unexpand_flags_new_with_non_ascending_tabstops() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,12,8"]);
            let result = UnexpandFlags::new(&matches);
            assert!(result.is_err());
            assert_eq!(
                result.err(),
                Some(UnexpandParseError::TabSizesMustBeAscending)
            );
        }

        #[test]
        fn test_unexpand_flags_new_with_too_large_tabstops() {
            let app = ct_app();
            let matches =
                app.get_matches_from(vec!["unexpand", "--tabs", "4,999999999999999999999,12"]);
            let result = UnexpandFlags::new(&matches);
            assert!(result.is_err());
            assert_eq!(
                result.err(),
                Some(UnexpandParseError::TabStopTooLarge(
                    "999999999999999999999".to_string()
                ))
            );
        }
    }
    #[test]
    fn test_unexpand_flags_new_with_combined_flags() {
        let app = ct_app();
        let matches = app.get_matches_from(vec![
            "unexpand",
            "--tabs",
            "4,8",
            "--all",
            "--no-utf8",
            "file1",
            "file2",
        ]);
        let flags = UnexpandFlags::new(&matches).unwrap();
        assert_eq!(flags.tabstops, vec![4, 8]);
        assert_eq!(flags.files, vec!["file1".to_string(), "file2".to_string()]);
        assert_eq!(flags.is_a_flag, true);
        assert_eq!(flags.is_u_flag, false);
    }

        #[test]
        fn test_unexpand_flags_new_with_combined_flags() {
            let app = ct_app();
            let matches = app.get_matches_from(vec![
                "unexpand",
                "--tabs",
                "4,8",
                "--all",
                "--no-utf8",
                "file1",
                "file2",
            ]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.tabstops, vec![4, 8]);
            assert_eq!(
                flags.files,
                vec![OsString::from("file1"), OsString::from("file2")]
            );
            assert!(flags.is_a_flag);
            assert!(!flags.is_u_flag);
        }

        #[test]
        fn test_unexpand_flags_new_with_tabs_and_all_but_not_first_only() {
            let app = ct_app();
            let matches = app.get_matches_from(vec![
                "unexpand",
                "--tabs",
                "4,8",
                "--all",
                "--first-only",
                "file1",
                "file2",
            ]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.tabstops, vec![4, 8]);
            assert_eq!(
                flags.files,
                vec![OsString::from("file1"), OsString::from("file2")]
            );
            assert!(!flags.is_a_flag);
        }

        #[test]
        fn test_unexpand_flags_new_with_tabs_and_default_flags() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,8"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.tabstops, vec![4, 8]);
            assert_eq!(flags.files, vec![OsString::from("-")]);
            assert!(flags.is_a_flag);
            assert!(flags.is_u_flag);
        }

        #[test]
        fn test_unexpand_flags_new_with_all_flags() {
            let app = ct_app();
            let matches = app.get_matches_from(vec![
                "unexpand",
                "--tabs",
                "4,8,12",
                "--all",
                "--first-only",
                "--no-utf8",
                "file1",
                "file2",
            ]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.tabstops, vec![4, 8, 12]);
            assert_eq!(
                flags.files,
                vec![OsString::from("file1"), OsString::from("file2")]
            );
            assert!(!flags.is_a_flag); // Because --first-only is present
            assert!(!flags.is_u_flag);
        }

        #[test]
        fn test_unexpand_flags_new_with_default_file() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,8"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.files, vec![OsString::from("-")]);
        }
    }

    #[cfg(test)]
    mod tabstops_parse_tests {
        use super::*;

        #[test]
        fn test_unexpand_tabstops_parse_valid_input() {
            let input = "1,2,3,4,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_allows_smaller_extension_size() {
            assert_eq!(
                unexpand_tabstops_parse("4,8,/3", false),
                Ok((RemainingMode::Slash, vec![4, 8, 3]))
            );
            assert_eq!(
                unexpand_tabstops_parse("4,8,+3", false),
                Ok((RemainingMode::Plus, vec![4, 8, 3]))
            );
        }

        #[test]
        fn test_unexpand_blank_classification_excludes_non_blank_whitespace() {
            assert!(is_blank_char(' '));
            assert!(!is_blank_char('\u{000B}'));
            assert!(!is_blank_char('\u{000C}'));
            assert!(!is_blank_char('\u{00A0}'));
        }

        #[test]
        fn test_unexpand_tabstops_parse_invalid_character() {
            let input = "1,2,x,4,5";
            let expected = Err(UnexpandParseError::InvalidCharacter("x,4,5".to_string()));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_reports_all_errors_before_invalid_character() {
            let error = unexpand_tabstops_parse("3/x", false)
                .expect_err("GNU reports the misplaced specifier and invalid character");
            let diagnostic = error.diagnostic_bytes();

            assert!(
                diagnostic
                    .as_ref()
                    .starts_with(b"'/' specifier not at start of number: '/x'\n")
            );
            assert!(
                diagnostic
                    .as_ref()
                    .ends_with(b": tab size contains invalid character(s): 'x'")
            );
        }

        #[test]
        fn test_unexpand_diagnostic_quote_uses_utf8_quote_marks() {
            assert_eq!(
                unexpand_quote_diagnostic_argument_with_quote_marks(
                    b"/x",
                    b"\xe2\x80\x98",
                    b"\xe2\x80\x99"
                ),
                b"\xe2\x80\x98/x\xe2\x80\x99"
            );
        }

        #[test]
        fn test_unexpand_tabstops_parse_skips_terminal_value_after_a_prior_error() {
            let error = unexpand_tabstops_parse("3/4,5", false)
                .expect_err("the misplaced slash must be reported");

            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"'/' specifier not at start of number: '/4,5'"
            );
        }

        #[test]
        fn test_unexpand_tabstops_parse_zero_value() {
            let input = "1,2,0,4,5";
            let expected = Err(UnexpandParseError::TabSizeCannotBeZero);
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_non_ascending_values() {
            let input = "1,3,2,4,5";
            let expected = Err(UnexpandParseError::TabSizesMustBeAscending);
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_too_large_value() {
            let input = "1,2,99999999999999999999999999,4,5";
            let expected = Err(UnexpandParseError::TabStopTooLarge(
                "99999999999999999999999999".to_string(),
            ));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_empty_input() {
            let input = "";
            let expected = Ok((RemainingMode::None, vec![UNEXPAND_DEFAULT_TABSTOP]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_single_value() {
            let input = "5";
            let expected = Ok((RemainingMode::None, vec![5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_trailing_comma() {
            let input = "1,2,3,4,5,";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_leading_comma() {
            let input = ",1,2,3,4,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_multiple_commas() {
            let input = "1,,2,3,4,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_spaces_in_values() {
            let input = "1, 2,3, 4,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_tabs_in_values() {
            let input = "4\t8";
            let expected = Ok((RemainingMode::None, vec![4, 8]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_spaces_around_commas() {
            let input = "1 ,2 ,3 ,4 ,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_mixed_invalid_characters() {
            let input = "1,2,3,a4,5";
            let expected = Err(UnexpandParseError::InvalidCharacter("a4,5".to_string()));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_with_leading_zeros() {
            let input = "01,02,03,04,05";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_large_numbers() {
            let input = "1,1000000000,2000000000,3000000000,4000000000";
            let expected = Ok((
                RemainingMode::None,
                vec![1, 1000000000, 2000000000, 3000000000, 4000000000],
            ));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }
    }

    #[cfg(test)]
    mod parse_error_tests {
        use super::*;

        #[test]
        fn test_invalid_character_display() {
            let error = UnexpandParseError::InvalidCharacter("x".to_string());
            assert_eq!(
                format!("{error}"),
                "tab size contains invalid character(s): 'x'"
            );
        }

        #[test]
        fn test_tab_size_cannot_be_zero_display() {
            let error = UnexpandParseError::TabSizeCannotBeZero;
            assert_eq!(format!("{error}"), "tab size cannot be 0");
        }

        #[test]
        fn test_tab_size_too_large_display() {
            let error = UnexpandParseError::TabStopValueTooLarge;
            assert_eq!(format!("{error}"), "tab stop value is too large");
        }

        #[test]
        fn test_tab_sizes_must_be_ascending_display() {
            let error = UnexpandParseError::TabSizesMustBeAscending;
            assert_eq!(format!("{error}"), "tab sizes must be ascending");
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use std::fs;

        use clap::error::ErrorKind;

        use crate::is_digit_or_comma;

        use super::*;

        #[test]
        fn test_is_digit_or_comma() {
            assert!(is_digit_or_comma('1'));
            assert!(is_digit_or_comma(','));
            assert!(!is_digit_or_comma('a'));
        }

        // unexpand 接口测试: unexpand [OPTION]... [FILE]...
        //   -a, --all             convert all blanks, instead of just initial blanks
        //       --first-only      convert only leading sequences of blanks (overrides -a)
        //   -t, --tabs <N, LIST>  use comma separated LIST of tab positions or have tabs N characters apart instead of 8 (enables -a)
        //   -U, --no-utf8         interpret input file as 8-bit ASCII rather than UTF-8
        //   -h, --help            Print help
        //   -V, --version         Print version
        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "--version"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "-V"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            // 测试用例3：验证当提供未知参数时是否正确报错
            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            // 测试用例4：验证当缺少必需的参数时是否正确报错
            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_file() {
            // Create a regular file for testing , 默认带文件
            let regular_file_path = "test_file";
            File::create(regular_file_path).expect("Failed to create regular file");

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), regular_file_path];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());

            // Clean up: remove the regular file after the test
            fs::remove_file(regular_file_path).expect("Failed to remove regular file");
        }

        #[test]
        fn test_ct_app_long_option_all() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--all"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_first_only() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--first-only"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_tabs() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--tabs", "N, LIST"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_no_utf8() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--no-utf8"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_posix_mode_treats_late_option_as_file() {
            let matches = ct_app_with_posix_mode(true)
                .try_get_matches_from(["unexpand", "input", "-a"])
                .unwrap();

            assert_eq!(matches.get_count(unexpand_flags::ALL), 0);
            assert_eq!(
                matches
                    .get_many::<OsString>(unexpand_flags::FILE)
                    .unwrap()
                    .map(OsString::as_os_str)
                    .collect::<Vec<_>>(),
                [OsStr::new("input"), OsStr::new("-a")]
            );
        }

        #[test]
        fn test_ct_app_short_option_a() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-a"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_t() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-t", "N, LIST"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_uppercase_u() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-U"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_unexpand_restores_default_sigpipe_only_for_default_callers() {
            let restored = std::cell::Cell::new(false);
            unexpand_restore_default_sigpipe_if_needed(false, || restored.set(true));
            assert!(!restored.get());

            unexpand_restore_default_sigpipe_if_needed(true, || restored.set(true));
            assert!(restored.get());
        }
    }
}
