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

//! logname指令，它会显示目前用户的名称。

extern crate rust_i18n;
use clap::{Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, CTsageError, CtSimpleError};
use ctcore::ct_quoting_style::{CtQuotes, CtQuotingStyle, escape_name};
use std::ffi::{CStr, OsStr, OsString};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use sys_locale::get_locale;

unsafe extern "C" {
    // POSIX 要求使用 getlogin（或同等代码）
    pub fn getlogin() -> *const libc::c_char;
}

fn get_user_login_bytes() -> Option<Vec<u8>> {
    unsafe {
        let login_name: *const libc::c_char = getlogin();
        match login_name.is_null() {
            true => None,
            false => Some(CStr::from_ptr(login_name).to_bytes().to_vec()),
        }
    }
}

fn get_user_login() -> Option<String> {
    get_user_login_bytes().map(|name| String::from_utf8_lossy(&name).into_owned())
}

fn first_extra_operand(args: &[OsString]) -> Option<&OsStr> {
    let mut operands_only = false;
    for argument in args.iter().skip(1) {
        let bytes = argument.as_os_str().as_bytes();
        if operands_only {
            return Some(argument.as_os_str());
        }
        if bytes == b"--" {
            operands_only = true;
        } else if bytes == b"-" || !bytes.starts_with(b"-") {
            return Some(argument.as_os_str());
        } else {
            // An option-like argument before the operand has diagnostic
            // priority.  Leave it to clap for now.
            return None;
        }
    }
    None
}

fn quote_logname_operand(operand: &OsStr) -> String {
    escape_name(
        operand,
        &CtQuotingStyle::C {
            quotes: CtQuotes::Locale,
        },
    )
}

fn gnu_logname_option_error(args: &[OsString]) -> Option<Box<dyn CTError>> {
    const LONG_OPTIONS: [&[u8]; 2] = [b"help", b"version"];

    for argument in args.iter().skip(1) {
        let bytes = argument.as_os_str().as_bytes();
        if bytes == b"--" || bytes == b"-" || !bytes.starts_with(b"-") {
            break;
        }

        if let Some(long_argument) = bytes.strip_prefix(b"--") {
            let value_index = long_argument.iter().position(|byte| *byte == b'=');
            let name = &long_argument[..value_index.unwrap_or(long_argument.len())];
            let matches = LONG_OPTIONS
                .iter()
                .copied()
                .filter(|option| !name.is_empty() && option.starts_with(name))
                .collect::<Vec<_>>();
            let Some(option) = matches.first() else {
                let argument = std::str::from_utf8(bytes).ok()?;
                return Some(CTsageError::new(
                    1,
                    format!("unrecognized option '{argument}'"),
                ));
            };
            if value_index.is_some() {
                let option = std::str::from_utf8(option).expect("GNU option names are ASCII");
                return Some(CTsageError::new(
                    1,
                    format!("option '--{option}' doesn't allow an argument"),
                ));
            }

            // GNU exits as soon as --help or --version is recognized.
            return None;
        }

        let invalid = bytes[1];
        if invalid.is_ascii() && bytes != b"-h" && bytes != b"-V" {
            return Some(CTsageError::new(
                1,
                format!("invalid option -- '{}'", invalid as char),
            ));
        }
        return None;
    }
    None
}

fn normalize_logname_args(mut args: Vec<OsString>, posixly_correct: bool) -> Vec<OsString> {
    if posixly_correct || args.len() < 3 {
        return args;
    }

    let option_end = args
        .iter()
        .position(|argument| argument.as_os_str().as_bytes() == b"--")
        .unwrap_or(args.len());
    let mut reordered = Vec::with_capacity(args.len());
    reordered.push(args.remove(0));

    for argument in &args[..option_end - 1] {
        let bytes = argument.as_os_str().as_bytes();
        if bytes.len() > 1 && bytes.starts_with(b"-") {
            reordered.push(argument.clone());
        }
    }
    for argument in &args[..option_end - 1] {
        let bytes = argument.as_os_str().as_bytes();
        if bytes.len() <= 1 || !bytes.starts_with(b"-") {
            reordered.push(argument.clone());
        }
    }
    reordered.extend(args[option_end - 1..].iter().cloned());
    reordered
}

