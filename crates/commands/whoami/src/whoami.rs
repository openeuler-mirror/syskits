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
use ctcore::ct_error::FromIo;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, strip_errno};
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
    write_whoami_username(&username)
        .map_err(|error| CtSimpleError::new(1, whoami_write_error_message(&error)))?;

    let result = username.into_string().unwrap();
    Ok(result)
}

fn write_whoami_username(username: &OsStr) -> io::Result<()> {
    if let Some(error) = whoami_closed_stdout_error(ctcore::ct_stdout_was_closed()) {
        return Err(error);
    }
    ct_println_verbatim(username)
}

fn whoami_write_error_message(error: &io::Error) -> String {
    format!("write error: {}", strip_errno(error))
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
}

impl WhoamiUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
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

    fn usage(&self) -> bool {
        true
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
        let mut message = b"extra operand ".to_vec();
        message.extend_from_slice(&quote_whoami_operand(operand));
        return Err(WhoamiUsageError::boxed(message));
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

fn quote_whoami_operand(operand: &OsStr) -> Vec<u8> {
    if whoami_locale_is_utf8() {
        quote_whoami_utf8_operand(operand)
    } else {
        quote_whoami_c_operand(operand)
    }
}

fn whoami_locale_is_utf8() -> bool {
    for name in ["LC_ALL", "LC_CTYPE", "LANG"] {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        return value
            .to_string_lossy()
            .to_ascii_uppercase()
            .contains("UTF-8")
            || value
                .to_string_lossy()
                .to_ascii_uppercase()
                .contains("UTF8");
    }
    false
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
    let input = operand.as_encoded_bytes();
    let left_quote = "‘".as_bytes();
    let right_quote = "’".as_bytes();
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
            push_whoami_quoted_ascii(&mut quoted, input[index], None);
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
    platform::get_username().map_err(|_| whoami_unknown_uid_error(uid))
}

#[cfg(unix)]
fn whoami_unknown_uid_error(uid: libc::uid_t) -> Box<dyn CTError> {
    CtSimpleError::new(1, format!("cannot find name for user ID {uid}"))
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
            "write error: No space left on device"
        );
        assert_eq!(
            whoami_write_error_message(&io::Error::from_raw_os_error(libc::EPIPE)),
            "write error: Broken pipe"
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
        let error = whoami_unknown_uid_error(60_000);

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"cannot find name for user ID 60000"
        );
    }
}
