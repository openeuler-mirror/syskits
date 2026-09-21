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

extern crate rust_i18n;
use clap::{Command, crate_version};
use rust_i18n::t;
use std::borrow::Cow;
use std::error::Error;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::ct_println_verbatim;
#[cfg(not(unix))]
use ctcore::ct_error::CtSimpleError;
#[cfg(not(unix))]
use ctcore::ct_error::FromIo;
use ctcore::ct_error::{CTError, CTResult, strip_errno};
#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString};
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::io;
use sys_locale::get_locale;

mod platform;

pub fn whoami_main(args: impl ctcore::Args) -> CTResult<String> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    #[cfg(unix)]
    if ctcore::ct_sigpipe_was_default() {
        let _ = ctcore::ct_signals::enable_pipe_errors();
    }

    ct_app().try_get_matches_from(prepare_whoami_args(args)?)?;
    let username = whoami_exec()?;
    write_whoami_username(&username).map_err(whoami_write_error)?;

    let result = username.to_string_lossy().into_owned();
    Ok(result)
}

fn write_whoami_username(username: &OsStr) -> io::Result<()> {
    if let Some(error) = whoami_closed_stdout_error(ctcore::ct_stdout_was_closed()) {
        return Err(error);
    }
    ct_println_verbatim(username)
}

fn whoami_write_error(error: io::Error) -> Box<dyn CTError> {
    WhoamiRuntimeError::boxed(whoami_write_error_message(&error))
}

fn whoami_write_error_message(error: &io::Error) -> Vec<u8> {
    whoami_write_error_message_for_locale(error, whoami_uses_simplified_chinese())
}

fn whoami_write_error_message_for_locale(error: &io::Error, simplified_chinese: bool) -> Vec<u8> {
    let error = strip_errno(error);
    let mut message = if simplified_chinese {
        whoami_encode_locale_text("写入错误: ")
    } else {
        b"write error: ".to_vec()
    };
    message.extend_from_slice(error.as_bytes());
    message
}

fn whoami_closed_stdout_error(stdout_was_closed: bool) -> Option<io::Error> {
    #[cfg(unix)]
    {
        stdout_was_closed.then(|| io::Error::from_raw_os_error(libc::EBADF))
    }

    #[cfg(not(unix))]
    {
        let _ = stdout_was_closed;
        None
    }
}

const WHOAMI_LONG_OPTIONS: &[&str] = &["help", "version"];

#[derive(Debug, PartialEq, Eq)]
enum WhoamiLongOptionMatch {
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
    None,
}

#[derive(Debug)]
struct WhoamiUsageError {
    message: Vec<u8>,
    usage_hint: Vec<u8>,
}

impl WhoamiUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self {
            message,
            usage_hint: whoami_usage_hint(
                ctcore::ct_help_utility_name(),
                whoami_uses_simplified_chinese(),
            ),
        })
    }
}

impl Display for WhoamiUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for WhoamiUsageError {}

impl CTError for WhoamiUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage_hint_bytes(&self) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Borrowed(&self.usage_hint))
    }

    fn usage(&self) -> bool {
        true
    }
}

#[derive(Debug)]
struct WhoamiRuntimeError {
    message: Vec<u8>,
}

impl WhoamiRuntimeError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

impl Display for WhoamiRuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for WhoamiRuntimeError {}

impl CTError for WhoamiRuntimeError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }
}

fn prepare_whoami_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    prepare_whoami_args_with_mode(args, ctcore::ct_posix::posixly_correct())
}

fn prepare_whoami_args_with_mode(
    args: impl ctcore::Args,
    posixly_correct: bool,
) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut parse_options = true;
    let mut first_operand = None;

    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if parse_options && bytes == b"--" {
            parse_options = false;
            continue;
        }

        if parse_options && bytes.len() > 1 && bytes[0] == b'-' {
            if bytes.starts_with(b"--") {
                let terminal = validate_whoami_long_option(bytes)?;
                if terminal {
                    let mut terminal_args = Vec::with_capacity(2);
                    if let Some(program) = args.first() {
                        terminal_args.push(program.clone());
                    }
                    terminal_args.push(argument.clone());
                    return Ok(terminal_args);
                }
                continue;
            }

            // Keep the syskits short help/version extensions outside GNU option handling.
            if bytes == b"-h" || bytes == b"-V" {
                return Ok(args);
            }

            let mut message = b"invalid option -- '".to_vec();
            message.push(bytes[1]);
            message.push(b'\'');
            return Err(WhoamiUsageError::boxed(message));
        }

        first_operand.get_or_insert(argument);
        if posixly_correct {
            parse_options = false;
        }
    }

    if let Some(operand) = first_operand {
        return Err(WhoamiUsageError::boxed(whoami_extra_operand_message(
            operand,
            whoami_uses_simplified_chinese(),
        )));
    }

    Ok(args)
}

