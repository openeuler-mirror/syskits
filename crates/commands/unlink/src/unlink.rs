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

//! unlink 命令用于删除一个文件或者一个文件的硬链接

extern crate rust_i18n;
use clap::builder::ValueParser;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, Command, crate_version};

use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, strip_errno};
#[cfg(target_os = "linux")]
use ctcore::ct_quoting_style::escape_shell_bytes_with_classifier;

use std::borrow::Cow;
use std::error::Error;
#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString};
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::fs::remove_file;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use sys_locale::get_locale;

static OPT_PATH: &str = "FILE";
const UNLINK_LONG_OPTIONS: &[&str] = &["help", "version"];

#[derive(Debug, PartialEq, Eq)]
enum UnlinkLongOptionMatch {
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
    None,
}

#[derive(Debug)]
struct UnlinkUsageError {
    message: Vec<u8>,
    usage_hint: Vec<u8>,
}

impl UnlinkUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        let simplified_chinese = unlink_uses_simplified_chinese();
        Box::new(Self {
            message,
            usage_hint: unlink_usage_hint(simplified_chinese),
        })
    }
}

impl Display for UnlinkUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for UnlinkUsageError {}

impl CTError for UnlinkUsageError {
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

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct UnlinkRuntimeError {
    message: Vec<u8>,
}

#[cfg(target_os = "linux")]
impl UnlinkRuntimeError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

#[cfg(target_os = "linux")]
impl Display for UnlinkRuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

#[cfg(target_os = "linux")]
impl Error for UnlinkRuntimeError {}

#[cfg(target_os = "linux")]
impl CTError for UnlinkRuntimeError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn mbrtowc(
        wide: *mut ctcore::libc::wchar_t,
        bytes: *const ctcore::libc::c_char,
        length: usize,
        state: *mut ctcore::libc::mbstate_t,
    ) -> usize;
    fn iswprint(wide: ctcore::libc::c_uint) -> ctcore::libc::c_int;
}

pub fn unlink_main(args: impl ctcore::Args) -> CTResult<()> {
    #[cfg(target_os = "linux")]
    initialize_unlink_locale();

    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(prepare_unlink_args(args)?)?;

    let path: &Path = matches.get_one::<OsString>(OPT_PATH).unwrap().as_ref();

    unlink_path(path)
}

#[cfg(target_os = "linux")]
fn initialize_unlink_locale() {
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
    }
}

#[cfg(target_os = "linux")]
fn unlink_path(path: &Path) -> CTResult<()> {
    remove_file(path).map_err(|error| unlink_io_error(path, error))
}

#[cfg(not(target_os = "linux"))]
fn unlink_path(path: &Path) -> CTResult<()> {
    remove_file(path).map_err(Into::into)
}

#[cfg(target_os = "linux")]
fn unlink_io_error(path: &Path, error: std::io::Error) -> Box<dyn CTError> {
    let mut message = b"cannot unlink ".to_vec();
    message.extend_from_slice(&unlink_quote_path(path.as_os_str()));
    message.extend_from_slice(b": ");
    message.extend_from_slice(strip_errno(&error).as_bytes());
    UnlinkRuntimeError::boxed(message)
}

#[cfg(target_os = "linux")]
fn unlink_quote_path(path: &OsStr) -> Vec<u8> {
    let bytes = path.as_bytes();
    let mut quoted = escape_shell_bytes_with_classifier(bytes, |remaining| unsafe {
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
    });

    if quoted.as_slice() == bytes {
        quoted.insert(0, b'\'');
        quoted.push(b'\'');
    }

    quoted
}

fn unlink_missing_operand_message() -> Vec<u8> {
    if unlink_uses_simplified_chinese() {
        unlink_encode_locale_text("缺少操作对象")
    } else {
        b"missing operand".to_vec()
    }
}

fn unlink_extra_operand_message(operand: &OsStr) -> Vec<u8> {
    unlink_extra_operand_message_for_locale(operand, unlink_uses_simplified_chinese())
}

fn unlink_extra_operand_message_for_locale(operand: &OsStr, simplified_chinese: bool) -> Vec<u8> {
    let mut message = if simplified_chinese {
        unlink_encode_locale_text("多余的操作对象 ")
    } else {
        b"extra operand ".to_vec()
    };
    message.extend(unlink_quote_operand(operand, simplified_chinese));
    message
}

