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

//! sleep 命令用于使当前进程暂停执行一段时间。这个时间可以是以秒为单位的整数或浮点数。

extern crate rust_i18n;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::time::Duration;

use clap::{Arg, ArgAction, Command, builder::OsStringValueParser, crate_version};

use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult};
use ctcore::ct_format::num_parser::{ParseError, ParsedNumber};
use ctcore::ct_posix::GnuGetoptCommandExt;
use std::borrow::Cow;
use std::error::Error;
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::ffi::{CStr, OsStr, OsString};
use std::fmt::{Display, Formatter};
#[cfg(target_os = "linux")]
use std::io;
#[cfg(not(target_os = "linux"))]
use std::thread;

mod sleep_flags {
    pub const SLEEP_NUMBER: &str = "NUMBER";
}

#[cfg(target_os = "linux")]
// GNU's "\xa1\ae" source literal expands \a as BEL and leaves the trailing e.
const GNU_GB18030_FALLBACK_LEFT_QUOTE: &[u8] = b"\xa1\x07e";
#[cfg(target_os = "linux")]
const GNU_GB18030_FALLBACK_RIGHT_QUOTE: &[u8] = b"\xa1\xaf";

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn mbrtowc(
        wide: *mut ctcore::libc::wchar_t,
        bytes: *const ctcore::libc::c_char,
        length: usize,
        state: *mut ctcore::libc::mbstate_t,
    ) -> usize;
    fn iswprint(wide: ctcore::libc::c_uint) -> ctcore::libc::c_int;
    fn iswprint_l(
        wide: ctcore::libc::c_uint,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
}

#[derive(Debug)]
struct InvalidTimeIntervalError {
    operands: Vec<OsString>,
}

impl Display for InvalidTimeIntervalError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&localized_invalid_time_interval(&rust_i18n::locale()))
    }
}

impl Error for InvalidTimeIntervalError {}

fn quote_duration_operand_for_locale(operand: &OsStr, locale: &str) -> Vec<u8> {
    #[cfg(target_os = "linux")]
    let codeset = sleep_output_codeset();

    #[cfg(not(target_os = "linux"))]
    let codeset = None::<String>;

    quote_duration_operand_for_locale_with_codeset(operand, locale, codeset.as_deref())
}

fn quote_duration_operand_for_locale_with_codeset(
    operand: &OsStr,
    locale: &str,
    codeset: Option<&str>,
) -> Vec<u8> {
    #[cfg(target_os = "linux")]
    if let Some(codeset) = codeset {
        let (opening_quote, closing_quote) = if locale == "zh-CN" {
            (b"\"".as_slice(), b"\"".as_slice())
        } else if sleep_gb18030_codeset(codeset) {
            (
                GNU_GB18030_FALLBACK_LEFT_QUOTE,
                GNU_GB18030_FALLBACK_RIGHT_QUOTE,
            )
        } else {
            (b"'".as_slice(), b"'".as_slice())
        };
        return quote_duration_operand_with_locale_encoding(
            operand,
            codeset,
            opening_quote,
            closing_quote,
        );
    }

    #[cfg(not(target_os = "linux"))]
    let _ = codeset;

    if locale == "zh-CN" {
        return quote_duration_operand_with_quote_marks(operand, true, b"\"", b"\"", Some(b'"'));
    }

    quote_duration_operand_with_style(operand, locale_uses_utf8_quotes())
}

#[cfg(target_os = "linux")]
fn sleep_gb18030_codeset(codeset: &str) -> bool {
    codeset.eq_ignore_ascii_case("GB18030")
}

fn localized_invalid_time_interval(locale: &str) -> String {
    t!("sleep.errors.invalid_time_interval", locale = locale).to_string()
}

#[cfg(target_os = "linux")]
fn localized_cannot_read_realtime_clock(locale: &str) -> String {
    t!("sleep.errors.cannot_read_realtime_clock", locale = locale).to_string()
}

fn locale_uses_utf8_quotes() -> bool {
    // SAFETY: setlocale with a null locale argument only returns the current
    // process-owned locale name, which is copied before it can be changed.
    unsafe {
        let locale = ctcore::libc::setlocale(ctcore::libc::LC_CTYPE, std::ptr::null());
        (!locale.is_null())
            .then(|| {
                CStr::from_ptr(locale)
                    .to_string_lossy()
                    .to_ascii_uppercase()
            })
            .is_some_and(|locale| locale.contains("UTF-8") || locale.contains("UTF8"))
    }
}

fn quote_duration_operand_with_style(operand: &OsStr, utf8_locale: bool) -> Vec<u8> {
    let (opening_quote, closing_quote) = if utf8_locale {
        ("‘".as_bytes(), "’".as_bytes())
    } else {
        (b"'".as_slice(), b"'".as_slice())
    };
    quote_duration_operand_with_quote_marks(
        operand,
        utf8_locale,
        opening_quote,
        closing_quote,
        (!utf8_locale).then_some(b'\''),
    )
}

fn quote_duration_operand_with_quote_marks(
    operand: &OsStr,
    utf8_locale: bool,
    opening_quote: &[u8],
    closing_quote: &[u8],
    quote_to_escape: Option<u8>,
) -> Vec<u8> {
    let bytes = operand.as_encoded_bytes();

    let mut quoted = Vec::with_capacity(bytes.len() + opening_quote.len() + closing_quote.len());
    quoted.extend_from_slice(opening_quote);
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii() {
            push_duration_operand_quoted_ascii(&mut quoted, byte, quote_to_escape);
        } else if utf8_locale {
            match std::str::from_utf8(&bytes[index..]) {
                Ok(_) => {
                    push_duration_operand_utf8_prefix(
                        &mut quoted,
                        &bytes[index..],
                        closing_quote,
                        quote_to_escape,
                    );
                    break;
                }
                Err(error) if error.valid_up_to() > 0 => {
                    let end = index + error.valid_up_to();
                    push_duration_operand_utf8_prefix(
                        &mut quoted,
                        &bytes[index..end],
                        closing_quote,
                        quote_to_escape,
                    );
                    index = end;
                    continue;
                }
                Err(error) => {
                    let invalid_length = error.error_len().unwrap_or(bytes.len() - index);
                    for invalid_byte in &bytes[index..index + invalid_length] {
                        push_duration_operand_octal_escape(&mut quoted, *invalid_byte);
                    }
                    index += invalid_length;
                    continue;
                }
            }
        } else {
            push_duration_operand_octal_escape(&mut quoted, byte);
        }
        index += 1;
    }
    quoted.extend_from_slice(closing_quote);
    quoted
}