fn validate_whoami_long_option(argument: &[u8]) -> CTResult<bool> {
    let long = &argument[2..];
    let separator = long.iter().position(|byte| *byte == b'=');
    let name = &long[..separator.unwrap_or(long.len())];

    match match_whoami_long_option(name) {
        WhoamiLongOptionMatch::None => {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(argument);
            message.push(b'\'');
            Err(WhoamiUsageError::boxed(message))
        }
        WhoamiLongOptionMatch::Ambiguous(candidates) => {
            let mut message = b"option '".to_vec();
            message.extend_from_slice(argument);
            message.extend_from_slice(b"' is ambiguous; possibilities:");
            for candidate in candidates {
                message.extend_from_slice(b" '--");
                message.extend_from_slice(candidate.as_bytes());
                message.push(b'\'');
            }
            Err(WhoamiUsageError::boxed(message))
        }
        WhoamiLongOptionMatch::Recognized(canonical) if separator.is_some() => {
            Err(WhoamiUsageError::boxed(
                format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
            ))
        }
        WhoamiLongOptionMatch::Recognized("help" | "version") => Ok(true),
        WhoamiLongOptionMatch::Recognized(_) => unreachable!("whoami only has standard options"),
    }
}

fn match_whoami_long_option(name: &[u8]) -> WhoamiLongOptionMatch {
    if let Some(option) = WHOAMI_LONG_OPTIONS
        .iter()
        .copied()
        .find(|option| option.as_bytes() == name)
    {
        return WhoamiLongOptionMatch::Recognized(option);
    }

    let candidates = WHOAMI_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|option| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [] => WhoamiLongOptionMatch::None,
        [candidate] => WhoamiLongOptionMatch::Recognized(candidate),
        _ => WhoamiLongOptionMatch::Ambiguous(candidates),
    }
}

fn whoami_extra_operand_message(operand: &OsStr, simplified_chinese: bool) -> Vec<u8> {
    let mut message = if simplified_chinese {
        whoami_encode_locale_text("多余的操作对象 ")
    } else {
        b"extra operand ".to_vec()
    };
    message.extend_from_slice(&quote_whoami_operand(operand, simplified_chinese));
    message
}

fn whoami_usage_hint(utility_name: &str, simplified_chinese: bool) -> Vec<u8> {
    if simplified_chinese {
        whoami_encode_locale_text(&format!(
            "请尝试执行 \"{utility_name} --help\" 来获取更多信息。"
        ))
    } else {
        format!("Try '{utility_name} --help' for more information.").into_bytes()
    }
}

fn whoami_uses_simplified_chinese() -> bool {
    #[cfg(target_os = "linux")]
    if !whoami_locale_environment_is_valid() {
        return false;
    }

    let simplified_chinese =
        whoami_message_locale().is_some_and(|locale| whoami_simplified_chinese_locale(&locale));
    if !simplified_chinese {
        return false;
    }

    #[cfg(target_os = "linux")]
    if let Some(codeset) = whoami_output_codeset() {
        return whoami_encode_locale_text_for_codeset("多余的操作对象", &codeset).is_some();
    }

    true
}

fn whoami_message_locale() -> Option<OsString> {
    let locale = whoami_effective_locale(["LC_ALL", "LC_MESSAGES", "LANG"])?;
    if whoami_c_message_locale(&locale) {
        return Some(locale);
    }

    let Some(language) = std::env::var_os("LANGUAGE").filter(|language| !language.is_empty())
    else {
        return Some(locale);
    };
    let language = language.to_string_lossy();
    for candidate in language
        .split(':')
        .filter(|candidate| !candidate.is_empty())
    {
        if whoami_c_message_locale(OsStr::new(candidate)) {
            return Some(OsString::from("C"));
        }
        if whoami_simplified_chinese_locale(OsStr::new(candidate)) {
            return Some(OsString::from(candidate));
        }
    }

    Some(OsString::from("C"))
}

fn whoami_c_message_locale(locale: &OsStr) -> bool {
    matches!(
        locale.to_string_lossy().to_ascii_uppercase().as_str(),
        "C" | "POSIX"
    )
}

fn whoami_simplified_chinese_locale(locale: &OsStr) -> bool {
    let locale = locale.to_string_lossy().to_ascii_lowercase();
    locale == "zh_cn" || locale.starts_with("zh_cn.") || locale.starts_with("zh_cn@")
}

fn whoami_effective_locale<const N: usize>(names: [&str; N]) -> Option<OsString> {
    names.into_iter().find_map(|name| {
        let value = std::env::var_os(name)?;
        (!value.is_empty()).then_some(value)
    })
}

fn whoami_encode_locale_text(text: &str) -> Vec<u8> {
    #[cfg(target_os = "linux")]
    if let Some(codeset) = whoami_output_codeset()
        && let Some(encoded) = whoami_encode_locale_text_for_codeset(text, &codeset)
    {
        return encoded;
    }

    text.as_bytes().to_vec()
}

