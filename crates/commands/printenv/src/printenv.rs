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
use clap::{Arg, ArgAction, ArgMatches, Command, builder::OsStringValueParser, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::{
    Tool,
    ct_error::{CTResult, CtSimpleError, strip_errno},
};
#[cfg(not(target_os = "linux"))]
use std::env;
use std::ffi::{CStr, OsString};
use std::io::{self, Write};
use sys_locale::get_locale;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    static mut environ: *mut *mut std::os::raw::c_char;
}

static PRINTENV_OPT_NULL: &str = "null";

static PRINTENV_ARG_VARIABLES: &str = "variables";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintenvRow {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintenvSemantic {
    pub rows: Vec<PrintenvRow>,
    pub classic_text: String,
    pub exit_code: i32,
}

struct PrintenvOptions {
    separator: u8,
    variables: Vec<OsString>,
}

impl PrintenvOptions {
    fn from_matches(args_match: &ArgMatches) -> Self {
        let variables = args_match
            .get_many::<OsString>(PRINTENV_ARG_VARIABLES)
            .map(|v| v.cloned().collect())
            .unwrap_or_default();
        let null = args_match.get_count(PRINTENV_OPT_NULL) > 0;
        Self {
            separator: if null { b'\0' } else { b'\n' },
            variables,
        }
    }
}

/// 主函数用于打印环境变量。
///
/// # 参数
/// `args`: 实现了 `ctcore::Args` 的参数对象，用于解析命令行参数。
///
/// # 返回值
/// 返回一个 `CTResult<()>`，成功则为 `Ok(())`，失败则为 `Err(1.into())`。
pub fn printenv_main(args: impl ctcore::Args) -> CTResult<()> {
    let _sigpipe_guard = SigpipeGuard::for_cli();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 从命令行参数中获取匹配项
    let args_match = ct_app().get_matches_from(args);
    let options = PrintenvOptions::from_matches(&args_match);
    let exit_code = printenv_classic_from_options(&options)?;
    if exit_code == 0 {
        Ok(())
    } else {
        Err(exit_code.into())
    }
}

#[cfg(target_os = "linux")]
struct SigpipeGuard {
    previous: ctcore::libc::sighandler_t,
}

#[cfg(target_os = "linux")]
impl SigpipeGuard {
    fn for_cli() -> Option<Self> {
        if !ctcore::ct_sigpipe_was_default() {
            return None;
        }

        let previous =
            unsafe { ctcore::libc::signal(ctcore::libc::SIGPIPE, ctcore::libc::SIG_DFL) };
        (previous != ctcore::libc::SIG_ERR).then_some(Self { previous })
    }
}

#[cfg(target_os = "linux")]
impl Drop for SigpipeGuard {
    fn drop(&mut self) {
        unsafe {
            ctcore::libc::signal(ctcore::libc::SIGPIPE, self.previous);
        }
    }
}

#[cfg(not(target_os = "linux"))]
struct SigpipeGuard;

#[cfg(not(target_os = "linux"))]
impl SigpipeGuard {
    fn for_cli() -> Option<Self> {
        None
    }
}

pub fn printenv_native_semantic(args: impl ctcore::Args) -> CTResult<PrintenvSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app().get_matches_from(args);
    let options = PrintenvOptions::from_matches(&args_match);
    Ok(printenv_semantic_from_options(&options))
}

fn printenv_semantic_from_options(options: &PrintenvOptions) -> PrintenvSemantic {
    let mut rows = Vec::new();
    let mut classic_text = String::new();
    let mut error_found = false;
    let environment = environment_entries();

    if options.variables.is_empty() {
        for entry in environment {
            let (name, value) = split_environment_entry(&entry);
            classic_text.push_str(&String::from_utf8_lossy(&entry));
            classic_text.push(options.separator.into());
            rows.push(PrintenvRow { name, value });
        }
        return PrintenvSemantic {
            rows,
            classic_text,
            exit_code: 0,
        };
    }

    for variable in &options.variables {
        let variable_bytes = variable.as_encoded_bytes();
        if variable_bytes.contains(&b'=') {
            error_found = true;
            continue;
        }

        let mut found = false;
        for entry in &environment {
            if let Some(value) = environment_entry_value(entry, variable_bytes) {
                let value = String::from_utf8_lossy(value).into_owned();
                classic_text.push_str(&value);
                classic_text.push(options.separator.into());
                rows.push(PrintenvRow {
                    name: variable.to_string_lossy().into_owned(),
                    value,
                });
                found = true;
            }
        }
        if !found {
            error_found = true;
        }
    }

    PrintenvSemantic {
        rows,
        classic_text,
        exit_code: if error_found { 1 } else { 0 },
    }
}