fn push_duration_operand_utf8_prefix(
    output: &mut Vec<u8>,
    input: &[u8],
    closing_quote: &[u8],
    quote_to_escape: Option<u8>,
) {
    let text = std::str::from_utf8(input).expect("input is a valid UTF-8 prefix");
    for (index, character) in text.char_indices() {
        if character.is_ascii() {
            push_duration_operand_quoted_ascii(output, character as u8, quote_to_escape);
            continue;
        }
        let character_len = character.len_utf8();
        let encoded = &input[index..index + character_len];
        if duration_utf8_character_is_printable(character) {
            if encoded == closing_quote {
                output.push(b'\\');
            }
            output.extend_from_slice(encoded);
        } else {
            for byte in encoded {
                push_duration_operand_octal_escape(output, *byte);
            }
        }
    }
}

fn duration_utf8_character_is_printable(character: char) -> bool {
    #[cfg(target_os = "linux")]
    {
        // The caller already established a UTF-8 locale.  Use a dedicated
        // UTF-8 locale here so unit-level quoting remains independent of the
        // process-wide locale changed by other tests.
        unsafe {
            let locale = ctcore::libc::newlocale(
                ctcore::libc::LC_CTYPE_MASK,
                c"C.UTF-8".as_ptr(),
                std::ptr::null_mut(),
            );
            if locale.is_null() {
                return duration_decoded_character_is_printable(character);
            }
            let printable = iswprint_l(character as ctcore::libc::c_uint, locale) != 0;
            ctcore::libc::freelocale(locale);
            printable
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        !character.is_control()
    }
}

#[cfg(target_os = "linux")]
fn quote_duration_operand_with_locale_encoding(
    operand: &OsStr,
    codeset: &str,
    opening_quote: &[u8],
    closing_quote: &[u8],
) -> Vec<u8> {
    let input = operand.as_encoded_bytes();
    let mut quoted = Vec::with_capacity(input.len() + opening_quote.len() + closing_quote.len());
    quoted.extend_from_slice(opening_quote);

    let mut index = 0;
    while index < input.len() {
        if input[index].is_ascii() {
            push_duration_operand_quoted_ascii(
                &mut quoted,
                input[index],
                (closing_quote.len() == 1).then_some(closing_quote[0]),
            );
            index += 1;
            continue;
        }

        match duration_locale_character(&input[index..], codeset) {
            DurationLocaleCharacter::Printable(character_len) => {
                let character = &input[index..index + character_len];
                if character == closing_quote {
                    quoted.push(b'\\');
                }
                quoted.extend_from_slice(character);
                index += character_len;
            }
            DurationLocaleCharacter::Nonprinting(character_len) => {
                for byte in &input[index..index + character_len] {
                    push_duration_operand_octal_escape(&mut quoted, *byte);
                }
                index += character_len;
            }
            DurationLocaleCharacter::Invalid => {
                push_duration_operand_octal_escape(&mut quoted, input[index]);
                index += 1;
            }
            DurationLocaleCharacter::Incomplete => {
                for byte in &input[index..] {
                    push_duration_operand_octal_escape(&mut quoted, *byte);
                }
                break;
            }
        }
    }

    quoted.extend_from_slice(closing_quote);
    quoted
}

#[cfg(target_os = "linux")]
enum DurationLocaleCharacter {
    Printable(usize),
    Nonprinting(usize),
    Invalid,
    Incomplete,
}

#[cfg(target_os = "linux")]
enum DurationLocaleDecodeError {
    Incomplete,
    Invalid,
}

#[cfg(target_os = "linux")]
fn duration_locale_character(input: &[u8], codeset: &str) -> DurationLocaleCharacter {
    if sleep_current_codeset_matches(codeset) {
        return duration_current_locale_character(input);
    }

    for length in 1..=input.len().min(4) {
        if let Ok(decoded) = duration_locale_bytes_to_utf8(&input[..length], codeset) {
            let printable = std::str::from_utf8(&decoded)
                .ok()
                .is_some_and(|text| text.chars().all(duration_decoded_character_is_printable));
            return if printable {
                DurationLocaleCharacter::Printable(length)
            } else {
                DurationLocaleCharacter::Nonprinting(length)
            };
        }
    }
    match duration_locale_bytes_to_utf8(input, codeset) {
        Err(DurationLocaleDecodeError::Incomplete) => DurationLocaleCharacter::Incomplete,
        Ok(_) | Err(DurationLocaleDecodeError::Invalid) => DurationLocaleCharacter::Invalid,
    }
}

#[cfg(target_os = "linux")]
fn sleep_current_codeset_matches(codeset: &str) -> bool {
    unsafe {
        let current = ctcore::libc::nl_langinfo(ctcore::libc::CODESET);
        (!current.is_null()).then(|| CStr::from_ptr(current).to_bytes() == codeset.as_bytes())
    }
    .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn duration_current_locale_character(input: &[u8]) -> DurationLocaleCharacter {
    unsafe {
        let mut state: ctcore::libc::mbstate_t = std::mem::zeroed();
        let mut wide = 0 as ctcore::libc::wchar_t;
        let length = mbrtowc(&mut wide, input.as_ptr().cast(), input.len(), &mut state);
        if length == usize::MAX {
            return DurationLocaleCharacter::Invalid;
        }
        if length == usize::MAX - 1 {
            return DurationLocaleCharacter::Incomplete;
        }

        let length = if length == 0 { 1 } else { length };
        if iswprint(wide as ctcore::libc::c_uint) != 0 {
            DurationLocaleCharacter::Printable(length)
        } else {
            DurationLocaleCharacter::Nonprinting(length)
        }
    }
}

#[cfg(target_os = "linux")]
fn duration_decoded_character_is_printable(character: char) -> bool {
    let scalar = character as u32;
    !character.is_control()
        && !(0xfdd0..=0xfdef).contains(&scalar)
        && scalar & 0xffff != 0xfffe
        && scalar & 0xffff != 0xffff
}

#[cfg(target_os = "linux")]
fn duration_locale_bytes_to_utf8(
    input: &[u8],
    codeset: &str,
) -> Result<Vec<u8>, DurationLocaleDecodeError> {
    let source = match CString::new(codeset) {
        Ok(source) => source,
        Err(_) => return Err(DurationLocaleDecodeError::Invalid),
    };
    let target = CString::new("UTF-8").expect("UTF-8 has no NUL byte");
    let converter = unsafe { ctcore::libc::iconv_open(target.as_ptr(), source.as_ptr()) };
    if converter == (-1_isize) as ctcore::libc::iconv_t {
        return Err(DurationLocaleDecodeError::Invalid);
    }

    let mut input_ptr = input.as_ptr().cast_mut().cast::<ctcore::libc::c_char>();
    let mut input_left = input.len();
    let mut output = vec![0_u8; input.len().saturating_mul(4).max(16)];
    let mut output_ptr = output.as_mut_ptr().cast::<ctcore::libc::c_char>();
    let mut output_left = output.len();
    let result = unsafe {
        ctcore::libc::iconv(
            converter,
            &mut input_ptr,
            &mut input_left,
            &mut output_ptr,
            &mut output_left,
        )
    };
    let incomplete = result == usize::MAX
        && unsafe { *ctcore::libc::__errno_location() } == ctcore::libc::EINVAL;
    unsafe { ctcore::libc::iconv_close(converter) };
    if result == usize::MAX || input_left != 0 {
        return Err(if incomplete {
            DurationLocaleDecodeError::Incomplete
        } else {
            DurationLocaleDecodeError::Invalid
        });
    }
    output.truncate(output.len() - output_left);
    Ok(output)
}

fn push_duration_operand_quoted_ascii(output: &mut Vec<u8>, byte: u8, quote_to_escape: Option<u8>) {
    match byte {
        b'\x07' => output.extend_from_slice(b"\\a"),
        b'\x08' => output.extend_from_slice(b"\\b"),
        b'\t' => output.extend_from_slice(b"\\t"),
        b'\n' => output.extend_from_slice(b"\\n"),
        b'\x0b' => output.extend_from_slice(b"\\v"),
        b'\x0c' => output.extend_from_slice(b"\\f"),
        b'\r' => output.extend_from_slice(b"\\r"),
        b'\\' => output.extend_from_slice(b"\\\\"),
        escaped if quote_to_escape == Some(escaped) => {
            output.push(b'\\');
            output.push(escaped);
        }
        b' '..=b'~' => output.push(byte),
        _ => push_duration_operand_octal_escape(output, byte),
    }
}

fn push_duration_operand_octal_escape(output: &mut Vec<u8>, byte: u8) {
    output.push(b'\\');
    output.push(b'0' + (byte >> 6));
    output.push(b'0' + ((byte >> 3) & 0o7));
    output.push(b'0' + (byte & 0o7));
}

impl CTError for InvalidTimeIntervalError {
    fn code(&self) -> i32 {
        1
    }

    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        let mut diagnostic = Vec::new();
        let locale = rust_i18n::locale();
        let message = localized_invalid_time_interval(&locale);
        for (index, operand) in self.operands.iter().enumerate() {
            if index > 0 {
                diagnostic.extend_from_slice(b"\n");
                diagnostic.extend_from_slice(ctcore::ct_util_name().as_bytes());
                diagnostic.extend_from_slice(b": ");
            }
            diagnostic.extend_from_slice(&sleep_encode_locale_text(&message));
            diagnostic.push(b' ');
            diagnostic.extend_from_slice(&quote_duration_operand_for_locale(operand, &locale));
        }
        Cow::Owned(diagnostic)
    }

    fn usage_hint_bytes(&self) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Owned(sleep_usage_hint()))
    }

    fn usage(&self) -> bool {
        true
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct SleepClockError {
    error: io::Error,
}

#[cfg(target_os = "linux")]
impl Display for SleepClockError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: {}",
            localized_cannot_read_realtime_clock(&rust_i18n::locale()),
            ctcore::ct_error::strip_errno(&self.error)
        )
    }
}

