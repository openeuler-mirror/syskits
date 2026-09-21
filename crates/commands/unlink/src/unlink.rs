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
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, FromIo};

use std::borrow::Cow;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::fs::remove_file;
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
        Box::new(Self {
            message,
            usage_hint: format!(
                "Try '{} --help' for more information.",
                ctcore::ct_help_utility_name()
            )
            .into_bytes(),
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

pub fn unlink_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(prepare_unlink_args(args)?)?;

    let path: &Path = matches.get_one::<OsString>(OPT_PATH).unwrap().as_ref();

    remove_file(path).map_err_context(|| format!("cannot unlink {}", path.quote()))
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
        [] => Err(UnlinkUsageError::boxed(b"missing operand".to_vec())),
        [_] => Ok(args),
        [_, extra, ..] => {
            let mut message = b"extra operand ".to_vec();
            message.extend_from_slice(&unlink_quote_c_operand(extra));
            Err(UnlinkUsageError::boxed(message))
        }
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

            assert_eq!(error.to_string(), "extra operand 'second'");
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

            assert_eq!(error.to_string(), "extra operand '--version'");
            assert!(error.usage());
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