fn printenv_classic_from_options(options: &PrintenvOptions) -> CTResult<i32> {
    let environment = environment_entries();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    write_environment_entries(&mut stdout, options, &environment).map_err(printenv_write_error)
}

fn printenv_write_error(error: io::Error) -> Box<dyn ctcore::ct_error::CTError> {
    printenv_redirect_stdout_to_dev_null();
    let error = printenv_normalize_stdout_write_error(error, ctcore::ct_stdout_was_closed());
    CtSimpleError::new(2, format!("write error: {}", strip_errno(&error)))
}

fn printenv_normalize_stdout_write_error(error: io::Error, stdout_was_closed: bool) -> io::Error {
    #[cfg(unix)]
    if stdout_was_closed {
        return io::Error::from_raw_os_error(ctcore::libc::EBADF);
    }

    error
}

#[cfg(unix)]
fn printenv_redirect_stdout_to_dev_null() {
    const DEV_NULL: &[u8] = b"/dev/null\0";

    unsafe {
        let fd = ctcore::libc::open(DEV_NULL.as_ptr().cast(), ctcore::libc::O_WRONLY);
        if fd >= 0 {
            ctcore::libc::dup2(fd, ctcore::libc::STDOUT_FILENO);
            ctcore::libc::close(fd);
        }
    }
}

#[cfg(not(unix))]
fn printenv_redirect_stdout_to_dev_null() {}

fn write_environment_entries(
    stdout: &mut impl Write,
    options: &PrintenvOptions,
    environment: &[Vec<u8>],
) -> io::Result<i32> {
    if options.variables.is_empty() {
        for entry in environment {
            stdout.write_all(entry)?;
            stdout.write_all(&[options.separator])?;
        }
        return Ok(0);
    }

    let mut all_found = true;
    for variable in &options.variables {
        let variable = variable.as_encoded_bytes();
        if variable.contains(&b'=') {
            all_found = false;
            continue;
        }

        let mut found = false;
        for entry in environment {
            if let Some(value) = environment_entry_value(entry, variable) {
                stdout.write_all(value)?;
                stdout.write_all(&[options.separator])?;
                found = true;
            }
        }
        all_found &= found;
    }

    Ok(i32::from(!all_found))
}

fn environment_entry_value<'a>(entry: &'a [u8], variable: &[u8]) -> Option<&'a [u8]> {
    if variable.is_empty()
        || !entry.starts_with(variable)
        || entry.get(variable.len()) != Some(&b'=')
    {
        return None;
    }
    Some(&entry[variable.len() + 1..])
}

fn split_environment_entry(entry: &[u8]) -> (String, String) {
    let Some(separator) = entry.iter().position(|byte| *byte == b'=') else {
        return (String::from_utf8_lossy(entry).into_owned(), String::new());
    };

    (
        String::from_utf8_lossy(&entry[..separator]).into_owned(),
        String::from_utf8_lossy(&entry[separator + 1..]).into_owned(),
    )
}

#[cfg(target_os = "linux")]
fn environment_entries() -> Vec<Vec<u8>> {
    let mut entries = Vec::new();
    let mut cursor = unsafe { environ };

    while !cursor.is_null() {
        let entry = unsafe { *cursor };
        if entry.is_null() {
            break;
        }
        entries.push(unsafe { CStr::from_ptr(entry) }.to_bytes().to_vec());
        cursor = unsafe { cursor.add(1) };
    }

    entries
}

