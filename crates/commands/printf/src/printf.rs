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
#![allow(dead_code)]
// 在Linux或类Unix系统中，printf 是一个内置的命令，它基于C语言的printf 函数，用于格式化输出数据。
// printf 命令允许你控制输出的布局，包括数值的宽度、精度、对齐方式等

extern crate rust_i18n;
use rust_i18n::t;
use std::io::{Write, stdout};
rust_i18n::i18n!("locales", fallback = "en-US");
use std::ops::ControlFlow;

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, CTsageError, CtSimpleError};
use ctcore::ct_format::{ArgCursor, FormatArgument, parse_spec_and_escape};
use std::ffi::OsString;
use sys_locale::get_locale;

const PRINTF_VERSION: &str = "version";
const PRINTF_HELP: &str = "help";

mod opt_flags {
    pub const PRINTF_FORMATSTRING: &str = "FORMATSTRING";
    pub const PRINTF_ARGUMENT: &str = "ARGUMENT";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintfSemanticRow {
    pub line_index: usize,
    pub text: String,
    pub byte_len: usize,
    pub terminated: bool,
    pub format_string: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintfSemantic {
    pub rows: Vec<PrintfSemanticRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct PrintfInvocation {
    format_string: String,
    arguments: Vec<FormatArgument>,
}

#[derive(Default)]
pub struct Printf;
impl Tool for Printf {
    fn name(&self) -> &'static str {
        "printf"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        printf_main(args.iter().cloned())
    }
}

fn init_printf_locale() {
    unsafe {
        // Follow GNU printf semantics: initialize C locale from environment
        // so localeconv() exposes LC_NUMERIC grouping/thousands_sep.
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr() as *const _);
    }

    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
}

fn printf_invocation_from_matches(args_match: &ArgMatches) -> CTResult<PrintfInvocation> {
    let format_string = args_match
        .get_one::<String>(opt_flags::PRINTF_FORMATSTRING)
        .ok_or_else(|| CTsageError::new(1, "missing operand"))?
        .to_string();

    let arguments = match args_match.get_many::<String>(opt_flags::PRINTF_ARGUMENT) {
        Some(values) => values
            .map(|value| FormatArgument::Unparsed(value.to_string()))
            .collect(),
        None => Vec::new(),
    };

    Ok(PrintfInvocation {
        format_string,
        arguments,
    })
}

fn printf_render_to_writer<W: Write>(
    invocation: &PrintfInvocation,
    writer: &mut W,
) -> CTResult<()> {
    let mut args_slice = invocation.arguments.as_slice();

    loop {
        let mut cursor = ArgCursor::new(args_slice);

        for item in parse_spec_and_escape(invocation.format_string.as_bytes()) {
            let item = item.map_err(|err| CtSimpleError::new(1, err.to_string()))?;
            match item
                .write(&mut *writer, &mut cursor)
                .map_err(|err| CtSimpleError::new(1, err.to_string()))?
            {
                ControlFlow::Break(()) => return Ok(()),
                ControlFlow::Continue(()) => {}
            }
        }

        let consumed = cursor.consumed_count();
        if consumed == 0 || consumed >= args_slice.len() {
            break;
        }

        args_slice = &args_slice[consumed..];
    }

    Ok(())
}

fn printf_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("printf: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'printf --help' for more information.\n");
    }
    stderr
}

fn printf_rows_from_output(format_string: &str, output: &[u8]) -> Vec<PrintfSemanticRow> {
    let mut rows = Vec::new();
    let mut start = 0usize;
    let mut line_index = 1usize;

    for (index, byte) in output.iter().enumerate() {
        if *byte == b'\n' {
            rows.push(PrintfSemanticRow {
                line_index,
                text: String::from_utf8_lossy(&output[start..index]).into_owned(),
                byte_len: index - start,
                terminated: true,
                format_string: format_string.to_string(),
            });
            start = index + 1;
            line_index += 1;
        }
    }

    if start < output.len() {
        rows.push(PrintfSemanticRow {
            line_index,
            text: String::from_utf8_lossy(&output[start..]).into_owned(),
            byte_len: output.len() - start,
            terminated: false,
            format_string: format_string.to_string(),
        });
    }

    rows
}

fn printf_semantic_from_output(
    format_string: &str,
    output: Vec<u8>,
    stderr_text: String,
    exit_code: i32,
) -> PrintfSemantic {
    PrintfSemantic {
        rows: printf_rows_from_output(format_string, &output),
        classic_text: String::from_utf8_lossy(&output).into_owned(),
        stderr_text,
        exit_code,
    }
}

fn printf_semantic_from_parse_error(err: clap::Error) -> PrintfSemantic {
    let rendered = err.to_string();
    if err.use_stderr() {
        PrintfSemantic {
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text: rendered,
            exit_code: 1,
        }
    } else {
        PrintfSemantic {
            rows: Vec::new(),
            classic_text: rendered,
            stderr_text: String::new(),
            exit_code: 0,
        }
    }
}

