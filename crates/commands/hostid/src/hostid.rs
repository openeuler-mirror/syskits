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
use clap::builder::ValueParser;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CtSimpleError, strip_errno};
#[cfg(all(target_os = "linux", not(test)))]
use ctcore::ct_signals::enable_pipe_errors;
use libc::c_long;
#[cfg(unix)]
use std::error::Error;
#[cfg(unix)]
use std::ffi::CStr;
use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::fmt::{self, Display, Formatter};
use std::io::{self, Write};
use std::path::Path;
use sys_locale::get_locale;

#[cfg(all(target_os = "linux", not(test)))]
static INHERITED_SIGPIPE_IGNORED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(all(target_os = "linux", not(test)))]
extern "C" fn record_inherited_sigpipe_disposition() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(libc::SIGPIPE, std::ptr::null(), &mut action) == 0
            && action.sa_sigaction == libc::SIG_IGN
        {
            INHERITED_SIGPIPE_IGNORED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[cfg(all(target_os = "linux", not(test)))]
#[used]
#[unsafe(link_section = ".preinit_array")]
static RECORD_INHERITED_SIGPIPE_DISPOSITION: extern "C" fn() = record_inherited_sigpipe_disposition;

#[cfg(all(target_os = "linux", not(test)))]
fn inherited_sigpipe_is_ignored() -> bool {
    INHERITED_SIGPIPE_IGNORED.load(std::sync::atomic::Ordering::Relaxed)
}

// currently rust libc interface doesn't include gethostid
unsafe extern "C" {
    pub unsafe fn gethostid() -> c_long;
    #[cfg(unix)]
    unsafe fn mbrtowc(
        wide_character: *mut libc::wchar_t,
        bytes: *const libc::c_char,
        byte_count: usize,
        state: *mut libc::mbstate_t,
    ) -> usize;
    #[cfg(unix)]
    unsafe fn iswprint(wide_character: libc::c_uint) -> libc::c_int;
    #[cfg(unix)]
    unsafe fn iconv_open(
        output_codeset: *const libc::c_char,
        input_codeset: *const libc::c_char,
    ) -> *mut libc::c_void;
    #[cfg(unix)]
    unsafe fn iconv(
        descriptor: *mut libc::c_void,
        input: *mut *mut libc::c_char,
        input_bytes_left: *mut usize,
        output: *mut *mut libc::c_char,
        output_bytes_left: *mut usize,
    ) -> usize;
    #[cfg(unix)]
    unsafe fn iconv_close(descriptor: *mut libc::c_void) -> libc::c_int;
}

const OPERAND_NAME: &str = "OPERAND";
const SYSKITS_HOSTID_LONG_VERSION: &str = concat!(
    "(syskits) ",
    env!("CARGO_PKG_VERSION"),
    "\nCopyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.\n\
License Mulan PSL v2: <http://license.coscl.org.cn/MulanPSL2>.\n\
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law."
);

#[derive(Default)]
pub struct Hostid;
impl Tool for Hostid {
    fn name(&self) -> &'static str {
        "hostid"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        hostid_main(args.iter().cloned())
    }
}

pub fn hostid_main(args: impl ctcore::Args) -> CTResult<()> {
    initialize_hostid_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    #[cfg(all(target_os = "linux", not(test)))]
    if !inherited_sigpipe_is_ignored() {
        enable_pipe_errors().map_err(|error| CtSimpleError::new(1, error.to_string()))?;
    }
    let args = args.collect::<Vec<_>>();
    match validate_standard_options(&args)? {
        Some(TerminalAction::Help) => {
            return write_hostid_output(&format!(
                "{}\n",
                hostid_help_text(&hostid_usage_name(&args))
            ));
        }
        Some(TerminalAction::Version) => {
            return write_hostid_output(&format!(
                "{}\n",
                hostid_version_text(&hostid_program_name(&args))
            ));
        }
        None => {}
    }
    let matches = ct_app().try_get_matches_from(hostid_clap_args(&args))?;
    reject_extra_operand(&matches, &args)?;
    write_hostid_output(&format!("{}\n", hostid()))
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("hostid.about");
    let usage_description = t!("hostid.usage");

    Command::new(utility_name)
        .display_name(ctcore::ct_help_utility_name())
        .version(command_version)
        .long_version(SYSKITS_HOSTID_LONG_VERSION)
        .about(application_info)
        .override_help(hostid_help_text(utility_name))
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(Arg::new("help").long("help").action(ArgAction::Help))
        .arg(
            Arg::new("version")
                .long("version")
                .action(ArgAction::Version),
        )
        .arg(
            Arg::new(OPERAND_NAME)
                .hide(true)
                .value_parser(ValueParser::os_string())
                .action(ArgAction::Append),
        )
}