fn parse_logname_args(args: impl ctcore::Args) -> CTResult<()> {
    let args = normalize_logname_args(
        args.collect(),
        std::env::var_os("POSIXLY_CORRECT").is_some(),
    );
    if let Some(error) = gnu_logname_option_error(&args) {
        return Err(error);
    }
    match ct_app().try_get_matches_from(args.clone()) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == clap::error::ErrorKind::UnknownArgument => {
            if let Some(operand) = first_extra_operand(&args) {
                return Err(CTsageError::new(
                    1,
                    format!("extra operand {}", quote_logname_operand(operand)),
                ));
            }
            Err(error.into())
        }
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LognameSemanticRow {
    pub login_name: String,
    pub available: bool,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LognameSemantic {
    pub rows: Vec<LognameSemanticRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

fn logname_semantic_from_parse_error(err: clap::Error) -> LognameSemantic {
    let rendered = err.to_string();
    let exit_code = match err.kind() {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => 0,
        _ => 1,
    };

    if err.use_stderr() {
        LognameSemantic {
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text: rendered,
            exit_code,
        }
    } else {
        LognameSemantic {
            rows: Vec::new(),
            classic_text: rendered,
            stderr_text: String::new(),
            exit_code,
        }
    }
}

pub fn logname_native_semantic(args: impl ctcore::Args) -> CTResult<LognameSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    if let Err(err) = ct_app().try_get_matches_from(args) {
        return Ok(logname_semantic_from_parse_error(err));
    }

    let source = "posix:getlogin".to_string();
    let semantic = match get_user_login() {
        Some(login_name) => LognameSemantic {
            rows: vec![LognameSemanticRow {
                login_name: login_name.clone(),
                available: true,
                source,
            }],
            classic_text: format!("{login_name}\n"),
            stderr_text: String::new(),
            exit_code: 0,
        },
        None => LognameSemantic {
            rows: vec![LognameSemanticRow {
                login_name: String::new(),
                available: false,
                source,
            }],
            classic_text: String::new(),
            stderr_text: "logname: no login name\n".to_string(),
            exit_code: 1,
        },
    };

    Ok(semantic)
}

#[derive(Default)]
pub struct Logname;
impl Tool for Logname {
    fn name(&self) -> &'static str {
        "logname"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        logname_main(args.iter().cloned())
    }
}

pub fn logname_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    parse_logname_args(args)?;

    match get_user_login_bytes() {
        Some(userlogin) => {
            let stdout = std::io::stdout();
            let mut output = stdout.lock();
            output.write_all(&userlogin).unwrap();
            output.write_all(b"\n").unwrap();
        }
        None => return Err(CtSimpleError::new(1, "no login name".to_string())),
    }

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("logname.about");
    let usage_description = t!("logname.usage");
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
}

#[cfg(test)]
mod tests_tool_implementation {
    use crate::Logname;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Logname;

        // 测试 name 方法
        assert_eq!(tool.name(), "logname");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("logname"));

        // 测试 execute 方法
        let args = vec![OsString::from("logname"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err()); // --help参数通常会返回错误
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;

        /// 检测是否在容器环境中运行
        fn is_container() -> bool {
            // 检查常见的容器环境标识
            if std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
                || std::env::var("DOCKER_CONTAINER").is_ok()
                || std::path::Path::new("/.dockerenv").exists()
                || std::path::Path::new("/run/.containerenv").exists()
            {
                return true;
            }

            // 检查 cgroup
            if let Ok(contents) = std::fs::read_to_string("/proc/1/cgroup") {
                if contents.contains("/docker/") || contents.contains("/kubepods/") {
                    return true;
                }
            }

            false
        }

        #[test]
        fn test_logname_main_execution_default() {
            let args = [ctcore::ct_util_name()];
            let result = logname_main(args.iter().map(OsString::from));

            if !is_container() && super::get_user_login().is_some() {
                assert!(result.is_ok());
            }
        }
        #[test]
        fn test_logname_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = logname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_logname_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = logname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_logname_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = logname_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_logname_main_execution_help_short() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = logname_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_logname_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = logname_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_logname_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = logname_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_logname_native_semantic_success_or_unavailable() {
            let args = [ctcore::ct_util_name()];
            let semantic =
                logname_native_semantic(args.iter().map(OsString::from)).expect("semantic");

            assert_eq!(semantic.rows.len(), 1);
            assert_eq!(semantic.rows[0].source, "posix:getlogin");

            if let Some(login_name) = super::get_user_login() {
                assert_eq!(semantic.exit_code, 0);
                assert_eq!(semantic.rows[0].login_name, login_name);
                assert!(semantic.rows[0].available);
                assert_eq!(semantic.classic_text, format!("{login_name}\n"));
                assert!(semantic.stderr_text.is_empty());
            } else {
                assert_eq!(semantic.exit_code, 1);
                assert!(semantic.rows[0].login_name.is_empty());
                assert!(!semantic.rows[0].available);
                assert!(semantic.classic_text.is_empty());
                assert_eq!(semantic.stderr_text, "logname: no login name\n");
            }
        }

        #[test]
        fn test_logname_native_semantic_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let semantic =
                logname_native_semantic(args.iter().map(OsString::from)).expect("semantic");

            assert!(semantic.rows.is_empty());
            assert!(semantic.classic_text.is_empty());
            assert_eq!(semantic.exit_code, 1);
            assert!(
                semantic.stderr_text.contains("unexpected argument"),
                "stderr: {:?}",
                semantic.stderr_text
            );
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // logname 接口: logname [OPTION]...
        //
        // Options:
        //   -h, --help     Print help
        //   -V, --version  Print version

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
    }
}