#[cfg(target_os = "linux")]
impl Error for SleepClockError {}

#[cfg(target_os = "linux")]
impl CTError for SleepClockError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        let mut diagnostic =
            sleep_encode_locale_text(&localized_cannot_read_realtime_clock(&rust_i18n::locale()));
        diagnostic.extend_from_slice(b": ");
        diagnostic.extend_from_slice(ctcore::ct_error::strip_errno(&self.error).as_bytes());
        Cow::Owned(diagnostic)
    }
}

#[derive(Debug)]
struct SleepUsageError {
    message: Vec<u8>,
}

impl SleepUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

impl Display for SleepUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for SleepUsageError {}

impl CTError for SleepUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage_hint_bytes(&self) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Owned(sleep_usage_hint()))
    }

    fn usage(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct Sleep;
impl Tool for Sleep {
    fn name(&self) -> &'static str {
        "sleep"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        sleep_main(args.iter().cloned())
    }
}

pub fn sleep_main(args: impl ctcore::Args) -> CTResult<()> {
    initialize_locale();
    rust_i18n::set_locale(sleep_i18n_locale());
    let args = prepare_sleep_args(args)?;
    let ends_with_option_delimiter = args.last().is_some_and(|arg| arg == "--");
    let matches = ct_app().try_get_matches_from(args)?;

    let numbers = sleep_parse_numbers(&matches, ends_with_option_delimiter)?;
    let sleep_dur = sleep_handle_second(&numbers)?;

    sleep(sleep_dur)
}

fn sleep_parse_numbers(matches: &clap::ArgMatches, allow_empty: bool) -> CTResult<Vec<&OsStr>> {
    let numbers = matches
        .get_many::<OsString>(sleep_flags::SLEEP_NUMBER)
        .map(|numbers| numbers.map(OsString::as_os_str).collect::<Vec<_>>())
        .or_else(|| allow_empty.then(Vec::new))
        .ok_or_else(|| {
            SleepUsageError::boxed(sleep_encode_locale_text(
                &t!("sleep.errors.missing_operand").to_string(),
            ))
        })?;

    Ok(numbers)
}