#[cfg(target_os = "linux")]
fn whoami_output_codeset() -> Option<String> {
    if !whoami_locale_environment_is_valid() {
        return Some("ASCII".to_owned());
    }

    if let Some(codeset) = std::env::var_os("OUTPUT_CHARSET").filter(|codeset| !codeset.is_empty())
    {
        return Some(codeset.to_string_lossy().into_owned());
    }

    whoami_ctype_codeset()
}

#[cfg(target_os = "linux")]
fn whoami_ctype_codeset() -> Option<String> {
    let locale = whoami_effective_locale(["LC_ALL", "LC_CTYPE", "LANG"])?;
    let locale_text = locale.to_string_lossy();
    let locale_uppercase = locale_text.to_ascii_uppercase();
    if locale_uppercase.contains("UTF-8") || locale_uppercase.contains("UTF8") {
        return None;
    }

    if matches!(locale_uppercase.as_str(), "C" | "POSIX") {
        return Some("ASCII".to_owned());
    }

    locale_text
        .split_once('.')
        .map(|(_, codeset)| {
            codeset
                .split_once('@')
                .map_or(codeset, |(codeset, _)| codeset)
        })
        .map(ToOwned::to_owned)
        .or_else(|| whoami_locale_codeset(&locale))
}

#[cfg(target_os = "linux")]
fn whoami_locale_codeset(locale: &OsStr) -> Option<String> {
    let locale = CString::new(locale.as_encoded_bytes()).ok()?;
    let locale_handle =
        unsafe { libc::newlocale(libc::LC_CTYPE_MASK, locale.as_ptr(), std::ptr::null_mut()) };
    if locale_handle.is_null() {
        return None;
    }

    let codeset = unsafe {
        let codeset = libc::nl_langinfo_l(libc::CODESET, locale_handle);
        (!codeset.is_null()).then(|| CStr::from_ptr(codeset).to_bytes().to_vec())
    };
    unsafe { libc::freelocale(locale_handle) };
    codeset.and_then(|codeset| String::from_utf8(codeset).ok())
}

#[cfg(target_os = "linux")]
const WHOAMI_LOCALE_CATEGORIES: &[(&str, libc::c_int)] = &[
    ("LC_CTYPE", libc::LC_CTYPE_MASK),
    ("LC_NUMERIC", libc::LC_NUMERIC_MASK),
    ("LC_TIME", libc::LC_TIME_MASK),
    ("LC_COLLATE", libc::LC_COLLATE_MASK),
    ("LC_MONETARY", libc::LC_MONETARY_MASK),
    ("LC_MESSAGES", libc::LC_MESSAGES_MASK),
];

#[cfg(all(target_os = "linux", target_env = "gnu"))]
const WHOAMI_GNU_LOCALE_CATEGORIES: &[(&str, libc::c_int)] = &[
    ("LC_PAPER", libc::LC_PAPER_MASK),
    ("LC_NAME", libc::LC_NAME_MASK),
    ("LC_ADDRESS", libc::LC_ADDRESS_MASK),
    ("LC_TELEPHONE", libc::LC_TELEPHONE_MASK),
    ("LC_MEASUREMENT", libc::LC_MEASUREMENT_MASK),
    ("LC_IDENTIFICATION", libc::LC_IDENTIFICATION_MASK),
];

#[cfg(all(target_os = "linux", not(target_env = "gnu")))]
const WHOAMI_GNU_LOCALE_CATEGORIES: &[(&str, libc::c_int)] = &[];

#[cfg(target_os = "linux")]
fn whoami_locale_environment_is_valid() -> bool {
    let lc_all = std::env::var_os("LC_ALL").filter(|value| !value.is_empty());
    let lang = std::env::var_os("LANG")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("C"));

    WHOAMI_LOCALE_CATEGORIES
        .iter()
        .chain(WHOAMI_GNU_LOCALE_CATEGORIES)
        .all(|(category, mask)| {
            let locale = lc_all
                .clone()
                .or_else(|| std::env::var_os(category).filter(|value| !value.is_empty()))
                .unwrap_or_else(|| lang.clone());
            whoami_locale_is_available(&locale, *mask)
        })
}

#[cfg(target_os = "linux")]
fn whoami_locale_is_available(locale: &OsStr, category_mask: libc::c_int) -> bool {
    let locale = match CString::new(locale.as_encoded_bytes()) {
        Ok(locale) => locale,
        Err(_) => return false,
    };
    let locale_handle =
        unsafe { libc::newlocale(category_mask, locale.as_ptr(), std::ptr::null_mut()) };
    if locale_handle.is_null() {
        return false;
    }
    unsafe { libc::freelocale(locale_handle) };
    true
}