fn unlink_usage_hint(simplified_chinese: bool) -> Vec<u8> {
    let utility_name = ctcore::ct_help_utility_name();
    if simplified_chinese {
        unlink_encode_locale_text(&format!(
            "请尝试执行 \"{utility_name} --help\" 来获取更多信息。"
        ))
    } else {
        format!("Try '{utility_name} --help' for more information.").into_bytes()
    }
}

fn unlink_uses_simplified_chinese() -> bool {
    #[cfg(target_os = "linux")]
    if !unlink_locale_environment_is_valid() {
        return false;
    }

    let simplified_chinese = unlink_message_locale()
        .is_some_and(|locale| unlink_simplified_chinese_locale(locale.as_os_str()));
    if !simplified_chinese {
        return false;
    }

    #[cfg(target_os = "linux")]
    if let Some(codeset) = unlink_output_codeset() {
        return unlink_encode_locale_text_for_codeset("多余的操作对象", &codeset).is_some();
    }

    true
}

fn unlink_message_locale() -> Option<OsString> {
    let locale = unlink_effective_locale(["LC_ALL", "LC_MESSAGES", "LANG"])?;
    if unlink_c_message_locale(locale.as_os_str()) {
        return Some(locale);
    }

    let Some(language) = std::env::var_os("LANGUAGE").filter(|language| !language.is_empty())
    else {
        return Some(locale);
    };
    for candidate in language
        .to_string_lossy()
        .split(':')
        .filter(|candidate| !candidate.is_empty())
    {
        if unlink_c_message_locale(OsStr::new(candidate)) {
            return Some(OsString::from("C"));
        }
        if unlink_simplified_chinese_locale(OsStr::new(candidate)) {
            return Some(OsString::from(candidate));
        }
    }

    Some(OsString::from("C"))
}

fn unlink_c_message_locale(locale: &OsStr) -> bool {
    matches!(
        locale.to_string_lossy().to_ascii_uppercase().as_str(),
        "C" | "POSIX"
    )
}

fn unlink_simplified_chinese_locale(locale: &OsStr) -> bool {
    let locale = locale.to_string_lossy();
    locale == "zh_CN" || locale.starts_with("zh_CN.") || locale.starts_with("zh_CN@")
}

fn unlink_effective_locale<const N: usize>(names: [&str; N]) -> Option<OsString> {
    names.into_iter().find_map(|name| {
        let value = std::env::var_os(name)?;
        (!value.is_empty()).then_some(value)
    })
}

fn unlink_encode_locale_text(text: &str) -> Vec<u8> {
    #[cfg(target_os = "linux")]
    if let Some(codeset) = unlink_output_codeset()
        && let Some(encoded) = unlink_encode_locale_text_for_codeset(text, &codeset)
    {
        return encoded;
    }

    text.as_bytes().to_vec()
}

#[cfg(target_os = "linux")]
fn unlink_output_codeset() -> Option<String> {
    if !unlink_locale_environment_is_valid() {
        return Some("ASCII".to_owned());
    }

    if let Some(codeset) = std::env::var_os("OUTPUT_CHARSET").filter(|codeset| !codeset.is_empty())
    {
        return Some(codeset.to_string_lossy().into_owned());
    }

    unlink_ctype_codeset()
}

#[cfg(target_os = "linux")]
fn unlink_ctype_codeset() -> Option<String> {
    let locale = unlink_effective_locale(["LC_ALL", "LC_CTYPE", "LANG"])?;
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
        .or_else(|| unlink_locale_codeset(locale.as_os_str()))
}

#[cfg(target_os = "linux")]
fn unlink_locale_codeset(locale: &OsStr) -> Option<String> {
    let locale = CString::new(locale.as_encoded_bytes()).ok()?;
    let locale_handle = unsafe {
        ctcore::libc::newlocale(
            ctcore::libc::LC_CTYPE_MASK,
            locale.as_ptr(),
            std::ptr::null_mut(),
        )
    };
    if locale_handle.is_null() {
        return None;
    }

    let codeset = unsafe {
        let codeset = ctcore::libc::nl_langinfo_l(ctcore::libc::CODESET, locale_handle);
        (!codeset.is_null()).then(|| CStr::from_ptr(codeset).to_bytes().to_vec())
    };
    unsafe { ctcore::libc::freelocale(locale_handle) };
    codeset.and_then(|codeset| String::from_utf8(codeset).ok())
}

#[cfg(target_os = "linux")]
fn unlink_locale_environment_is_valid() -> bool {
    let locale_handle = unsafe {
        ctcore::libc::newlocale(
            ctcore::libc::LC_ALL_MASK,
            c"".as_ptr(),
            std::ptr::null_mut(),
        )
    };
    if locale_handle.is_null() {
        return false;
    }
    unsafe { ctcore::libc::freelocale(locale_handle) };
    true
}