#[cfg(not(target_os = "linux"))]
fn environment_entries() -> Vec<Vec<u8>> {
    env::vars_os()
        .map(|(name, value)| {
            let mut entry = name.as_encoded_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_encoded_bytes());
            entry
        })
        .collect()
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("printenv.about");
    let usage_description = t!("printenv.usage");

    let args = vec![
        Arg::new(PRINTENV_OPT_NULL)
            .short('0')
            .long(PRINTENV_OPT_NULL)
            .help(t!("printenv.clap.printenv_opt_null"))
            .action(ArgAction::Count),
        Arg::new(PRINTENV_ARG_VARIABLES)
            .action(ArgAction::Append)
            .num_args(1..)
            .trailing_var_arg(true)
            .value_parser(OsStringValueParser::new()),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

#[derive(Default)]
pub struct Printenv;
impl Tool for Printenv {
    fn name(&self) -> &'static str {
        "printenv"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        printenv_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Printenv;

        // 测试 name 方法
        assert_eq!(tool.name(), "printenv");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("printenv"));

        // 测试 execute 方法
        let args = vec![OsString::from("printenv"), OsString::from("--version")];
        assert!(tool.execute(&args).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn accepts_non_utf8_variable_name() {
        use std::os::unix::ffi::OsStringExt;

        let variable = OsString::from_vec(b"\xffPRINTENV_TEST".to_vec());
        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name().into(), "--".into(), variable.clone()])
            .expect("non-UTF-8 variable name should parse");

        assert_eq!(
            matches
                .get_many::<OsString>(PRINTENV_ARG_VARIABLES)
                .expect("variable")
                .collect::<Vec<_>>(),
            [&variable]
        );
    }

    #[test]
    fn writes_non_utf8_environment_value_verbatim() {
        let options = PrintenvOptions {
            separator: b'\n',
            variables: Vec::new(),
        };
        let mut output = Vec::new();

        let exit_code = write_environment_entries(&mut output, &options, &[b"RAW=\xff".to_vec()])
            .expect("write output");

        assert_eq!(exit_code, 0);
        assert_eq!(output, b"RAW=\xff\n");
    }

    #[cfg(unix)]
    #[test]
    fn closed_stdout_write_error_uses_bad_file_descriptor() {
        let error = printenv_normalize_stdout_write_error(
            io::Error::from_raw_os_error(ctcore::libc::ENOSPC),
            true,
        );

        assert_eq!(error.raw_os_error(), Some(ctcore::libc::EBADF));
    }

    #[cfg(unix)]
    #[test]
    fn writes_value_for_non_utf8_variable_name() {
        use std::os::unix::ffi::OsStringExt;

        let options = PrintenvOptions {
            separator: b'\n',
            variables: vec![OsString::from_vec(b"\xffRAW".to_vec())],
        };
        let mut output = Vec::new();

        let exit_code =
            write_environment_entries(&mut output, &options, &[b"\xffRAW=value".to_vec()])
                .expect("write output");

        assert_eq!(exit_code, 0);
        assert_eq!(output, b"value\n");
    }

    mod tests_printenv_main {
        use crate::printenv_main;

        use std::ffi::OsString;

        #[test]
        fn test_printenv_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = printenv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = printenv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = printenv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = printenv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main() {
            let args = [ctcore::ct_util_name()];
            let result = printenv_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    mod tests_printenv_app {
        use crate::{OsString, PRINTENV_ARG_VARIABLES, PRINTENV_OPT_NULL, ct_app};

        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn stops_option_parsing_after_first_variable() {
            let command = ct_app();
            let matches = command
                .try_get_matches_from([ctcore::ct_util_name(), "PRINTENV_TEST", "-0"])
                .expect("arguments should parse");

            assert_eq!(matches.get_count(PRINTENV_OPT_NULL), 0);
            assert_eq!(
                matches
                    .get_many::<OsString>(PRINTENV_ARG_VARIABLES)
                    .expect("variables")
                    .map(|value| value.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                ["PRINTENV_TEST", "-0"]
            );
        }

        #[test]
        fn accepts_repeated_null_option() {
            let command = ct_app();
            let matches = command
                .try_get_matches_from([ctcore::ct_util_name(), "-00", "PRINTENV_TEST"])
                .expect("repeated -0 should parse");

            assert_eq!(matches.get_count(PRINTENV_OPT_NULL), 2);
        }

        #[test]
        fn rejects_unknown_short_option_before_variable() {
            let error = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), "-a"])
                .expect_err("unknown option should not be a variable");

            assert_eq!(error.kind(), ErrorKind::UnknownArgument);
        }
    }
}
