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
#[cfg(not(unix))]
use ctcore::ct_display::Quotable;
use ctcore::ct_display::locale_quote;
use ctcore::ct_error::{CTResult, CtSimpleError, strip_errno};
use libc::c_long;
#[cfg(unix)]
use std::error::Error;
use std::ffi::OsString;
#[cfg(unix)]
use std::fmt::{self, Display, Formatter};
use std::io::{self, Write};
use std::path::Path;
use sys_locale::get_locale;

// currently rust libc interface doesn't include gethostid
unsafe extern "C" {
    pub unsafe fn gethostid() -> c_long;
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
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
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
    let matches = ct_app().try_get_matches_from(args.iter().cloned())?;
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
    use std::os::unix::ffi::OsStrExt;

    let program_name = args
        .first()
        .map(|arg| arg.as_os_str())
        .unwrap_or_else(|| std::ffi::OsStr::new(ctcore::ct_util_name()));
    let mut diagnostic = Vec::with_capacity(program_name.as_bytes().len() + message.len() + 64);
    diagnostic.extend_from_slice(program_name.as_bytes());
    diagnostic.extend_from_slice(b": ");
    diagnostic.extend_from_slice(message);
    diagnostic.extend_from_slice(b"\nTry '");
    diagnostic.extend_from_slice(program_name.as_bytes());
    diagnostic.extend_from_slice(b" --help' for more information.\n");

    Box::new(HostidRawOptionError { diagnostic })
}

fn hostid_usage_error(args: &[OsString], message: String) -> Box<dyn ctcore::ct_error::CTError> {
    #[cfg(unix)]
    if args.first().is_some_and(|arg| arg.to_str().is_none()) {
        return raw_usage_error(args, message.as_bytes());
    }

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
        return Err(hostid_usage_error(
            args,
            format!("extra operand {}", quote_hostid_operand(operand)),
        ));
    }
    Ok(())
}

fn quote_hostid_operand(operand: &OsString) -> String {
    if let Some(operand) = operand.to_str() {
        return locale_quote(operand);
    }

    #[cfg(unix)]
    return locale_quote_non_utf8_operand(operand);

    #[cfg(not(unix))]
    operand.quote().to_string()
}

#[cfg(unix)]
fn locale_quote_non_utf8_operand(operand: &OsString) -> String {
    use std::os::unix::ffi::OsStrExt;

    let marker_quote = locale_quote("x");
    let (left_quote, right_quote) = marker_quote
        .split_once('x')
        .expect("locale quote must surround its content");
    let utf8_locale = left_quote == "‘";
    let mut escaped = String::new();
    let mut remaining = operand.as_bytes();

    while !remaining.is_empty() {
        match std::str::from_utf8(remaining) {
            Ok(text) => {
                append_locale_quoted_text(&mut escaped, text, right_quote, utf8_locale);
                break;
            }
            Err(error) => {
                let valid_end = error.valid_up_to();
                if valid_end != 0 {
                    let (valid, rest) = remaining.split_at(valid_end);
                    append_locale_quoted_text(
                        &mut escaped,
                        std::str::from_utf8(valid).expect("valid UTF-8 prefix"),
                        right_quote,
                        utf8_locale,
                    );
                    remaining = rest;
                }
                let byte = remaining[0];
                use std::fmt::Write;
                write!(escaped, "\\{byte:03o}").expect("write to string");
                remaining = &remaining[1..];
            }
        }
    }

    format!("{left_quote}{escaped}{right_quote}")
}

#[cfg(unix)]
fn append_locale_quoted_text(
    escaped: &mut String,
    text: &str,
    right_quote: &str,
    utf8_locale: bool,
) {
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            character if is_hostid_right_quote(character, right_quote) => {
                escaped.push('\\');
                escaped.push(character);
            }
            '\u{0007}' => escaped.push_str("\\a"),
            '\u{0008}' => escaped.push_str("\\b"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\u{000B}' => escaped.push_str("\\v"),
            '\u{000C}' => escaped.push_str("\\f"),
            '\r' => escaped.push_str("\\r"),
            character if character.is_control() || (!utf8_locale && !character.is_ascii()) => {
                use std::fmt::Write;
                for byte in character.to_string().as_bytes() {
                    write!(escaped, "\\{byte:03o}").expect("write to string");
                }
            }
            character => escaped.push(character),
        }
    }
}

#[cfg(unix)]
fn is_hostid_right_quote(character: char, right_quote: &str) -> bool {
    let mut quote_characters = right_quote.chars();
    quote_characters.next() == Some(character) && quote_characters.next().is_none()
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
    CtSimpleError::new(1, format!("write error: {}", strip_errno(&error)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostidSemantic {
    pub hostid: String,
}

pub fn hostid_native_semantic(args: impl ctcore::Args) -> CTResult<HostidSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect::<Vec<_>>();
    let _ = validate_standard_options(&args)?;
    let matches = ct_app().try_get_matches_from(args.iter().cloned())?;
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