#[cfg(target_os = "linux")]
fn whoami_encode_locale_text_for_codeset(text: &str, codeset: &str) -> Option<Vec<u8>> {
    let target = CString::new(format!("{codeset}//TRANSLIT")).ok()?;
    let source = CString::new("UTF-8").expect("UTF-8 has no NUL byte");
    let converter = unsafe { libc::iconv_open(target.as_ptr(), source.as_ptr()) };
    if converter == (-1_isize) as libc::iconv_t {
        return None;
    }

    let mut input = text.as_ptr().cast_mut().cast::<libc::c_char>();
    let mut input_left = text.len();
    let mut output = vec![0_u8; text.len().saturating_mul(4).max(16)];
    let mut output_ptr = output.as_mut_ptr().cast::<libc::c_char>();
    let mut output_left = output.len();
    let result = unsafe {
        libc::iconv(
            converter,
            &mut input,
            &mut input_left,
            &mut output_ptr,
            &mut output_left,
        )
    };
    unsafe { libc::iconv_close(converter) };
    if result == usize::MAX || input_left != 0 {
        return None;
    }

    output.truncate(output.len() - output_left);
    Some(output)
}

fn quote_whoami_operand(operand: &OsStr, simplified_chinese: bool) -> Vec<u8> {
    if simplified_chinese {
        #[cfg(target_os = "linux")]
        if let Some(codeset) = whoami_ctype_codeset() {
            return quote_whoami_locale_encoded_operand(operand, &codeset, b"\"", b"\"");
        }
        quote_whoami_utf8_operand_with_quotes(operand, b"\"", b"\"", Some(b'\"'))
    } else if whoami_locale_is_utf8() {
        quote_whoami_utf8_operand(operand)
    } else {
        #[cfg(target_os = "linux")]
        if let Some(codeset) = whoami_ctype_codeset() {
            if whoami_gb18030_codeset(&codeset) {
                return quote_whoami_locale_encoded_operand(
                    operand,
                    &codeset,
                    GNU_GB18030_FALLBACK_LEFT_QUOTE,
                    GNU_GB18030_FALLBACK_RIGHT_QUOTE,
                );
            }
            return quote_whoami_locale_encoded_operand(operand, &codeset, b"'", b"'");
        }
        quote_whoami_c_operand(operand)
    }
}

#[cfg(target_os = "linux")]
// GNU's "\xa1\ae" source literal expands \a as BEL and leaves the trailing e.
const GNU_GB18030_FALLBACK_LEFT_QUOTE: &[u8] = b"\xa1\x07e";
#[cfg(target_os = "linux")]
const GNU_GB18030_FALLBACK_RIGHT_QUOTE: &[u8] = b"\xa1\xaf";

#[cfg(target_os = "linux")]
fn whoami_gb18030_codeset(codeset: &str) -> bool {
    codeset.eq_ignore_ascii_case("GB18030")
}

fn whoami_locale_is_utf8() -> bool {
    #[cfg(target_os = "linux")]
    if !whoami_locale_environment_is_valid() {
        return false;
    }

    whoami_effective_locale(["LC_ALL", "LC_CTYPE", "LANG"]).is_some_and(|value| {
        let value = value.to_string_lossy().to_ascii_uppercase();
        value.contains("UTF-8") || value.contains("UTF8")
    })
}

fn quote_whoami_c_operand(operand: &OsStr) -> Vec<u8> {
    let mut quoted = Vec::with_capacity(operand.as_encoded_bytes().len() + 2);
    quoted.push(b'\'');
    for byte in operand.as_encoded_bytes() {
        push_whoami_quoted_ascii(&mut quoted, *byte, Some(b'\''));
    }
    quoted.push(b'\'');
    quoted
}

fn quote_whoami_utf8_operand(operand: &OsStr) -> Vec<u8> {
    quote_whoami_utf8_operand_with_quotes(operand, "‘".as_bytes(), "’".as_bytes(), None)
}