#[cfg(target_os = "linux")]
fn unlink_encode_locale_text_for_codeset(text: &str, codeset: &str) -> Option<Vec<u8>> {
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

#[cfg(target_os = "linux")]
fn unlink_quote_operand(operand: &OsStr, simplified_chinese: bool) -> Vec<u8> {
    let (left_quote, right_quote, quote_to_escape) = if simplified_chinese {
        (b"\"".as_slice(), b"\"".as_slice(), Some(b'\"'))
    } else if unlink_ctype_is_utf8() {
        ("‘".as_bytes(), "’".as_bytes(), None)
    } else {
        (b"'".as_slice(), b"'".as_slice(), Some(b'\''))
    };
    unlink_quote_operand_bytes(operand.as_bytes(), left_quote, right_quote, quote_to_escape)
}

#[cfg(not(target_os = "linux"))]
fn unlink_quote_operand(operand: &OsStr, _simplified_chinese: bool) -> Vec<u8> {
    unlink_quote_c_operand(operand)
}

#[cfg(target_os = "linux")]
fn unlink_ctype_is_utf8() -> bool {
    let locale = unsafe { ctcore::libc::setlocale(ctcore::libc::LC_CTYPE, std::ptr::null()) };
    if locale.is_null() {
        return false;
    }
    let locale = unsafe { CStr::from_ptr(locale) }.to_string_lossy();
    let locale = locale.to_ascii_uppercase();
    locale.contains("UTF-8") || locale.contains("UTF8")
}

#[cfg(target_os = "linux")]
fn unlink_quote_operand_bytes(
    input: &[u8],
    left_quote: &[u8],
    right_quote: &[u8],
    quote_to_escape: Option<u8>,
) -> Vec<u8> {
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
            unlink_push_quoted_ascii(&mut quoted, input[index], quote_to_escape);
            index += 1;
            continue;
        }

        let (length, printable) = unlink_classify_locale_sequence(&input[index..]);
        if printable {
            quoted.extend_from_slice(&input[index..index + length]);
        } else {
            for byte in &input[index..index + length] {
                unlink_push_octal_escape(&mut quoted, *byte);
            }
        }
        index += length;
    }

    quoted.extend_from_slice(right_quote);
    quoted
}

#[cfg(target_os = "linux")]
fn unlink_classify_locale_sequence(remaining: &[u8]) -> (usize, bool) {
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

#[cfg(target_os = "linux")]
fn unlink_push_quoted_ascii(output: &mut Vec<u8>, byte: u8, quote_to_escape: Option<u8>) {
    match byte {
        b'\x07' => output.extend_from_slice(b"\\a"),
        b'\x08' => output.extend_from_slice(b"\\b"),
        b'\t' => output.extend_from_slice(b"\\t"),
        b'\n' => output.extend_from_slice(b"\\n"),
        b'\x0b' => output.extend_from_slice(b"\\v"),
        b'\x0c' => output.extend_from_slice(b"\\f"),
        b'\r' => output.extend_from_slice(b"\\r"),
        b'\\' => output.extend_from_slice(b"\\\\"),
        byte if quote_to_escape == Some(byte) => {
            output.push(b'\\');
            output.push(byte);
        }
        b' '..=b'~' => output.push(byte),
        _ => unlink_push_octal_escape(output, byte),
    }
}

#[cfg(target_os = "linux")]
fn unlink_push_octal_escape(output: &mut Vec<u8>, byte: u8) {
    output.push(b'\\');
    output.push(b'0' + (byte >> 6));
    output.push(b'0' + ((byte >> 3) & 7));
    output.push(b'0' + (byte & 7));
}

fn prepare_unlink_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    prepare_unlink_args_with_mode(args, ctcore::ct_posix::posixly_correct())
}