fn sleep_usage_hint() -> Vec<u8> {
    sleep_usage_hint_for_locale(ctcore::ct_help_utility_name(), &rust_i18n::locale())
}

fn sleep_usage_hint_for_locale(utility_name: &str, locale: &str) -> Vec<u8> {
    sleep_encode_locale_text(&sleep_usage_hint_text_for_locale(utility_name, locale))
}

fn sleep_usage_hint_text_for_locale(utility_name: &str, locale: &str) -> String {
    t!(
        "sleep.errors.try_help",
        locale = locale,
        utility_name = utility_name
    )
    .to_string()
}

fn sleep_encode_locale_text(text: &str) -> Vec<u8> {
    #[cfg(target_os = "linux")]
    if let Some(codeset) = sleep_output_codeset()
        && let Some(encoded) = sleep_encode_locale_text_for_codeset(text, &codeset)
    {
        return encoded;
    }

    text.as_bytes().to_vec()
}

#[cfg(target_os = "linux")]
fn sleep_output_codeset() -> Option<String> {
    // SAFETY: nl_langinfo returns storage owned by the active C locale. It is
    // copied before any later locale call can replace it.
    unsafe {
        let codeset = ctcore::libc::nl_langinfo(ctcore::libc::CODESET);
        if codeset.is_null() {
            return None;
        }
        let codeset = CStr::from_ptr(codeset).to_str().ok()?.to_owned();
        (!codeset.eq_ignore_ascii_case("UTF-8") && !codeset.eq_ignore_ascii_case("UTF8"))
            .then_some(codeset)
    }
}

#[cfg(target_os = "linux")]
fn sleep_encode_locale_text_for_codeset(text: &str, codeset: &str) -> Option<Vec<u8>> {
    let target = CString::new(format!("{codeset}//TRANSLIT")).ok()?;
    let source = CString::new("UTF-8").expect("UTF-8 has no NUL byte");
    let converter = unsafe { ctcore::libc::iconv_open(target.as_ptr(), source.as_ptr()) };
    if converter == (-1_isize) as ctcore::libc::iconv_t {
        return None;
    }

    let mut input = text.as_ptr().cast_mut().cast::<ctcore::libc::c_char>();
    let mut input_left = text.len();
    let mut output = vec![0_u8; text.len().saturating_mul(4).max(16)];
    let mut output_ptr = output.as_mut_ptr().cast::<ctcore::libc::c_char>();
    let mut output_left = output.len();
    let result = unsafe {
        ctcore::libc::iconv(
            converter,
            &mut input,
            &mut input_left,
            &mut output_ptr,
            &mut output_left,
        )
    };
    unsafe { ctcore::libc::iconv_close(converter) };
    if result == usize::MAX || input_left != 0 {
        return None;
    }

    output.truncate(output.len() - output_left);
    Some(output)
}

enum StandardLongOption {
    Unique(&'static str),
    Ambiguous,
    Unrecognized,
}

fn standard_long_option(name: &[u8]) -> StandardLongOption {
    if let Some(option) = ["help", "version"]
        .into_iter()
        .find(|option| option.as_bytes() == name)
    {
        return StandardLongOption::Unique(option);
    }

    let mut matches = ["help", "version"]
        .into_iter()
        .filter(|option| option.as_bytes().starts_with(name));
    let Some(option) = matches.next() else {
        return StandardLongOption::Unrecognized;
    };
    if matches.next().is_some() {
        StandardLongOption::Ambiguous
    } else {
        StandardLongOption::Unique(option)
    }
}

fn prepare_sleep_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    prepare_sleep_args_with_mode(args, ctcore::ct_posix::posixly_correct())
}

fn prepare_sleep_args_with_mode(
    args: impl ctcore::Args,
    posixly_correct: bool,
) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();

    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if bytes == b"--" {
            break;
        }
        if bytes.len() <= 1 || bytes[0] != b'-' {
            if posixly_correct {
                break;
            }
            continue;
        }

        if let Some(long) = bytes.strip_prefix(b"--") {
            let separator = long.iter().position(|byte| *byte == b'=');
            let name = &long[..separator.unwrap_or(long.len())];
            let option = match standard_long_option(name) {
                StandardLongOption::Unique(option) => option,
                StandardLongOption::Ambiguous => {
                    let mut message = b"option '".to_vec();
                    message.extend_from_slice(bytes);
                    message
                        .extend_from_slice(b"' is ambiguous; possibilities: '--help' '--version'");
                    return Err(SleepUsageError::boxed(message));
                }
                StandardLongOption::Unrecognized => {
                    let mut message = b"unrecognized option '".to_vec();
                    message.extend_from_slice(bytes);
                    message.push(b'\'');
                    return Err(SleepUsageError::boxed(message));
                }
            };
            if separator.is_some() {
                return Err(SleepUsageError::boxed(
                    format!("option '--{option}' doesn't allow an argument").into_bytes(),
                ));
            }

            let mut terminal_args = Vec::with_capacity(2);
            if let Some(program) = args.first() {
                terminal_args.push(program.clone());
            }
            terminal_args.push(argument.clone());
            return Ok(terminal_args);
        }

        let mut message = b"invalid option -- '".to_vec();
        message.push(bytes[1]);
        message.push(b'\'');
        return Err(SleepUsageError::boxed(message));
    }

    Ok(args)
}

fn initialize_locale() {
    // SAFETY: setlocale reads the process environment and receives a static
    // NUL-terminated string. GNU sleep initializes the C locale before parsing.
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
    }
}

fn sleep_i18n_locale() -> &'static str {
    let message_locale = sleep_effective_locale(["LC_ALL", "LC_MESSAGES", "LANG"]);
    let language = std::env::var_os("LANGUAGE");
    sleep_i18n_locale_for(message_locale.as_deref(), language.as_deref())
}

fn sleep_i18n_locale_for(message_locale: Option<&OsStr>, language: Option<&OsStr>) -> &'static str {
    let Some(message_locale) = message_locale else {
        return "en-US";
    };
    if sleep_c_locale(message_locale) {
        return "en-US";
    }

    if let Some(language) = language.filter(|language| !language.is_empty()) {
        for candidate in language
            .to_string_lossy()
            .split(':')
            .filter(|candidate| !candidate.is_empty())
        {
            if sleep_c_locale(OsStr::new(candidate)) {
                return "en-US";
            }
            if sleep_simplified_chinese_locale(OsStr::new(candidate)) {
                return "zh-CN";
            }
        }
    }

    if sleep_simplified_chinese_locale(message_locale) {
        "zh-CN"
    } else {
        "en-US"
    }
}