fn quote_whoami_utf8_operand_with_quotes(
    operand: &OsStr,
    left_quote: &[u8],
    right_quote: &[u8],
    quote_to_escape: Option<u8>,
) -> Vec<u8> {
    let input = operand.as_encoded_bytes();
    let mut quoted = Vec::with_capacity(input.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(left_quote);

    let mut index = 0;
    while index < input.len() {
        if input[index..].starts_with(right_quote) {
            quoted.push(b'\\');
            quoted.extend_from_slice(right_quote);
            index += right_quote.len();
            continue;
        }

        if input[index].is_ascii() {
            push_whoami_quoted_ascii(&mut quoted, input[index], quote_to_escape);
            index += 1;
            continue;
        }

        match std::str::from_utf8(&input[index..]) {
            Ok(_) => {
                quoted.extend_from_slice(&input[index..]);
                break;
            }
            Err(error) if error.valid_up_to() > 0 => {
                let end = index + error.valid_up_to();
                quoted.extend_from_slice(&input[index..end]);
                index = end;
            }
            Err(error) => {
                let invalid_length = error.error_len().unwrap_or(input.len() - index);
                for byte in &input[index..index + invalid_length] {
                    push_whoami_octal_escape(&mut quoted, *byte);
                }
                index += invalid_length;
            }
        }
    }

    quoted.extend_from_slice(right_quote);
    quoted
}

#[cfg(target_os = "linux")]
fn quote_whoami_locale_encoded_operand(
    operand: &OsStr,
    codeset: &str,
    left_quote: &[u8],
    right_quote: &[u8],
) -> Vec<u8> {
    let input = operand.as_encoded_bytes();
    let mut quoted = Vec::with_capacity(input.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(left_quote);

    let mut index = 0;
    while index < input.len() {
        if input[index..].starts_with(right_quote) {
            quoted.push(b'\\');
            quoted.extend_from_slice(right_quote);
            index += right_quote.len();
            continue;
        }

        if input[index].is_ascii() {
            push_whoami_quoted_ascii(
                &mut quoted,
                input[index],
                (right_quote.len() == 1).then_some(right_quote[0]),
            );
            index += 1;
            continue;
        }

        let valid_character_len = whoami_valid_locale_character_len(&input[index..], codeset);
        if valid_character_len == 0 {
            push_whoami_octal_escape(&mut quoted, input[index]);
            index += 1;
        } else {
            quoted.extend_from_slice(&input[index..index + valid_character_len]);
            index += valid_character_len;
        }
    }

    quoted.extend_from_slice(right_quote);
    quoted
}

#[cfg(target_os = "linux")]
fn whoami_valid_locale_character_len(input: &[u8], codeset: &str) -> usize {
    for length in 1..=input.len().min(4) {
        if whoami_locale_bytes_are_valid(&input[..length], codeset) {
            return length;
        }
    }
    0
}

#[cfg(target_os = "linux")]
fn whoami_locale_bytes_are_valid(input: &[u8], codeset: &str) -> bool {
    let source = match CString::new(codeset) {
        Ok(source) => source,
        Err(_) => return false,
    };
    let target = CString::new("UTF-8").expect("UTF-8 has no NUL byte");
    let converter = unsafe { libc::iconv_open(target.as_ptr(), source.as_ptr()) };
    if converter == (-1_isize) as libc::iconv_t {
        return false;
    }

    let mut input_ptr = input.as_ptr().cast_mut().cast::<libc::c_char>();
    let mut input_left = input.len();
    let mut output = vec![0_u8; input.len().saturating_mul(4).max(16)];
    let mut output_ptr = output.as_mut_ptr().cast::<libc::c_char>();
    let mut output_left = output.len();
    let result = unsafe {
        libc::iconv(
            converter,
            &mut input_ptr,
            &mut input_left,
            &mut output_ptr,
            &mut output_left,
        )
    };
    unsafe { libc::iconv_close(converter) };
    result != usize::MAX && input_left == 0
}

fn push_whoami_quoted_ascii(output: &mut Vec<u8>, byte: u8, quote_to_escape: Option<u8>) {
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
        _ => push_whoami_octal_escape(output, byte),
    }
}

fn push_whoami_octal_escape(output: &mut Vec<u8>, byte: u8) {
    output.push(b'\\');
    output.push(b'0' + (byte >> 6));
    output.push(b'0' + ((byte >> 3) & 7));
    output.push(b'0' + (byte & 7));
}

/// 获取当前用户名
#[cfg(unix)]
pub fn whoami_exec() -> CTResult<OsString> {
    let uid = unsafe { libc::geteuid() };
    platform::get_username().map_err(|error| whoami_unknown_uid_error(uid, &error))
}

#[cfg(unix)]
fn whoami_unknown_uid_error(uid: libc::uid_t, error: &io::Error) -> Box<dyn CTError> {
    WhoamiRuntimeError::boxed(whoami_unknown_uid_message(
        uid,
        error,
        whoami_uses_simplified_chinese(),
    ))
}

#[cfg(unix)]
fn whoami_unknown_uid_message(
    uid: libc::uid_t,
    error: &io::Error,
    simplified_chinese: bool,
) -> Vec<u8> {
    let mut message = if simplified_chinese {
        whoami_encode_locale_text(&format!("无法找到 ID 为 {uid} 的用户的名称"))
    } else {
        format!("cannot find name for user ID {uid}").into_bytes()
    };
    if !matches!(error.raw_os_error(), Some(0) | None) {
        message.extend_from_slice(b": ");
        message.extend_from_slice(strip_errno(error).as_bytes());
    }
    message
}

#[cfg(not(unix))]
pub fn whoami_exec() -> CTResult<OsString> {
    platform::get_username().map_err_context(|| t!("whoami.errors.failed_get_username"))
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("whoami.about");
    let usage_description = t!("whoami.usage");

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            clap::Arg::new("help")
                .short('h')
                .long("help")
                .help(t!("whoami.clap.help"))
                .action(clap::ArgAction::Help),
        )
        .arg(
            clap::Arg::new("version")
                .short('V')
                .long("version")
                .help(t!("whoami.clap.version"))
                .action(clap::ArgAction::Version),
        )
}

#[derive(Default)]
pub struct Whoami;
impl Tool for Whoami {
    fn name(&self) -> &'static str {
        "whoami"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        whoami_main(args.iter().cloned()).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use std::sync::Mutex;