fn prepare_unlink_args_with_mode(
    args: impl ctcore::Args,
    posixly_correct: bool,
) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut parse_options = true;
    let mut operands = Vec::new();

    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if parse_options && bytes == b"--" {
            parse_options = false;
            continue;
        }

        if parse_options && bytes.len() > 1 && bytes[0] == b'-' {
            if bytes.starts_with(b"--") {
                if validate_unlink_long_option(bytes)? {
                    let mut terminal_args = Vec::with_capacity(2);
                    if let Some(program) = args.first() {
                        terminal_args.push(program.clone());
                    }
                    terminal_args.push(argument.clone());
                    return Ok(terminal_args);
                }
                continue;
            }

            // Keep the syskits -h/-V extensions outside GNU option handling.
            if bytes == b"-h" || bytes == b"-V" {
                return Ok(args);
            }

            let mut message = b"invalid option -- '".to_vec();
            message.push(bytes[1]);
            message.push(b'\'');
            return Err(UnlinkUsageError::boxed(message));
        }

        operands.push(argument);
        if posixly_correct {
            parse_options = false;
        }
    }

    match operands.as_slice() {
        [] => Err(UnlinkUsageError::boxed(unlink_missing_operand_message())),
        [_] => Ok(args),
        [_, extra, ..] => Err(UnlinkUsageError::boxed(unlink_extra_operand_message(extra))),
    }
}

fn validate_unlink_long_option(argument: &[u8]) -> CTResult<bool> {
    let long = &argument[2..];
    let separator = long.iter().position(|byte| *byte == b'=');
    let name = &long[..separator.unwrap_or(long.len())];

    match match_unlink_long_option(name) {
        UnlinkLongOptionMatch::None => {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(argument);
            message.push(b'\'');
            Err(UnlinkUsageError::boxed(message))
        }
        UnlinkLongOptionMatch::Ambiguous(candidates) => {
            let mut message = b"option '".to_vec();
            message.extend_from_slice(argument);
            message.extend_from_slice(b"' is ambiguous; possibilities:");
            for candidate in candidates {
                message.extend_from_slice(b" '--");
                message.extend_from_slice(candidate.as_bytes());
                message.push(b'\'');
            }
            Err(UnlinkUsageError::boxed(message))
        }
        UnlinkLongOptionMatch::Recognized(canonical) if separator.is_some() => {
            Err(UnlinkUsageError::boxed(
                format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
            ))
        }
        UnlinkLongOptionMatch::Recognized("help" | "version") => Ok(true),
        UnlinkLongOptionMatch::Recognized(_) => unreachable!("unlink only has standard options"),
    }
}

fn match_unlink_long_option(name: &[u8]) -> UnlinkLongOptionMatch {
    if let Some(option) = UNLINK_LONG_OPTIONS
        .iter()
        .copied()
        .find(|option| option.as_bytes() == name)
    {
        return UnlinkLongOptionMatch::Recognized(option);
    }

    let candidates = UNLINK_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|option| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [] => UnlinkLongOptionMatch::None,
        [candidate] => UnlinkLongOptionMatch::Recognized(candidate),
        _ => UnlinkLongOptionMatch::Ambiguous(candidates),
    }
}

#[cfg(not(target_os = "linux"))]
fn unlink_quote_c_operand(operand: &OsStr) -> Vec<u8> {
    let mut quoted = Vec::with_capacity(operand.as_encoded_bytes().len() + 2);
    quoted.push(b'\'');
    for byte in operand.as_encoded_bytes() {
        match *byte {
            b'\x07' => quoted.extend_from_slice(b"\\a"),
            b'\x08' => quoted.extend_from_slice(b"\\b"),
            b'\t' => quoted.extend_from_slice(b"\\t"),
            b'\n' => quoted.extend_from_slice(b"\\n"),
            b'\x0b' => quoted.extend_from_slice(b"\\v"),
            b'\x0c' => quoted.extend_from_slice(b"\\f"),
            b'\r' => quoted.extend_from_slice(b"\\r"),
            b'\\' => quoted.extend_from_slice(b"\\\\"),
            b'\'' => quoted.extend_from_slice(b"\\'"),
            b' '..=b'~' => quoted.push(*byte),
            _ => {
                quoted.push(b'\\');
                quoted.push(b'0' + (byte >> 6));
                quoted.push(b'0' + ((byte >> 3) & 7));
                quoted.push(b'0' + (byte & 7));
            }
        }
    }
    quoted.push(b'\'');
    quoted
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("unlink.about");
    let usage_description = t!("unlink.usage");
    let arg = Arg::new(OPT_PATH)
        .required(true)
        .hide(true)
        .value_parser(ValueParser::os_string())
        .value_hint(clap::ValueHint::AnyPath);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
}

#[derive(Default)]
pub struct Unlink;
impl Tool for Unlink {
    fn name(&self) -> &'static str {
        "unlink"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        unlink_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctcore::Tool;
    use std::ffi::OsString;
    use std::sync::Mutex;