fn sleep_effective_locale<const N: usize>(names: [&str; N]) -> Option<OsString> {
    names.into_iter().find_map(|name| {
        let value = std::env::var_os(name)?;
        (!value.is_empty()).then_some(value)
    })
}

fn sleep_c_locale(locale: &OsStr) -> bool {
    matches!(
        locale.to_string_lossy().to_ascii_uppercase().as_str(),
        "C" | "POSIX"
    )
}

fn sleep_simplified_chinese_locale(locale: &OsStr) -> bool {
    let locale = locale.to_string_lossy();
    locale == "zh_CN" || locale.starts_with("zh_CN.") || locale.starts_with("zh_CN@")
}

fn current_numeric_decimal_point() -> String {
    // SAFETY: localeconv returns process-owned locale storage. The decimal
    // separator is copied before the next locale operation can invalidate it.
    unsafe {
        let locale = ctcore::libc::localeconv();
        if locale.is_null() || (*locale).decimal_point.is_null() {
            return ".".to_string();
        }
        CStr::from_ptr((*locale).decimal_point)
            .to_str()
            .ok()
            .filter(|point| !point.is_empty())
            .unwrap_or(".")
            .to_string()
    }
}

#[derive(Clone, Copy)]
enum FloatParseAttempt {
    Complete(f64),
    Partial(f64, usize),
    Invalid,
}

fn normalize_decimal_point(
    input: &str,
    decimal_point: &str,
    reject_c_decimal_point: bool,
) -> (String, Vec<usize>) {
    let mut normalized = String::with_capacity(input.len());
    let mut offsets = Vec::with_capacity(input.len() + 1);
    let mut index = 0;
    offsets.push(index);

    while index < input.len() {
        if !decimal_point.is_empty() && input[index..].starts_with(decimal_point) {
            normalized.push('.');
            index += decimal_point.len();
            offsets.push(index);
        } else if reject_c_decimal_point && decimal_point != "." && input.as_bytes()[index] == b'.'
        {
            normalized.push('\u{1}');
            index += 1;
            offsets.push(index);
        } else {
            let character = input[index..]
                .chars()
                .next()
                .expect("index is before the end of a UTF-8 string");
            normalized.push(character);
            for byte in 1..=character.len_utf8() {
                offsets.push(index + byte);
            }
            index += character.len_utf8();
        }
    }

    (normalized, offsets)
}

fn parse_float_attempt(
    input: &str,
    decimal_point: &str,
    reject_c_decimal_point: bool,
) -> FloatParseAttempt {
    // ct_format also supports printf's leading quote character literal, while
    // GNU sleep delegates to strtod, which rejects that syntax.
    if input.starts_with(['\'', '"']) {
        return FloatParseAttempt::Invalid;
    }

    let (normalized, offsets) =
        normalize_decimal_point(input, decimal_point, reject_c_decimal_point);
    match ParsedNumber::parse_f64(&normalized) {
        Ok(value) => FloatParseAttempt::Complete(value),
        Err(ParseError::CtPartialMatch(value, rest)) => {
            let consumed = normalized.len() - rest.len();
            FloatParseAttempt::Partial(value, offsets[consumed])
        }
        Err(ParseError::CtNotNumeric | ParseError::CtOverflow) => FloatParseAttempt::Invalid,
    }
}

fn parse_duration_with_decimal_point(input: &str, decimal_point: &str) -> Option<f64> {
    let locale = parse_float_attempt(input, decimal_point, true);
    let c_locale = parse_float_attempt(input, ".", false);

    let (seconds, consumed) = match (locale, c_locale) {
        (FloatParseAttempt::Complete(seconds), _) => (seconds, input.len()),
        (_, FloatParseAttempt::Complete(seconds)) => (seconds, input.len()),
        (
            FloatParseAttempt::Partial(seconds, consumed),
            FloatParseAttempt::Partial(c_seconds, c_consumed),
        ) => {
            if c_consumed > consumed {
                (c_seconds, c_consumed)
            } else {
                (seconds, consumed)
            }
        }
        (FloatParseAttempt::Partial(seconds, consumed), FloatParseAttempt::Invalid) => {
            (seconds, consumed)
        }
        (FloatParseAttempt::Invalid, FloatParseAttempt::Partial(seconds, consumed)) => {
            (seconds, consumed)
        }
        (FloatParseAttempt::Invalid, FloatParseAttempt::Invalid) => return None,
    };

    let suffix = &input[consumed..];

    match suffix {
        "" | "s" => Some(seconds),
        "m" => Some(seconds * 60.0),
        "h" => Some(seconds * 60.0 * 60.0),
        "d" => Some(seconds * 60.0 * 60.0 * 24.0),
        _ => None,
    }
}

fn parse_duration(input: &str) -> Option<f64> {
    parse_duration_with_decimal_point(input, &current_numeric_decimal_point())
}

fn duration_from_seconds(seconds: f64) -> Duration {
    const NANOS_PER_SECOND: u64 = 1_000_000_000;
    const MAX_TIME_T_SECONDS: u64 = i64::MAX as u64;

    if !seconds.is_finite() || seconds >= MAX_TIME_T_SECONDS as f64 {
        return Duration::new(MAX_TIME_T_SECONDS, NANOS_PER_SECOND as u32 - 1);
    }

    let mut whole_seconds = seconds as u64;
    let fractional_nanos = NANOS_PER_SECOND as f64 * (seconds - whole_seconds as f64);
    let mut nanos = fractional_nanos as u64;
    nanos += u64::from((nanos as f64) < fractional_nanos);
    whole_seconds += nanos / NANOS_PER_SECOND;
    nanos %= NANOS_PER_SECOND;

    Duration::new(whole_seconds, nanos as u32)
}