    use super::*;
    rust_i18n::i18n!("locales", fallback = "en-US");

    static LOCALE_LOCK: Mutex<()> = Mutex::new(());

    fn with_lc_all<T>(locale: &str, test: impl FnOnce() -> T) -> T {
        let _guard = LOCALE_LOCK.lock().unwrap();
        let previous = std::env::var_os("LC_ALL");
        unsafe { std::env::set_var("LC_ALL", locale) };
        let result = test();
        match previous {
            Some(value) => unsafe { std::env::set_var("LC_ALL", value) },
            None => unsafe { std::env::remove_var("LC_ALL") },
        }
        result
    }

    fn with_locale_variables<T>(variables: &[(&str, Option<&str>)], test: impl FnOnce() -> T) -> T {
        let _guard = LOCALE_LOCK.lock().unwrap();
        let previous = variables
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();
        for (name, value) in variables {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
        let result = test();
        for (name, value) in previous {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
        result
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Whoami;

        // Test name method
        assert_eq!(tool.name(), "whoami");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("whoami"));

        // Test execute method with help flag (should work)
        let args = vec![OsString::from("whoami"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use clap::error::ErrorKind;
        use std::ffi::OsString;

        #[test]
        fn test_ctmain_input_h() {
            {
                let args = ["-h", ""];
                let result = whoami_main(args.iter().map(OsString::from));
                assert!(result.is_err());
            }

            {
                let command = ct_app();
                let args = vec![ctcore::ct_util_name(), "-h"];

                let result = command.try_get_matches_from(args);
                assert!(result.is_err());
                assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
            }
        }

        #[test]
        fn test_ctmain_input_v() {
            {
                let args = ["--version", ""];
                let result = whoami_main(args.iter().map(OsString::from));
                assert!(result.is_err());
            }
            {
                let command = ct_app();
                let args = vec![ctcore::ct_util_name(), "--version"];
                // let result = ct_main(args.iter().map(|s| OsString::from(s)));

                let result = command.try_get_matches_from(args);
                assert!(result.is_err());
                assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
            }
        }

        #[test]
        fn test_ctmain_input_uppercase_v() {
            {
                let args = ["-V", ""];
                let result = whoami_main(args.iter().map(OsString::from));
                assert!(result.is_err());
            }
            {
                let command = ct_app();
                let args = vec![ctcore::ct_util_name(), "-V"];
                // let result = ct_main(args.iter().map(|s| OsString::from(s)));

                let result = command.try_get_matches_from(args);
                assert!(result.is_err());
                assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
            }
        }

        #[test]
        fn test_ctmain_return() {
            // println!("当前操作系统架构：{}", expected_arch);
            let expected = if let Ok(username) = std::env::var("USER") {
                username
            } else {
                "root".to_string()
            };
            let args = [ctcore::ct_util_name()];
            let result = whoami_main(args.iter().map(OsString::from));
            let mut s = String::new();
            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {code}");
                    println!("Error message: {message}");
                }
                Ok(output) => {
                    s = output.to_string();
                    println!("result:{s}");
                    // //assert_eq!(s,expected_output);
                }
            }
            assert_eq!(s, expected);
        }
    }

    ///////////////////////////////

    // whoami 接口: whoami [OPTION]...
    //       --help     display this help and exit
    //       --version  output version information and exit
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
    fn whoami_reports_extra_operand_with_gnu_diagnostic() {
        with_lc_all("C", || {
            let error =
                whoami_main([OsString::from("whoami"), OsString::from("operand")].into_iter())
                    .expect_err("whoami must reject an operand");

            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"extra operand 'operand'"
            );
            assert!(error.usage());
        });
    }

    #[test]
    fn whoami_preparse_reports_gnu_standard_option_errors() {
        let cases = [
            (
                "--help=x",
                b"option '--help' doesn't allow an argument".as_slice(),
            ),
            ("--unknown", b"unrecognized option '--unknown'".as_slice()),
            ("-x", b"invalid option -- 'x'".as_slice()),
            (
                "--=",
                b"option '--=' is ambiguous; possibilities: '--help' '--version'".as_slice(),
            ),
        ];

        for (argument, expected) in cases {
            let error = prepare_whoami_args_with_mode(
                [OsString::from("whoami"), OsString::from(argument)].into_iter(),
                false,
            )
            .expect_err("GNU option error must be detected before clap");

            assert_eq!(error.diagnostic_bytes().as_ref(), expected);
            assert!(error.usage());
        }
    }