/// 主函数，用于处理命令行输入并格式化输出。
pub fn printf_main(args: impl ctcore::Args) -> CTResult<()> {
    let stdout = stdout();
    printf_main_with_writer(args, stdout.lock())
}

pub fn printf_main_with_writer<W: Write>(args: impl ctcore::Args, mut writer: W) -> CTResult<()> {
    init_printf_locale();
    let args_match = ct_app().get_matches_from(args);
    let invocation = printf_invocation_from_matches(&args_match)?;
    printf_render_to_writer(&invocation, &mut writer)?;
    writer
        .flush()
        .map_err(|err| CtSimpleError::new(1, err.to_string()))?;
    Ok(())
}

pub fn printf_native_semantic(args: impl ctcore::Args) -> CTResult<PrintfSemantic> {
    init_printf_locale();

    let argv: Vec<OsString> = args.collect();
    let matches = match ct_app().try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(err) => return Ok(printf_semantic_from_parse_error(err)),
    };

    let invocation = match printf_invocation_from_matches(&matches) {
        Ok(invocation) => invocation,
        Err(err) => {
            return Ok(PrintfSemantic {
                rows: Vec::new(),
                classic_text: String::new(),
                stderr_text: printf_error_text(err.as_ref()),
                exit_code: err.code(),
            });
        }
    };

    let mut output = Vec::new();
    match printf_render_to_writer(&invocation, &mut output) {
        Ok(()) => Ok(printf_semantic_from_output(
            &invocation.format_string,
            output,
            String::new(),
            0,
        )),
        Err(err) => Ok(printf_semantic_from_output(
            &invocation.format_string,
            output,
            printf_error_text(err.as_ref()),
            err.code(),
        )),
    }
}

/// 构建命令行解析器对象。
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("printf.about");
    let usage_description = t!("printf.usage");
    let after_help = t!("printf.after_help");

    let args = vec![
        Arg::new(PRINTF_HELP)
            .long(PRINTF_HELP)
            .help(t!("printf.clap.printf_help"))
            .action(ArgAction::Help),
        Arg::new(PRINTF_VERSION)
            .long(PRINTF_VERSION)
            .help(t!("printf.clap.printf_version"))
            .action(ArgAction::Version),
        Arg::new(opt_flags::PRINTF_FORMATSTRING),
        Arg::new(opt_flags::PRINTF_ARGUMENT).action(ArgAction::Append),
    ];

    Command::new(utility_name)
        .allow_hyphen_values(true)
        .version(command_version)
        .about(application_info)
        .after_help(after_help)
        .override_usage(usage_description)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Printf;

        // 测试 name 方法
        assert_eq!(tool.name(), "printf");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("printf"));

        // 测试 execute 方法 - 有效的格式化字符串和参数
        let args = vec![
            OsString::from("printf"),
            OsString::from("%s"),
            OsString::from("test"),
        ];
        assert!(tool.execute(&args).is_ok());
    }

    mod tests_printf_main {
        use crate::printf_main;

        use std::ffi::OsString;

        #[test]
        fn test_printf_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = printf_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printf_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = printf_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printf_main_f() {
            let args = [ctcore::ct_util_name(), "%0.3f", "1.23456"];
            let result = printf_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    mod tests_printf_app {
        use crate::ct_app;

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
    }

    mod tests_printf_semantic {
        use crate::printf_native_semantic;

        use std::ffi::OsString;

        #[test]
        fn semantic_collects_rows_without_trailing_newline() {
            let args = [ctcore::ct_util_name(), "%s", "alpha"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "alpha");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
            assert_eq!(semantic.rows.len(), 1);
            assert_eq!(semantic.rows[0].line_index, 1);
            assert_eq!(semantic.rows[0].text, "alpha");
            assert_eq!(semantic.rows[0].byte_len, 5);
            assert!(!semantic.rows[0].terminated);
            assert_eq!(semantic.rows[0].format_string, "%s");
        }

        #[test]
        fn semantic_collects_multiple_lines_with_termination() {
            let args = [ctcore::ct_util_name(), "alpha\nbeta\n"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "alpha\nbeta\n");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
            assert_eq!(semantic.rows.len(), 2);
            assert_eq!(semantic.rows[0].text, "alpha");
            assert!(semantic.rows[0].terminated);
            assert_eq!(semantic.rows[1].text, "beta");
            assert!(semantic.rows[1].terminated);
        }

        #[test]
        fn semantic_preserves_partial_output_on_error() {
            let args = [ctcore::ct_util_name(), "alpha%z"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "alpha");
            assert_eq!(
                semantic.stderr_text,
                "printf: %z: invalid conversion specification\n"
            );
            assert_eq!(semantic.exit_code, 1);
            assert_eq!(semantic.rows.len(), 1);
            assert_eq!(semantic.rows[0].text, "alpha");
            assert!(!semantic.rows[0].terminated);
        }
    }
}
