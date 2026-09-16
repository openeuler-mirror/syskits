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

//! tty 命令行工具，用于打印当前终端设备的文件名

extern crate rust_i18n;
use clap::error::ErrorKind;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, set_ct_exit_code, strip_errno};
use std::borrow::Cow;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::io::{self, IsTerminal, Write};
use std::os::unix::ffi::OsStrExt;
use sys_locale::get_locale;

mod tty_flags {
    pub const TTY_SILENT: &str = "silent";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtySemantic {
    pub is_tty: bool,
    pub tty_name: Option<String>,
    pub silent: bool,
    pub classic_text: String,
    pub exit_code: i32,
}

pub fn tty_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = match ct_app().try_get_matches_from(prepare_tty_args(args)?) {
        Ok(m) => m,
        Err(e) => {
            e.print().ok();
            match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => return Ok(()),
                _ => return Err(2.into()),
            }
        }
    };

    if let Some(value) = tty_handle_silent(matches) {
        return value;
    }

    let mut stdout = std::io::stdout();

    let tty_name = nix::unistd::ttyname(std::io::stdin());

    let tty_write_result = match tty_name {
        Ok(name) => writeln!(stdout, "{}", name.display()),
        Err(_) => {
            set_ct_exit_code(1);
            writeln!(stdout, "{}", t!("tty.not_a_tty"))
        }
    };

    if let Err(error) = tty_write_result.and_then(|()| stdout.flush()) {
        exit_tty_write_error(&error);
    }

    Ok(())
}

const TTY_LONG_OPTIONS: &[(&str, u8)] = &[
    ("silent", b's'),
    ("quiet", b's'),
    ("help", b'h'),
    ("version", b'v'),
];
// Keep the syskits -h/-V extensions outside GNU tty compatibility handling.
const TTY_SHORT_OPTIONS: &[u8] = b"shV";

enum TtyLongOptionMatch {
    None,
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
}

fn prepare_tty_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    prepare_tty_args_with_mode(args, ctcore::ct_posix::posixly_correct())
}

fn prepare_tty_args_with_mode(
    args: impl ctcore::Args,
    posixly_correct: bool,
) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut parse_options = true;
    let mut first_operand = None;

    for argument in args.iter().skip(1) {
        let bytes = argument.as_bytes();
        if parse_options && bytes == b"--" {
            parse_options = false;
            continue;
        }

        if parse_options && bytes.len() > 1 && bytes[0] == b'-' {
            let terminal = if bytes.starts_with(b"--") {
                validate_tty_long_option(bytes)?
            } else if bytes == b"-h" || bytes == b"-V" {
                true
            } else {
                validate_tty_short_options(bytes)?;
                false
            };

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

        first_operand.get_or_insert(argument);
        if posixly_correct {
            parse_options = false;
        }
    }

    if let Some(operand) = first_operand {
        let mut message = b"extra operand ".to_vec();
        message.extend_from_slice(&tty_quote_operand(operand));
        return Err(TtyUsageError::boxed(message));
    }

    Ok(args)
}

fn match_tty_long_option(name: &[u8]) -> TtyLongOptionMatch {
    if let Some((option, _)) = TTY_LONG_OPTIONS
        .iter()
        .find(|(option, _)| option.as_bytes() == name)
    {
        return TtyLongOptionMatch::Recognized(option);
    }

    let mut option_values = Vec::new();
    let mut matches = Vec::new();
    for (option, value) in TTY_LONG_OPTIONS
        .iter()
        .filter(|(option, _)| option.as_bytes().starts_with(name))
    {
        if !option_values.contains(value) {
            option_values.push(*value);
            matches.push(*option);
        }
    }

    match matches.as_slice() {
        [] => TtyLongOptionMatch::None,
        [option] => TtyLongOptionMatch::Recognized(option),
        _ => TtyLongOptionMatch::Ambiguous(matches),
    }
}

fn validate_tty_long_option(argument: &[u8]) -> CTResult<bool> {
    let long = &argument[2..];
    let separator = long.iter().position(|byte| *byte == b'=');
    let name = &long[..separator.unwrap_or(long.len())];

    match match_tty_long_option(name) {
        TtyLongOptionMatch::None => {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(argument);
            message.push(b'\'');
            Err(TtyUsageError::boxed(message))
        }
        TtyLongOptionMatch::Ambiguous(matches) => {
            let possibilities = matches
                .into_iter()
                .map(|option| format!("'--{option}'"))
                .collect::<Vec<_>>()
                .join(" ");
            let mut message = b"option '".to_vec();
            message.extend_from_slice(argument);
            message.extend_from_slice(b"' is ambiguous; possibilities: ");
            message.extend_from_slice(possibilities.as_bytes());
            Err(TtyUsageError::boxed(message))
        }
        TtyLongOptionMatch::Recognized(canonical) if separator.is_some() => {
            Err(TtyUsageError::boxed(
                format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
            ))
        }
        TtyLongOptionMatch::Recognized("help" | "version") => Ok(true),
        TtyLongOptionMatch::Recognized(_) => Ok(false),
    }
}