fn sleep_handle_second<T: AsRef<OsStr>>(args: &[T]) -> CTResult<Duration> {
    let mut invalid_operands = Vec::new();
    let mut seconds = 0.0;

    for input in args {
        match input.as_ref().to_str().and_then(parse_duration) {
            Some(interval) if interval >= 0.0 => seconds += interval,
            _ => invalid_operands.push(input.as_ref().to_os_string()),
        }
    }

    if !invalid_operands.is_empty() {
        return Err(InvalidTimeIntervalError {
            operands: invalid_operands,
        }
        .into());
    }

    Ok(duration_from_seconds(seconds))
}

#[cfg(target_os = "linux")]
fn sleep_with_nanosleep(
    sleep_dur: Duration,
    mut nanosleep: impl FnMut(&ctcore::libc::timespec, &mut ctcore::libc::timespec) -> io::Result<()>,
) -> CTResult<()> {
    let mut remaining = ctcore::libc::timespec {
        tv_sec: sleep_dur.as_secs() as ctcore::libc::time_t,
        tv_nsec: sleep_dur.subsec_nanos().into(),
    };

    loop {
        let mut next = ctcore::libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        match nanosleep(&remaining, &mut next) {
            Ok(()) => return Ok(()),
            // GNU xnanosleep also retries when a resumed old Linux kernel
            // reports failure without setting errno.
            Err(error)
                if error.kind() == io::ErrorKind::Interrupted
                    || error.raw_os_error() == Some(0) =>
            {
                remaining = next;
            }
            Err(error) => return Err(Box::new(SleepClockError { error })),
        }
    }
}

fn sleep(sleep_dur: Duration) -> CTResult<()> {
    #[cfg(target_os = "linux")]
    {
        sleep_with_nanosleep(sleep_dur, |remaining, next| {
            // SAFETY: both pointers refer to initialized local timespec values.
            let result = unsafe { ctcore::libc::nanosleep(remaining, next) };
            if result == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        })
    }

    #[cfg(not(target_os = "linux"))]
    {
        thread::sleep(sleep_dur);
        Ok(())
    }
}

pub fn ct_app() -> Command {
    ct_app_with_getopt_mode(ctcore::ct_posix::posixly_correct())
}