fn hostid_help_text(usage_name: &str) -> String {
    format!(
        "\
Usage: {usage_name} [OPTION]
Print the numeric identifier (in hexadecimal) for the current host.

      --help        display this help and exit
      --version     output version information and exit

syskits online help: <https://www.ctyun.cn/developer>"
    )
}

fn hostid_version_text(program_name: &str) -> String {
    format!("{program_name} {SYSKITS_HOSTID_LONG_VERSION}")
}

fn hostid_usage_name(args: &[OsString]) -> String {
    args.first()
        .map(|arg| arg.to_string_lossy().into_owned())
        .filter(|arg| !arg.is_empty())
        .unwrap_or_else(|| ctcore::ct_util_name().to_owned())
}

fn hostid_program_name(args: &[OsString]) -> String {
    let usage_name = hostid_usage_name(args);

    Path::new(&usage_name)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or(usage_name)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalAction {
    Help,
    Version,
}

fn validate_standard_options(args: &[OsString]) -> CTResult<Option<TerminalAction>> {
    let mut options_ended = false;
    let posixly_correct = std::env::var_os("POSIXLY_CORRECT").is_some();

    for arg in args.iter().skip(1) {
        #[cfg(unix)]
        if !options_ended && arg.to_str().is_none() {
            use std::os::unix::ffi::OsStrExt;

            let bytes = arg.as_bytes();
            if bytes.starts_with(b"--") {
                return Err(raw_long_option_error(args, bytes));
            }
            if bytes.starts_with(b"-") && bytes.len() > 1 {
                return Err(raw_short_option_error(args, bytes[1]));
            }
        }

        let arg = arg.to_string_lossy();
        if options_ended {
            continue;
        }
        if arg == "--" {
            options_ended = true;
            continue;
        }
        if posixly_correct && (arg == "-" || !arg.starts_with('-')) {
            options_ended = true;
            continue;
        }
        if arg.starts_with("--") {
            if let Some(action) = validate_long_option(&arg, args)? {
                return Ok(Some(action));
            }
            continue;
        }
        if let Some(short) = arg.strip_prefix('-').filter(|short| !short.is_empty()) {
            let invalid = short.chars().next().unwrap();
            return Err(hostid_usage_error(
                args,
                format!("invalid option -- '{invalid}'"),
            ));
        }
    }

    Ok(None)
}

fn hostid_clap_args(args: &[OsString]) -> Vec<OsString> {
    if std::env::var_os("POSIXLY_CORRECT").is_none() {
        return args.to_vec();
    }

    let mut options_ended = false;
    let mut clap_args = Vec::with_capacity(args.len() + 1);

    for (index, arg) in args.iter().enumerate() {
        if index != 0 && !options_ended && !is_hostid_option(arg) {
            clap_args.push(OsString::from("--"));
            options_ended = true;
        }
        clap_args.push(arg.clone());
        if index != 0 && arg == "--" {
            options_ended = true;
        }
    }

    clap_args
}

#[cfg(unix)]
fn is_hostid_option(arg: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let bytes = arg.as_bytes();
    bytes.len() > 1 && bytes[0] == b'-'
}

#[cfg(not(unix))]
fn is_hostid_option(arg: &std::ffi::OsStr) -> bool {
    let arg = arg.to_string_lossy();
    arg.len() > 1 && arg.starts_with('-')
}

#[cfg(unix)]
#[derive(Debug)]
struct HostidRawOptionError {
    diagnostic: Vec<u8>,
}

#[cfg(unix)]
impl Display for HostidRawOptionError {
    fn fmt(&self, _formatter: &mut Formatter<'_>) -> fmt::Result {
        let stderr = io::stderr();
        let _ = stderr.lock().write_all(&self.diagnostic);
        Ok(())
    }
}

#[cfg(unix)]
impl Error for HostidRawOptionError {}

#[cfg(unix)]
impl ctcore::ct_error::CTError for HostidRawOptionError {
    fn code(&self) -> i32 {
        1
    }
}

#[cfg(unix)]
fn raw_short_option_error(args: &[OsString], invalid: u8) -> Box<dyn ctcore::ct_error::CTError> {
    let mut message = b"invalid option -- '".to_vec();
    message.push(invalid);
    message.push(b'\'');
    raw_usage_error(args, &message)
}

#[cfg(unix)]
fn raw_long_option_error(args: &[OsString], option: &[u8]) -> Box<dyn ctcore::ct_error::CTError> {
    let mut message = b"unrecognized option '".to_vec();
    message.extend_from_slice(option);
    message.push(b'\'');
    raw_usage_error(args, &message)
}

#[cfg(unix)]
fn raw_usage_error(args: &[OsString], message: &[u8]) -> Box<dyn ctcore::ct_error::CTError> {
    let program_name = args
        .first()
        .map(|arg| arg.as_os_str())
        .unwrap_or_else(|| std::ffi::OsStr::new(ctcore::ct_util_name()));
    raw_diagnostic_error(program_name, message, true)
}

#[cfg(unix)]
fn raw_diagnostic_error(
    program_name: &OsStr,
    message: &[u8],
    include_try_help: bool,
) -> Box<dyn ctcore::ct_error::CTError> {
    use std::os::unix::ffi::OsStrExt;

    let mut diagnostic = Vec::with_capacity(program_name.as_bytes().len() + message.len() + 64);
    diagnostic.extend_from_slice(program_name.as_bytes());
    diagnostic.extend_from_slice(b": ");
    diagnostic.extend_from_slice(message);
    diagnostic.push(b'\n');
    if include_try_help {
        append_hostid_try_help(&mut diagnostic, program_name.as_bytes());
    }

    Box::new(HostidRawOptionError { diagnostic })
}

#[cfg(unix)]
fn append_hostid_try_help(diagnostic: &mut Vec<u8>, program_name: &[u8]) {
    let (prefix, suffix) = match hostid_message_language() {
        HostidMessageLanguage::SimplifiedChinese => (
            encode_hostid_message("请尝试执行 \""),
            encode_hostid_message(" --help\" 来获取更多信息。\n"),
        ),
        HostidMessageLanguage::TraditionalChinese => (
            encode_hostid_message("請嘗試執行「"),
            encode_hostid_message(" --help」取得更多訊息。\n"),
        ),
        HostidMessageLanguage::Other => (
            b"Try '".to_vec(),
            b" --help' for more information.\n".to_vec(),
        ),
    };
    diagnostic.extend_from_slice(&prefix);
    diagnostic.extend_from_slice(program_name);
    diagnostic.extend_from_slice(&suffix);
}

fn hostid_usage_error(args: &[OsString], message: String) -> Box<dyn ctcore::ct_error::CTError> {
    hostid_usage_error_bytes(args, message.as_bytes())
}

fn hostid_usage_error_bytes(
    args: &[OsString],
    message: &[u8],
) -> Box<dyn ctcore::ct_error::CTError> {
    #[cfg(unix)]
    if args.first().is_some_and(|arg| arg.to_str().is_none())
        || std::str::from_utf8(message).is_err()
        || hostid_message_language() != HostidMessageLanguage::Other
    {
        return raw_usage_error(args, message);
    }

    let message = std::str::from_utf8(message).expect("non-Unix diagnostics must be UTF-8");
    CtSimpleError::new(
        1,
        format!(
            "{message}\nTry '{} --help' for more information.",
            hostid_usage_name(args)
        ),
    )
}

fn validate_long_option(arg: &str, args: &[OsString]) -> CTResult<Option<TerminalAction>> {
    let long = arg.strip_prefix("--").unwrap();
    let (name, has_value) = match long.split_once('=') {
        Some((name, _)) => (name, true),
        None => (long, false),
    };
    let matches = ["help", "version"]
        .into_iter()
        .filter(|option| option.starts_with(name))
        .collect::<Vec<_>>();

    let message = match matches.as_slice() {
        [option] if has_value => format!("option '--{option}' doesn't allow an argument"),
        ["help"] => return Ok(Some(TerminalAction::Help)),
        ["version"] => return Ok(Some(TerminalAction::Version)),
        [] => format!("unrecognized option '{arg}'"),
        _ => format!("option '{arg}' is ambiguous; possibilities: '--help' '--version'"),
    };

    Err(hostid_usage_error(args, message))
}

fn reject_extra_operand(matches: &clap::ArgMatches, args: &[OsString]) -> CTResult<()> {
    if let Some(operand) = matches
        .get_many::<OsString>(OPERAND_NAME)
        .and_then(|operands| operands.into_iter().next())
    {
        #[cfg(unix)]
        let mut message = match hostid_message_language() {
            HostidMessageLanguage::SimplifiedChinese => encode_hostid_message("多余的操作对象 "),
            HostidMessageLanguage::TraditionalChinese => encode_hostid_message("額外的運算元 "),
            HostidMessageLanguage::Other => b"extra operand ".to_vec(),
        };
        #[cfg(not(unix))]
        let mut message = b"extra operand ".to_vec();
        message.extend_from_slice(&quote_hostid_operand(operand));
        return Err(hostid_usage_error_bytes(args, &message));
    }
    Ok(())
}

fn quote_hostid_operand(operand: &OsStr) -> Vec<u8> {
    #[cfg(unix)]
    return locale_quote_hostid_bytes(operand);

    #[cfg(not(unix))]
    format!("'{operand:?}'").into_bytes()
}

#[cfg(unix)]
fn locale_quote_hostid_bytes(operand: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    let (left_quote, right_quote) = hostid_locale_quote_marks();
    let bytes = operand.as_bytes();
    let mut quoted = Vec::with_capacity(bytes.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(&left_quote);
    append_locale_quoted_bytes(&mut quoted, bytes, &right_quote);
    quoted.extend_from_slice(&right_quote);
    quoted
}

#[cfg(unix)]
fn append_locale_quoted_bytes(quoted: &mut Vec<u8>, bytes: &[u8], right_quote: &[u8]) {
    let mut index = 0;

    while index < bytes.len() {
        if !right_quote.is_empty() && bytes[index..].starts_with(right_quote) {
            quoted.push(b'\\');
            quoted.extend_from_slice(right_quote);
            index += right_quote.len();
            continue;
        }

        let byte = bytes[index];
        let escape = match byte {
            b'\x07' => Some(b'a'),
            b'\x08' => Some(b'b'),
            b'\t' => Some(b't'),
            b'\n' => Some(b'n'),
            b'\x0b' => Some(b'v'),
            b'\x0c' => Some(b'f'),
            b'\r' => Some(b'r'),
            b'\\' => Some(b'\\'),
            _ => None,
        };
        if let Some(escape) = escape {
            quoted.extend_from_slice(&[b'\\', escape]);
            index += 1;
            continue;
        }
        if byte.is_ascii() {
            if byte.is_ascii_control() {
                append_octal_byte(quoted, byte);
            } else {
                quoted.push(byte);
            }
            index += 1;
            continue;
        }

        let sequence_length = locale_character_length(&bytes[index..]);
        match sequence_length {
            Some((length, true)) => quoted.extend_from_slice(&bytes[index..index + length]),
            Some((length, false)) => {
                for &byte in &bytes[index..index + length] {
                    append_octal_byte(quoted, byte);
                }
            }
            None => append_octal_byte(quoted, byte),
        }
        index += sequence_length.map_or(1, |(length, _)| length);
    }
}

#[cfg(unix)]
fn locale_character_length(bytes: &[u8]) -> Option<(usize, bool)> {
    let mut wide_character: libc::wchar_t = 0;
    let mut state = unsafe { std::mem::zeroed::<libc::mbstate_t>() };
    let length = unsafe {
        mbrtowc(
            &mut wide_character,
            bytes.as_ptr().cast(),
            bytes.len(),
            &mut state,
        )
    };

    if length == usize::MAX || length == usize::MAX - 1 || length == 0 {
        None
    } else {
        Some((
            length,
            unsafe { iswprint(wide_character as libc::c_uint) } != 0,
        ))
    }
}

#[cfg(unix)]
fn append_octal_byte(quoted: &mut Vec<u8>, byte: u8) {
    quoted.extend_from_slice(&[
        b'\\',
        b'0' + (byte >> 6),
        b'0' + ((byte >> 3) & 7),
        b'0' + (byte & 7),
    ]);
}

#[cfg(unix)]
fn hostid_locale_quote_marks() -> (Vec<u8>, Vec<u8>) {
    match hostid_message_language() {
        HostidMessageLanguage::SimplifiedChinese => return (b"\"".to_vec(), b"\"".to_vec()),
        HostidMessageLanguage::TraditionalChinese => {
            return (encode_hostid_message("「"), encode_hostid_message("」"));
        }
        HostidMessageLanguage::Other => {}
    }

    let codeset = hostid_codeset();

    if codeset.eq_ignore_ascii_case(b"UTF-8") {
        ("‘".as_bytes().to_vec(), "’".as_bytes().to_vec())
    } else if codeset.eq_ignore_ascii_case(b"GB18030") {
        (b"\xa1\x07e".to_vec(), b"\xa1\xaf".to_vec())
    } else {
        (b"'".to_vec(), b"'".to_vec())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostidMessageLanguage {
    SimplifiedChinese,
    TraditionalChinese,
    Other,
}

#[cfg(unix)]
fn hostid_message_language() -> HostidMessageLanguage {
    let locale = unsafe {
        let locale = libc::setlocale(libc::LC_MESSAGES, std::ptr::null());
        if locale.is_null() {
            String::new()
        } else {
            CStr::from_ptr(locale).to_string_lossy().into_owned()
        }
    };

    if locale.eq_ignore_ascii_case("C") || locale.eq_ignore_ascii_case("POSIX") {
        return HostidMessageLanguage::Other;
    }

    if let Ok(language) = std::env::var("LANGUAGE")
        && !language.is_empty()
    {
        return language
            .split(':')
            .find_map(classify_hostid_message_language)
            .unwrap_or(HostidMessageLanguage::Other);
    }

    classify_hostid_message_language(&locale).unwrap_or(HostidMessageLanguage::Other)
}

fn classify_hostid_message_language(locale: &str) -> Option<HostidMessageLanguage> {
    let locale = locale.to_ascii_lowercase();

    if locale.starts_with("zh_cn") || locale.starts_with("zh_sg") {
        Some(HostidMessageLanguage::SimplifiedChinese)
    } else if locale.starts_with("zh_tw") || locale.starts_with("zh_hk") {
        Some(HostidMessageLanguage::TraditionalChinese)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn hostid_message_language() -> HostidMessageLanguage {
    HostidMessageLanguage::Other
}

#[cfg(unix)]
fn hostid_codeset() -> &'static [u8] {
    unsafe {
        let codeset = libc::nl_langinfo(libc::CODESET);
        if codeset.is_null() {
            &[]
        } else {
            CStr::from_ptr(codeset).to_bytes()
        }
    }
}

#[cfg(unix)]
fn encode_hostid_message(message: &str) -> Vec<u8> {
    let codeset = unsafe { libc::nl_langinfo(libc::CODESET) };
    if codeset.is_null() || hostid_codeset().eq_ignore_ascii_case(b"UTF-8") {
        return message.as_bytes().to_vec();
    }

    let descriptor = unsafe { iconv_open(codeset, c"UTF-8".as_ptr()) };
    if descriptor as isize == -1 {
        return message.as_bytes().to_vec();
    }

    let mut input_bytes_left = message.len();
    let mut input = message.as_ptr().cast_mut().cast::<libc::c_char>();
    let mut encoded = vec![0; message.len() * 4 + 16];
    let mut output_bytes_left = encoded.len();
    let mut output = encoded.as_mut_ptr().cast::<libc::c_char>();
    while input_bytes_left != 0 {
        let result = unsafe {
            iconv(
                descriptor,
                &mut input,
                &mut input_bytes_left,
                &mut output,
                &mut output_bytes_left,
            )
        };
        if result != usize::MAX {
            break;
        }
        if output_bytes_left == 0 {
            input_bytes_left = message.len();
            break;
        }

        let input_offset = message.len() - input_bytes_left;
        let invalid_character_length = message[input_offset..]
            .chars()
            .next()
            .map_or(1, char::len_utf8);
        unsafe {
            input = input.add(invalid_character_length);
            output.write(b'?' as libc::c_char);
            output = output.add(1);
        }
        input_bytes_left -= invalid_character_length;
        output_bytes_left -= 1;
    }
    unsafe {
        iconv_close(descriptor);
    }

    if input_bytes_left != 0 {
        message.as_bytes().to_vec()
    } else {
        encoded.truncate(encoded.len() - output_bytes_left);
        encoded
    }
}

fn initialize_hostid_locale() {
    #[cfg(unix)]
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }
}

fn hostid() -> String {
    /*
     * POSIX says gethostid returns a "32-bit identifier" but is silent
     * whether it's sign-extended.  Turn off any sign-extension.  This
     * is a no-op unless unsigned int is wider than 32 bits.
     */

    let mut result: c_long = unsafe { gethostid() };

    #[allow(overflowing_literals)]
    let mask = 0xffff_ffff;

    result &= mask;
    format!("{result:0>8x}")
}

fn write_hostid_output(output: &str) -> CTResult<()> {
    #[cfg(target_os = "linux")]
    if ctcore::ct_stdout_was_closed() {
        return Err(hostid_write_error(io::Error::from_raw_os_error(
            libc::EBADF,
        )));
    }

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout
        .write_all(output.as_bytes())
        .and_then(|_| stdout.flush())
        .map_err(hostid_write_error)
}

fn hostid_write_error(error: io::Error) -> Box<dyn ctcore::ct_error::CTError> {
    let detail = strip_errno(&error);

    #[cfg(unix)]
    let program_name = ctcore::ct_os_args()
        .next()
        .unwrap_or_else(|| OsString::from(ctcore::ct_util_name()));
    #[cfg(unix)]
    if hostid_message_language() != HostidMessageLanguage::Other || program_name.to_str().is_none()
    {
        let mut message = match hostid_message_language() {
            HostidMessageLanguage::SimplifiedChinese => encode_hostid_message("写入错误: "),
            HostidMessageLanguage::TraditionalChinese => encode_hostid_message("寫入時發生錯誤: "),
            HostidMessageLanguage::Other => b"write error: ".to_vec(),
        };
        message.extend_from_slice(detail.as_bytes());
        return raw_diagnostic_error(&program_name, &message, false);
    }

    CtSimpleError::new(1, format!("write error: {detail}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostidSemantic {
    pub hostid: String,
}

pub fn hostid_native_semantic(args: impl ctcore::Args) -> CTResult<HostidSemantic> {
    initialize_hostid_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect::<Vec<_>>();
    let _ = validate_standard_options(&args)?;
    let matches = ct_app().try_get_matches_from(hostid_clap_args(&args))?;
    reject_extra_operand(&matches, &args)?;

    Ok(HostidSemantic { hostid: hostid() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;
    #[test]
    fn test_tool_implementation() {
        let tool = Hostid;

        // 测试 name 方法
        assert_eq!(tool.name(), "hostid");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("hostid"));

        // 测试 execute 方法
        let args = vec![OsString::from("hostid")];
        assert!(tool.execute(&args).is_ok());
    }

    #[cfg(test)]
    mod tests_ct_main {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_expand_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_expand_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_expand_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_expand_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_expand_main() {
            let args = [ctcore::ct_util_name()];
            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    #[cfg(test)]
    mod tests_ct_app {
        use super::*;
        // hostid 接口: hostid [options]...
        // Print the numeric identifier (in hexadecimal) for the current host
        // Usage: target/debug/syskits hostid [options]
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
        fn test_ct_app_rejects_short_version() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-V"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
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
    }
}