fn validate_tty_short_options(argument: &[u8]) -> CTResult<()> {
    if let Some(unknown) = argument[1..]
        .iter()
        .find(|option| !TTY_SHORT_OPTIONS.contains(option))
    {
        let mut message = b"invalid option -- '".to_vec();
        message.push(*unknown);
        message.push(b'\'');
        return Err(TtyUsageError::boxed(message));
    }
    Ok(())
}

fn tty_quote_operand(operand: &OsStr) -> Vec<u8> {
    if tty_locale_is_utf8() {
        tty_quote_utf8_operand(operand)
    } else {
        tty_quote_c_operand(operand)
    }
}

fn tty_locale_is_utf8() -> bool {
    for name in ["LC_ALL", "LC_CTYPE", "LANG"] {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let value = value.to_string_lossy().to_ascii_uppercase();
        return value.contains("UTF-8") || value.contains("UTF8");
    }
    false
}

fn tty_quote_c_operand(operand: &OsStr) -> Vec<u8> {
    let mut quoted = Vec::with_capacity(operand.as_bytes().len() + 2);
    quoted.push(b'\'');
    for byte in operand.as_bytes() {
        tty_push_quoted_ascii(&mut quoted, *byte, Some(b'\''));
    }
    quoted.push(b'\'');
    quoted
}

fn tty_quote_utf8_operand(operand: &OsStr) -> Vec<u8> {
    let input = operand.as_bytes();
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
            tty_push_quoted_ascii(&mut quoted, input[index], None);
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
                    tty_push_octal_escape(&mut quoted, *byte);
                }
                index += invalid_length;
            }
        }
    }

    quoted.extend_from_slice(right_quote);
    quoted
}

fn tty_push_quoted_ascii(output: &mut Vec<u8>, byte: u8, quote_to_escape: Option<u8>) {
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
        _ => tty_push_octal_escape(output, byte),
    }
}

fn tty_push_octal_escape(output: &mut Vec<u8>, byte: u8) {
    output.push(b'\\');
    output.push(b'0' + (byte >> 6));
    output.push(b'0' + ((byte >> 3) & 7));
    output.push(b'0' + (byte & 7));
}

#[derive(Debug)]
struct TtyUsageError {
    message: Vec<u8>,
}

impl TtyUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

impl Display for TtyUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for TtyUsageError {}

impl CTError for TtyUsageError {
    fn code(&self) -> i32 {
        2
    }

    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage(&self) -> bool {
        true
    }
}

fn tty_write_error_message(error: &io::Error) -> String {
    format!("write error: {}", strip_errno(error))
}

fn exit_tty_write_error(error: &io::Error) -> ! {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(
        stderr,
        "{}: {}",
        ctcore::ct_util_name(),
        tty_write_error_message(error)
    );
    std::process::exit(3);
}

fn tty_handle_silent(matches: ArgMatches) -> Option<CTResult<()>> {
    let is_silent = matches.get_flag(tty_flags::TTY_SILENT);

    // 如果处于静默模式，我们不需要名称，只需要判断标准输入是否是TTY
    if is_silent {
        return Some(match std::io::stdin().is_terminal() {
            true => Ok(()),
            false => Err(1.into()),
        });
    };
    None
}

fn tty_semantic_from_matches(matches: &ArgMatches) -> TtySemantic {
    let silent = matches.get_flag(tty_flags::TTY_SILENT);
    let tty_name = nix::unistd::ttyname(std::io::stdin())
        .ok()
        .map(|name| name.display().to_string());
    let is_tty = tty_name.is_some();
    let exit_code = if is_tty { 0 } else { 1 };
    let classic_text = if silent {
        String::new()
    } else {
        tty_name
            .clone()
            .unwrap_or_else(|| t!("tty.not_a_tty").to_string())
    };

    TtySemantic {
        is_tty,
        tty_name,
        silent,
        classic_text,
        exit_code,
    }
}

pub fn tty_native_semantic(args: impl ctcore::Args) -> CTResult<TtySemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;
    Ok(tty_semantic_from_matches(&matches))
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("tty.about");
    let usage_description = t!("tty.usage");

    let arg = Arg::new(tty_flags::TTY_SILENT)
        .long(tty_flags::TTY_SILENT)
        .visible_alias("quiet")
        .short('s')
        .help(t!("tty.clap.tty_silent"))
        .action(ArgAction::SetTrue);
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args_override_self(true)
        .arg(arg)
}

#[derive(Default)]
pub struct Tty;
impl Tool for Tty {
    fn name(&self) -> &'static str {
        "tty"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 直接调用原有的 tty_main 函数
        tty_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::io;
    use std::sync::Mutex;

    static LOCALE_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_tool_implementation() {
        let tool = Tty;

        // 测试 name 方法
        assert_eq!(tool.name(), "tty");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("tty"));