fn ct_app_with_getopt_mode(posixly_correct: bool) -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("sleep.about");
    let usage_description = t!("sleep.usage");
    let args = vec![
        Arg::new(sleep_flags::SLEEP_NUMBER)
            .help(t!("sleep.clap.sleep_number"))
            .value_name(sleep_flags::SLEEP_NUMBER)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new()),
        Arg::new("help").long("help").action(ArgAction::Help),
        Arg::new("version")
            .long("version")
            .action(ArgAction::Version),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(t!("sleep.after_help"))
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(args)
        .gnu_getopt_with_mode(posixly_correct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn test_tool_implementation() {
        let tool = Sleep;

        // Test name method
        assert_eq!(tool.name(), "sleep");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("sleep"));

        // Test execute method with help flag (should work)
        let args: Vec<OsString> = vec![OsString::from("sleep"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    #[cfg(test)]
    mod handle_second_tests {
        use super::*;
        use clap::Command;

        #[test]
        fn test_sleep_parse_numbers() {
            let cmd = Command::new("test").arg(
                Arg::new(sleep_flags::SLEEP_NUMBER)
                    .action(ArgAction::Append)
                    .value_parser(OsStringValueParser::new()),
            );

            let matches = cmd.try_get_matches_from(vec!["test", "5", "10"]).unwrap();
            let numbers = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(numbers, vec!["5", "10"]);
        }

        #[test]
        fn test_sleep_handle_second() {
            let args = vec!["5s", "1m", "2h"];
            let duration = sleep_handle_second(&args).unwrap();

            assert_eq!(duration, Duration::from_secs(5 + 60 + 2 * 3600));
        }

        #[test]
        fn test_sleep_handle_second_invalid_input() {
            let args = vec!["5x", "1m"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_single_second() {
            let args = vec!["5s"];
            let duration = sleep_handle_second(&args).unwrap();

            assert_eq!(duration, Duration::from_secs(5));
        }

        #[test]
        fn test_sleep_handle_second_multiple_units() {
            let args = vec!["1d", "2h", "30m", "45s"];
            let duration = sleep_handle_second(&args).unwrap();

            let expected_duration = Duration::from_secs(86400 + 2 * 3600 + 30 * 60 + 45);
            assert_eq!(duration, expected_duration);
        }

        #[test]
        fn test_sleep_handle_second_empty_input() {
            let args = vec![""];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_whitespace_input() {
            let args = vec!["  "];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_invalid_format() {
            let args = vec!["5x", "2y"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_mixed_valid_invalid() {
            let args = vec!["5s", "invalid", "10m"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_large_duration() {
            let args = vec!["1000d", "24h"];
            let duration = sleep_handle_second(&args).unwrap();

            let expected_duration = Duration::from_secs(1000 * 86400 + 24 * 3600);
            assert_eq!(duration, expected_duration);
        }

        #[test]
        fn test_sleep_handle_second_negative_duration() {
            let args = vec!["-5s"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_accepts_hex_duration_with_suffix() {
            let duration = sleep_handle_second(&["0x1p0s"]).unwrap();

            assert_eq!(duration, Duration::from_secs(1));
        }

        #[test]
        fn test_sleep_handle_second_rejects_hex_duration_without_mantissa() {
            for input in ["0x.", "0x.p0"] {
                assert!(sleep_handle_second(&[input]).is_err(), "{input}");
            }
        }

        #[test]
        fn test_sleep_handle_second_rejects_printf_character_literals() {
            for input in ["'0", "\"0"] {
                assert!(sleep_handle_second(&[input]).is_err(), "{input}");
            }
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_sleep_reports_nanosleep_failure_as_gnu_clock_error() {
            let error = sleep_with_nanosleep(Duration::ZERO, |_, _| {
                Err(std::io::Error::from_raw_os_error(ctcore::libc::EPERM))
            })
            .unwrap_err();

            assert_eq!(error.code(), 1);
            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"cannot read realtime clock: Operation not permitted"
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_sleep_retries_interrupted_nanosleep() {
            let mut calls = 0;
            sleep_with_nanosleep(Duration::ZERO, |_, _| {
                calls += 1;
                if calls == 1 {
                    Err(std::io::Error::from_raw_os_error(ctcore::libc::EINTR))
                } else {
                    Ok(())
                }
            })
            .unwrap();

            assert_eq!(calls, 2);
        }

        #[test]
        fn test_sleep_handle_second_rejects_trailing_whitespace() {
            assert!(sleep_handle_second(&["0 "]).is_err());
        }

        #[test]
        fn test_sleep_handle_second_accepts_leading_whitespace() {
            assert_eq!(sleep_handle_second(&[" 0"]).unwrap(), Duration::ZERO);
        }

        #[test]
        fn test_sleep_handle_second_accumulates_and_rounds_subnanosecond_intervals() {
            assert_eq!(
                sleep_handle_second(&["0.0000000001"]).unwrap(),
                Duration::from_nanos(1)
            );
            assert_eq!(
                sleep_handle_second(&["0.0000000006", "0.0000000006"]).unwrap(),
                Duration::from_nanos(2)
            );
        }

        #[test]
        fn test_parse_duration_prefers_locale_decimal_point_and_falls_back_to_c_locale() {
            assert_eq!(parse_duration_with_decimal_point("0,0", ","), Some(0.0));
            assert_eq!(parse_duration_with_decimal_point("0.0", ","), Some(0.0));
            assert_eq!(parse_duration_with_decimal_point("0,0.0", ","), None);
        }

        #[test]
        fn test_prepare_sleep_args_uses_gnu_standard_option_diagnostics() {
            let cases = [
                (
                    vec![OsString::from("sleep"), OsString::from("-h")],
                    b"invalid option -- 'h'".as_slice(),
                ),
                (
                    vec![OsString::from("sleep"), OsString::from("--unknown")],
                    b"unrecognized option '--unknown'".as_slice(),
                ),
                (
                    vec![OsString::from("sleep"), OsString::from("--hel=value")],
                    b"option '--help' doesn't allow an argument".as_slice(),
                ),
                (
                    vec![OsString::from("sleep"), OsString::from("--=")],
                    b"option '--=' is ambiguous; possibilities: '--help' '--version'".as_slice(),
                ),
            ];

            for (args, diagnostic) in cases {
                let error = prepare_sleep_args(args.into_iter()).unwrap_err();
                assert_eq!(error.diagnostic_bytes().as_ref(), diagnostic);
            }
        }

        #[test]
        fn test_posix_mode_stops_option_parsing_after_the_first_duration() {
            let args = vec![
                OsString::from("sleep"),
                OsString::from("0"),
                OsString::from("--help"),
            ];
            assert_eq!(
                prepare_sleep_args_with_mode(args.clone().into_iter(), true).unwrap(),
                args
            );

            let matches = ct_app_with_getopt_mode(true)
                .try_get_matches_from(args)
                .unwrap();
            let durations = matches
                .get_many::<OsString>(sleep_flags::SLEEP_NUMBER)
                .unwrap()
                .collect::<Vec<_>>();
            assert_eq!(durations, [&OsString::from("0"), &OsString::from("--help")]);
        }

        #[test]
        fn test_quote_duration_operand_uses_utf8_locale_quotes() {
            assert_eq!(
                quote_duration_operand_with_style(OsStr::new("a'b"), true),
                "‘a'b’".as_bytes()
            );
            assert_eq!(
                quote_duration_operand_with_style(OsStr::new("\u{4e2d}"), true),
                "‘\u{4e2d}’".as_bytes()
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_quote_duration_operand_escapes_invalid_utf8_in_utf8_locale() {
            let input = OsString::from_vec(vec![0xff]);

            assert_eq!(
                quote_duration_operand_with_style(input.as_os_str(), true),
                "‘\\377’".as_bytes()
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_quote_duration_operand_preserves_valid_utf8_before_invalid_bytes() {
            let input = OsString::from_vec(vec![0xe4, 0xb8, 0xad, 0xff]);

            assert_eq!(
                quote_duration_operand_with_style(input.as_os_str(), true),
                "‘中\\377’".as_bytes()
            );
        }

        #[test]
        fn test_quote_duration_operand_escapes_nonprinting_utf8_characters() {
            assert_eq!(
                quote_duration_operand_with_style(OsStr::new("\u{80}"), true),
                "‘\\302\\200’".as_bytes()
            );
        }

        #[test]
        fn test_quote_duration_operand_escapes_special_characters_after_utf8() {
            assert_eq!(
                quote_duration_operand_with_quote_marks(
                    OsStr::new("中\"\\"),
                    true,
                    b"\"",
                    b"\"",
                    Some(b'\"')
                ),
                b"\"\xe4\xb8\xad\\\"\\\\\""
            );
            assert_eq!(
                quote_duration_operand_with_style(OsStr::new("中’"), true),
                "‘中\\’’".as_bytes()
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_quote_duration_operand_preserves_non_utf8_c_locale_bytes() {
            let input = OsString::from_vec(vec![0xff]);

            assert_eq!(
                quote_duration_operand_with_style(input.as_os_str(), false),
                b"'\\377'"
            );
        }

        #[test]
        fn test_invalid_time_interval_uses_simplified_chinese_diagnostic() {
            assert_eq!(localized_invalid_time_interval("zh-CN"), "无效的时间间隔");
            assert_eq!(
                t!("sleep.errors.missing_operand", locale = "zh-CN"),
                "缺少操作对象"
            );
            assert_eq!(
                quote_duration_operand_for_locale(OsStr::new("invalid"), "zh-CN"),
                b"\"invalid\""
            );
            assert_eq!(
                quote_duration_operand_for_locale(OsStr::new("a\"b"), "zh-CN"),
                b"\"a\\\"b\""
            );
            assert_eq!(
                sleep_usage_hint_text_for_locale("sleep", "zh-CN"),
                "请尝试执行 \"sleep --help\" 来获取更多信息。"
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_sleep_zh_cn_gbk_diagnostic_text_uses_locale_encoding() {
            assert_eq!(
                sleep_encode_locale_text_for_codeset("无效的时间间隔", "GBK"),
                Some(vec![
                    0xce, 0xde, 0xd0, 0xa7, 0xb5, 0xc4, 0xca, 0xb1, 0xbc, 0xe4, 0xbc, 0xe4, 0xb8,
                    0xf4,
                ])
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_quote_duration_operand_preserves_valid_gbk_characters() {
            use std::os::unix::ffi::OsStringExt;

            let input = OsString::from_vec(vec![0xd6, 0xd0, 0xb9, 0xfa, b'"']);

            assert_eq!(
                quote_duration_operand_with_locale_encoding(input.as_os_str(), "GBK", b"\"", b"\""),
                b"\"\xd6\xd0\xb9\xfa\\\"\""
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_english_diagnostic_preserves_valid_gbk_characters() {
            use std::os::unix::ffi::OsStringExt;

            let input = OsString::from_vec(vec![0xd6, 0xd0, 0xb9, 0xfa]);

            assert_eq!(
                quote_duration_operand_for_locale_with_codeset(
                    input.as_os_str(),
                    "en-US",
                    Some("GBK")
                ),
                b"'\xd6\xd0\xb9\xfa'"
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_english_diagnostic_uses_gnu_gb18030_fallback_quotes() {
            assert_eq!(
                quote_duration_operand_for_locale_with_codeset(
                    OsStr::new("x"),
                    "en-US",
                    Some("GB18030")
                ),
                b"\xa1\x07ex\xa1\xaf"
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_english_gb18030_diagnostic_escapes_fallback_right_quote() {
            use std::os::unix::ffi::OsStringExt;

            let input = OsString::from_vec(vec![0xa1, 0xaf]);

            assert_eq!(
                quote_duration_operand_for_locale_with_codeset(
                    input.as_os_str(),
                    "en-US",
                    Some("GB18030")
                ),
                b"\xa1\x07e\\\xa1\xaf\xa1\xaf"
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_english_gb18030_diagnostic_escapes_nonprinting_character() {
            use std::os::unix::ffi::OsStringExt;

            let input = OsString::from_vec(vec![0x81, 0x30, 0x81, 0x30]);

            assert_eq!(
                quote_duration_operand_for_locale_with_codeset(
                    input.as_os_str(),
                    "en-US",
                    Some("GB18030")
                ),
                b"\xa1\x07e\\201\\060\\201\\060\xa1\xaf"
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_english_gb18030_diagnostic_escapes_incomplete_character_suffix() {
            use std::os::unix::ffi::OsStringExt;

            let input = OsString::from_vec(vec![0x81, 0x30]);

            assert_eq!(
                quote_duration_operand_for_locale_with_codeset(
                    input.as_os_str(),
                    "en-US",
                    Some("GB18030")
                ),
                b"\xa1\x07e\\201\\060\xa1\xaf"
            );
        }

        #[test]
        fn test_invalid_time_interval_uses_default_locale_diagnostic() {
            assert_eq!(
                localized_invalid_time_interval("en-US"),
                "invalid time interval"
            );
            assert_eq!(
                sleep_usage_hint_for_locale("sleep", "en-US"),
                b"Try 'sleep --help' for more information."
            );
        }

        #[test]
        fn test_sleep_i18n_locale_uses_message_locale_and_language_override() {
            assert_eq!(
                sleep_i18n_locale_for(Some(OsStr::new("zh_CN.utf8")), None),
                "zh-CN"
            );
            assert_eq!(
                sleep_i18n_locale_for(Some(OsStr::new("zh_CN.utf8")), Some(OsStr::new("C"))),
                "en-US"
            );
            assert_eq!(
                sleep_i18n_locale_for(Some(OsStr::new("C")), Some(OsStr::new("zh_CN"))),
                "en-US"
            );
        }
    }
    #[cfg(test)]
    mod sleep_parse_numbers_tests {
        use super::*;
        #[test]
        fn test_sleep_parse_numbers_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false);

            assert!(result.is_err());
            let error = result.unwrap_err();
            assert_eq!(error.to_string(), "missing operand");
            assert!(error.usage());
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_5() {
            let args = vec![ctcore::ct_util_name(), "5"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["5"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_0() {
            let args = vec![ctcore::ct_util_name(), "0"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["0"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_seconds_2() {
            let args = vec![ctcore::ct_util_name(), "2s"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2s"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_minutes_2() {
            let args = vec![ctcore::ct_util_name(), "2m"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2m"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_hours_2() {
            let args = vec![ctcore::ct_util_name(), "2h"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2h"]);
        }
        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_days_2() {
            let args = vec![ctcore::ct_util_name(), "2d"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2d"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_err_2() {
            let args = vec![ctcore::ct_util_name(), "2q"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2q"]);
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_sleep_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = sleep_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_rejects_short_version_option() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = sleep_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_rejects_short_help_option() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_accepts_end_of_options_without_operands() {
            let args = [ctcore::ct_util_name(), "--"];
            let result = sleep_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_1() {
            let args = [ctcore::ct_util_name(), "1"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_0_3_0_2() {
            let args = [ctcore::ct_util_name(), "0.3", "0.2"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_0_3_0_2_0_1() {
            let args = [ctcore::ct_util_name(), "0.3", "0.2", "0.1"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_1_qq() {
            let args = [ctcore::ct_util_name(), "1", "qq"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_sleep_0_1_0_3_s() {
            let args = [ctcore::ct_util_name(), "0.1s", "0.3s"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_0() {
            let args = [ctcore::ct_util_name(), "0"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_suffix_seconds_1() {
            let args = [ctcore::ct_util_name(), "1s"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_suffix_err_1() {
            let args = [ctcore::ct_util_name(), "1q"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;
        #[cfg(unix)]
        use std::os::unix::ffi::OsStringExt;

        // sleep 接口: sleep NUMBER[SUFFIX]...
        //             sleep OPTION
        //
        // Arguments:
        //   [NUMBER]...  pause for NUMBER seconds
        //
        // Options:
        //   --help         Print help
        //   --version      Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_rejects_short_version_option() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_rejects_short_help_option() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[cfg(unix)]
        #[test]
        fn test_ct_app_accepts_non_utf8_duration_operand() {
            let command = ct_app();
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from_vec(vec![0xff]),
            ];

            assert!(command.try_get_matches_from(args).is_ok());
        }

        #[test]
        fn test_ct_app_sleep_5() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "5"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_0() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "0"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_seconds_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2s"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_minutes_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2m"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_hours_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2h"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_sleep_suffix_days_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2d"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_err_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2q"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