    static POSIXLY_CORRECT_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_tool_implementation() {
        let tool = Unlink;

        // 测试 name 方法
        assert_eq!(tool.name(), "unlink");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("unlink"));

        // 测试 execute 方法
        let args = vec![OsString::from("unlink"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err()); // unlink需要参数，没有参数会失败
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::fs::File;
        use std::os::unix::ffi::OsStringExt;
        use std::path::PathBuf;

        use super::*;

        #[test]
        fn test_unlink_main_argument_file_parsing() {
            let regular_file_path = "test_unlink_main_argument_file_parsing";
            File::create(regular_file_path).expect("Failed to create file");
            let args = [ctcore::ct_util_name(), regular_file_path];
            let result = unlink_main(args.iter().map(OsString::from));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {code}");
                    println!("Error message: {message}");
                }
                Ok(_output) => {
                    assert!(!PathBuf::from(regular_file_path).exists());
                }
            }
        }

        #[test]
        fn test_unlink_main_argument_no_file_parsing() {
            let regular_file_path = "test_no_unlink_file";

            let args = [ctcore::ct_util_name(), regular_file_path];
            let result = unlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_argument_default() {
            let args = [ctcore::ct_util_name()];
            let result = unlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = unlink_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = unlink_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = unlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = unlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = unlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()]; // 缺少任何参数
            let result = unlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn unlink_reports_gnu_missing_operand_diagnostic() {
            let args = [ctcore::ct_util_name()];
            let error = unlink_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(error.to_string(), "missing operand");
            assert!(error.usage());
        }

        #[test]
        fn unlink_reports_second_file_as_gnu_extra_operand() {
            let args = [ctcore::ct_util_name(), "first", "second"];
            let error = unlink_main(args.iter().map(OsString::from)).unwrap_err();

            assert!(error.to_string().starts_with("extra operand "));
            assert!(error.to_string().contains("second"));
            assert!(error.usage());
        }

        #[test]
        fn unlink_rejects_argument_attached_to_gnu_standard_option() {
            let args = [ctcore::ct_util_name(), "--help=value"];
            let error = unlink_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(
                error.to_string(),
                "option '--help' doesn't allow an argument"
            );
            assert!(error.usage());
        }

        #[test]
        fn unlink_posix_mode_stops_standard_option_scanning_at_file() {
            let _guard = POSIXLY_CORRECT_LOCK.lock().unwrap();
            let previous = std::env::var_os("POSIXLY_CORRECT");
            unsafe { std::env::set_var("POSIXLY_CORRECT", "1") };

            let args = [ctcore::ct_util_name(), "first", "--version"];
            let error = unlink_main(args.iter().map(OsString::from)).unwrap_err();

            match previous {
                Some(value) => unsafe { std::env::set_var("POSIXLY_CORRECT", value) },
                None => unsafe { std::env::remove_var("POSIXLY_CORRECT") },
            }

            assert!(error.to_string().starts_with("extra operand "));
            assert!(error.to_string().contains("--version"));
            assert!(error.usage());
        }

        #[test]
        fn unlink_uses_gnu_shell_escape_for_non_utf8_file_error() {
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from_vec(vec![0xff]),
            ];
            let error = unlink_main(args.into_iter()).unwrap_err();

            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"cannot unlink ''$'\\377': No such file or directory"
            );
        }

        #[test]
        fn unlink_formats_extra_operand_for_simplified_chinese_locale() {
            assert_eq!(
                unlink_extra_operand_message_for_locale(OsStr::new("second"), true),
                "多余的操作对象 \"second\"".as_bytes()
            );
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use std::fs;
        use std::fs::File;

        use clap::error::ErrorKind;

        use super::*;

        // unlink 接口, unlink FILE
        //       --help     display this help and exit
        //       --version  output version information and exit

        #[test]
        fn test_ct_app_argument_file_parsing() {
            // Create a file for testing , 默认带文件
            let regular_file_path = "test_ct_app_argument_file_parsing";
            File::create(regular_file_path).expect("Failed to create file");

            let command = ct_app();
            // 测试正确的文件路径参数解析
            let args = vec![ctcore::ct_util_name(), regular_file_path];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            fs::remove_file(regular_file_path).expect("Failed to remove file");
        }

        #[test]
        fn test_ct_app_argument_no_file_parsing() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name()];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
            assert_eq!(
                executable.unwrap_err().kind(),
                ErrorKind::MissingRequiredArgument
            );
        }

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];

            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
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
            assert!(result.is_err());
        }
    }
}