        // 测试 execute 方法
        let args = vec![OsString::from("tty"), OsString::from("--version")];
        assert!(tool.execute(&args).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_tty_write_error_message_preserves_linux_errno() {
        let error = io::Error::from_raw_os_error(nix::libc::ENOSPC);

        assert_eq!(
            tty_write_error_message(&error),
            "write error: No space left on device"
        );
    }

    #[test]
    fn test_tty_main_reports_extra_operand_like_gnu() {
        let _guard = LOCALE_LOCK.lock().unwrap();
        let previous = std::env::var_os("LC_ALL");
        unsafe { std::env::set_var("LC_ALL", "C") };
        let error =
            tty_main([OsString::from("tty"), OsString::from("extra")].into_iter()).unwrap_err();

        match previous {
            Some(value) => unsafe { std::env::set_var("LC_ALL", value) },
            None => unsafe { std::env::remove_var("LC_ALL") },
        }

        assert_eq!(error.to_string(), "extra operand 'extra'");
    }

    #[test]
    fn test_prepare_tty_args_reports_gnu_option_errors() {
        let cases = [
            (
                "--quiet=value",
                b"option '--quiet' doesn't allow an argument".as_slice(),
            ),
            ("--unknown", b"unrecognized option '--unknown'".as_slice()),
            ("-sfoo", b"invalid option -- 'f'".as_slice()),
        ];

        for (argument, expected) in cases {
            let error = prepare_tty_args_with_mode(
                [OsString::from("tty"), OsString::from(argument)].into_iter(),
                false,
            )
            .unwrap_err();

            assert_eq!(error.diagnostic_bytes().as_ref(), expected);
            assert_eq!(error.code(), 2);
            assert!(error.usage());
        }
    }

    #[test]
    fn test_prepare_tty_args_deduplicates_long_option_aliases_in_ambiguity() {
        let error = prepare_tty_args_with_mode(
            [OsString::from("tty"), OsString::from("--=")].into_iter(),
            false,
        )
        .unwrap_err();

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"option '--=' is ambiguous; possibilities: '--silent' '--help' '--version'"
        );
    }

    #[test]
    fn test_prepare_tty_args_recognizes_late_help_like_gnu_getopt() {
        let _guard = LOCALE_LOCK.lock().unwrap();
        let previous = std::env::var_os("LC_ALL");
        unsafe { std::env::set_var("LC_ALL", "C") };
        let args = [
            OsString::from("tty"),
            OsString::from("extra"),
            OsString::from("--help"),
        ];

        let prepared = prepare_tty_args_with_mode(args.clone().into_iter(), false).unwrap();
        assert_eq!(prepared, [OsString::from("tty"), OsString::from("--help")]);

        let error = prepare_tty_args_with_mode(args.into_iter(), true).unwrap_err();
        match previous {
            Some(value) => unsafe { std::env::set_var("LC_ALL", value) },
            None => unsafe { std::env::remove_var("LC_ALL") },
        }

        assert_eq!(error.to_string(), "extra operand 'extra'");
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::ffi::OsString;

        use super::*;

        #[test]
        fn test_tty_main_execution_default() {
            let args = [ctcore::ct_util_name()];
            let result = tty_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_tty_main_execution_version() {
            let args_vec = [ctcore::ct_util_name(), "--version"];
            let args = args_vec.iter().map(OsString::from);
            let result = tty_main(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_tty_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = tty_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_tty_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = tty_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_tty_main_execution_help_short() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = tty_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_tty_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = tty_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_tty_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = tty_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_tty_main_silent_long() {
            let args = [ctcore::ct_util_name(), "--silent"];
            let result = tty_main(args.iter().map(OsString::from));
            if std::io::stdin().is_terminal() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }
        #[test]
        fn test_tty_main_silent_short() {
            let args = [ctcore::ct_util_name(), "-s"];
            let result = tty_main(args.iter().map(OsString::from));
            if std::io::stdin().is_terminal() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }

        #[test]
        fn test_tty_main_quiet_long() {
            let args = [ctcore::ct_util_name(), "--quiet"];
            let result = tty_main(args.iter().map(OsString::from));
            if std::io::stdin().is_terminal() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }

        #[test]
        fn test_tty_native_semantic_silent_reflects_terminal_state() {
            let args = [ctcore::ct_util_name(), "-s"];
            let result = tty_native_semantic(args.iter().map(OsString::from)).expect("semantic");

            assert!(result.silent);
            assert_eq!(result.is_tty, std::io::stdin().is_terminal());
            assert_eq!(result.exit_code, if result.is_tty { 0 } else { 1 });
            assert_eq!(result.classic_text, "");
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // tty 接口: tty [OPTION]...
        //
        // Options:
        //   -s, --silent   print nothing, only return an exit status [aliases: quiet]
        //   -h, --help     Print help
        //   -V, --version  Print version

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
        fn test_ct_app_execution_help_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
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

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_silent_long() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--silent"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_silent_short() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-s"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_quiet_long() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--quiet"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_repeated_silent_aliases() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-s", "--quiet", "--silent", "-s"];

            let matches = command.try_get_matches_from(args).unwrap();

            assert!(matches.get_flag(tty_flags::TTY_SILENT));
        }
    }
}