    #[test]
    fn whoami_preparse_honors_gnu_option_scan_order() {
        let args = [
            OsString::from("whoami"),
            OsString::from("operand"),
            OsString::from("--version"),
        ];
        let prepared = prepare_whoami_args_with_mode(args.clone().into_iter(), false).unwrap();
        assert_eq!(
            prepared,
            [OsString::from("whoami"), OsString::from("--version")]
        );

        with_lc_all("C", || {
            let error = prepare_whoami_args_with_mode(args.into_iter(), true)
                .expect_err("POSIXLY_CORRECT must stop option scanning at the operand");
            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"extra operand 'operand'"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn whoami_preparse_preserves_raw_unknown_option_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let error = prepare_whoami_args_with_mode(
            [
                OsString::from("whoami"),
                OsString::from_vec(vec![b'-', b'-', 0xff]),
            ]
            .into_iter(),
            false,
        )
        .expect_err("unknown options must be rejected");

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"unrecognized option '--\xff'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn whoami_write_errors_use_gnu_diagnostic_text() {
        assert_eq!(
            whoami_write_error_message(&io::Error::from_raw_os_error(libc::ENOSPC)),
            b"write error: No space left on device"
        );
        assert_eq!(
            whoami_write_error_message(&io::Error::from_raw_os_error(libc::EPIPE)),
            b"write error: Broken pipe"
        );
    }

    #[cfg(unix)]
    #[test]
    fn whoami_closed_stdout_is_reported_as_ebadf() {
        assert_eq!(
            whoami_closed_stdout_error(true)
                .expect("a closed stdout requires an error")
                .raw_os_error(),
            Some(libc::EBADF)
        );
        assert!(whoami_closed_stdout_error(false).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn whoami_unknown_uid_uses_gnu_diagnostic() {
        let error = whoami_unknown_uid_error(60_000, &io::Error::from(io::ErrorKind::NotFound));

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"cannot find name for user ID 60000"
        );
    }

    #[cfg(unix)]
    #[test]
    fn whoami_nss_error_appends_gnu_errno_text() {
        let error = whoami_unknown_uid_error(0, &io::Error::from_raw_os_error(libc::ENOENT));

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"cannot find name for user ID 0: No such file or directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn whoami_zh_cn_diagnostics_match_gnu() {
        let _guard = LOCALE_LOCK.lock().unwrap();
        let operand = OsString::from("a\"b");

        assert_eq!(
            whoami_extra_operand_message(&operand, true),
            "多余的操作对象 \"a\\\"b\"".as_bytes()
        );
        assert_eq!(
            whoami_usage_hint("whoami", true),
            "请尝试执行 \"whoami --help\" 来获取更多信息。".as_bytes()
        );
        assert_eq!(
            whoami_write_error_message_for_locale(
                &io::Error::from_raw_os_error(libc::ENOSPC),
                true,
            ),
            "写入错误: No space left on device".as_bytes()
        );
        assert_eq!(
            whoami_unknown_uid_message(60_000, &io::Error::from(io::ErrorKind::NotFound), true),
            "无法找到 ID 为 60000 的用户的名称".as_bytes()
        );
        assert!(whoami_simplified_chinese_locale(OsStr::new("zh_CN.UTF-8")));
        assert!(!whoami_simplified_chinese_locale(OsStr::new("zh_TW.UTF-8")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn whoami_zh_cn_gbk_diagnostic_text_uses_locale_encoding() {
        assert_eq!(
            whoami_encode_locale_text_for_codeset("多余的操作对象", "GBK"),
            Some(vec![
                0xb6, 0xe0, 0xd3, 0xe0, 0xb5, 0xc4, 0xb2, 0xd9, 0xd7, 0xf7, 0xb6, 0xd4, 0xcf, 0xf3,
            ])
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn whoami_zh_cn_gbk_quote_preserves_valid_multibyte_operand_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let operand = OsString::from_vec(vec![0xd6, 0xd0, 0xb9, 0xfa, 0xff]);

        assert_eq!(
            quote_whoami_locale_encoded_operand(&operand, "GBK", b"\"", b"\""),
            b"\"\xd6\xd0\xb9\xfa\\377\""
        );

        let ascii_trailing_byte = OsString::from_vec(vec![0x81, b'@']);
        assert_eq!(
            quote_whoami_locale_encoded_operand(&ascii_trailing_byte, "GBK", b"\"", b"\""),
            b"\"\x81@\""
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn whoami_non_utf8_locale_preserves_valid_single_byte_operand() {
        use std::os::unix::ffi::OsStrExt;

        with_lc_all("en_US", || {
            assert_eq!(whoami_output_codeset().as_deref(), Some("ISO-8859-1"));
            assert_eq!(
                quote_whoami_operand(OsStr::from_bytes(b"\xe9"), false),
                b"'\xe9'"
            );
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn whoami_gb18030_c_messages_use_gnu_quote_fallback_bytes() {
        use std::os::unix::ffi::OsStrExt;

        let quoted = with_locale_variables(
            &[("LC_ALL", Some("zh_CN.gb18030")), ("LANGUAGE", Some("C"))],
            || quote_whoami_operand(OsStr::from_bytes(b"alpha"), false),
        );

        assert_eq!(quoted, b"\xa1\x07ealpha\xa1\xaf");

        let embedded_right_quote = with_locale_variables(
            &[("LC_ALL", Some("zh_CN.gb18030")), ("LANGUAGE", Some("C"))],
            || quote_whoami_operand(OsStr::from_bytes(b"\xa1\xaf"), false),
        );
        assert_eq!(embedded_right_quote, b"\xa1\x07e\\\xa1\xaf\xa1\xaf");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn whoami_invalid_output_charset_falls_back_to_gnu_c_diagnostics() {
        use std::os::unix::ffi::OsStrExt;

        let message = with_locale_variables(
            &[
                ("LC_ALL", Some("zh_CN.UTF-8")),
                ("LANGUAGE", Some("zh_CN")),
                ("OUTPUT_CHARSET", Some("INVALID")),
            ],
            || {
                let simplified_chinese = whoami_uses_simplified_chinese();
                whoami_extra_operand_message(OsStr::from_bytes(b"alpha"), simplified_chinese)
            },
        );

        assert_eq!(message, "extra operand ‘alpha’".as_bytes());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn whoami_invalid_locale_environment_uses_c_diagnostics() {
        use std::os::unix::ffi::OsStrExt;

        let (simplified_chinese, codeset, message) = with_lc_all("zh_CN.invalid", || {
            let simplified_chinese = whoami_uses_simplified_chinese();
            (
                simplified_chinese,
                whoami_output_codeset(),
                whoami_extra_operand_message(OsStr::from_bytes(b"\xff"), simplified_chinese),
            )
        });
        assert!(!simplified_chinese);
        assert_eq!(codeset.as_deref(), Some("ASCII"));
        assert_eq!(message, b"extra operand '\\377'");

        let _guard = LOCALE_LOCK.lock().unwrap();
        let previous = ["LC_ALL", "LC_CTYPE", "LC_MESSAGES", "LC_NUMERIC", "LANG"]
            .map(|name| (name, std::env::var_os(name)));
        unsafe {
            std::env::remove_var("LC_ALL");
            std::env::set_var("LC_CTYPE", "zh_CN");
            std::env::set_var("LC_MESSAGES", "zh_CN");
            std::env::set_var("LC_NUMERIC", "invalid");
            std::env::set_var("LANG", "C");
        }
        let simplified_chinese = whoami_uses_simplified_chinese();
        let codeset = whoami_output_codeset();
        for (name, value) in previous {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
        assert!(!simplified_chinese);
        assert_eq!(codeset.as_deref(), Some("ASCII"));
        drop(_guard);

        #[cfg(target_env = "gnu")]
        let (simplified_chinese, codeset) = with_locale_variables(
            &[
                ("LC_ALL", None),
                ("LC_CTYPE", Some("zh_CN")),
                ("LC_MESSAGES", Some("zh_CN")),
                ("LC_NUMERIC", None),
                ("LC_TIME", None),
                ("LC_COLLATE", None),
                ("LC_MONETARY", None),
                ("LC_PAPER", Some("invalid")),
                ("LC_NAME", None),
                ("LC_ADDRESS", None),
                ("LC_TELEPHONE", None),
                ("LC_MEASUREMENT", None),
                ("LC_IDENTIFICATION", None),
                ("LANG", Some("C")),
            ],
            || (whoami_uses_simplified_chinese(), whoami_output_codeset()),
        );
        #[cfg(target_env = "gnu")]
        assert!(!simplified_chinese);
        #[cfg(target_env = "gnu")]
        assert_eq!(codeset.as_deref(), Some("ASCII"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn whoami_language_overrides_message_locale() {
        let simplified_chinese = with_locale_variables(
            &[("LC_ALL", Some("zh_CN.UTF-8")), ("LANGUAGE", Some("C"))],
            whoami_uses_simplified_chinese,
        );
        assert!(!simplified_chinese);

        let simplified_chinese = with_locale_variables(
            &[("LC_ALL", Some("en_US")), ("LANGUAGE", Some("zh_CN"))],
            whoami_uses_simplified_chinese,
        );
        assert!(simplified_chinese);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn whoami_language_skips_unavailable_candidates() {
        let simplified_chinese = with_locale_variables(
            &[
                ("LC_ALL", Some("zh_CN.UTF-8")),
                ("LANGUAGE", Some("does_NOT_exist:zh_CN")),
            ],
            whoami_uses_simplified_chinese,
        );

        assert!(simplified_chinese);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn whoami_output_charset_encodes_text_without_reinterpreting_operands() {
        use std::os::unix::ffi::OsStrExt;

        let (codeset, quoted_operand) = with_locale_variables(
            &[
                ("LC_ALL", Some("zh_CN.UTF-8")),
                ("OUTPUT_CHARSET", Some("GBK")),
            ],
            || {
                (
                    whoami_output_codeset(),
                    quote_whoami_operand(OsStr::from_bytes(b"\xe4\xb8\xad"), true),
                )
            },
        );
        assert_eq!(codeset.as_deref(), Some("GBK"));
        assert_eq!(quoted_operand, b"\"\xe4\xb8\xad\"");
    }
}
