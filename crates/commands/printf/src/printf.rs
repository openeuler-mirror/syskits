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
use std::io::{Write, stderr, stdout};
rust_i18n::i18n!("locales", fallback = "en-US");
use std::ops::ControlFlow;

use clap::{Arg, ArgAction, ArgMatches, Command, builder::OsStringValueParser, crate_version};
use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, CTsageError, CtSimpleError};
use ctcore::ct_format::{ArgCursor, FormatArgument, parse_spec_and_escape};
use ctcore::ct_quoting_style::{CtQuotes, CtQuotingStyle, escape_name};
use std::borrow::Cow;
use std::ffi::{CStr, OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use sys_locale::get_locale;

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
    format_string: Vec<u8>,
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
        .get_one::<OsString>(opt_flags::PRINTF_FORMATSTRING)
        .ok_or_else(|| CTsageError::new(1, "missing operand"))?
        .as_encoded_bytes()
        .to_vec();

    let arguments = match args_match.get_many::<OsString>(opt_flags::PRINTF_ARGUMENT) {
        Some(values) => values
            .map(|value| FormatArgument::Bytes(value.as_encoded_bytes().to_vec()))
            .collect(),
        None => Vec::new(),
    };

    Ok(PrintfInvocation {
        format_string,
        arguments,
    })
}

fn printf_special_output(argv: &[OsString]) -> Option<Vec<u8>> {
    if argv.len() != 2 {
        return None;
    }

    match argv[1].as_encoded_bytes() {
        b"--help" => {
            let mut command = ct_app();
            let mut output = command.render_long_help().to_string().into_bytes();
            output.push(b'\n');
            Some(output)
        }
        b"--version" => Some(format!("printf {}\n", crate_version!()).into_bytes()),
        _ => None,
    }
}

fn printf_render_to_writer<W: Write>(
    invocation: &PrintfInvocation,
    writer: &mut W,
) -> CTResult<String> {
    let mut args_slice = invocation.arguments.as_slice();
    let mut stderr_text = String::new();

    loop {
        let mut cursor = ArgCursor::new(args_slice);

        for item in parse_spec_and_escape(&invocation.format_string) {
            let item = item.map_err(|err| CtSimpleError::new(1, err.to_string()))?;
            match item
                .write(&mut *writer, &mut cursor)
                .map_err(|err| CtSimpleError::new(1, err.to_string()))?
            {
                ControlFlow::Break(()) => return Ok(stderr_text),
                ControlFlow::Continue(()) => {}
            }
        }

        let consumed = cursor.consumed_count();
        if consumed == 0 {
            if let Some(first_excess) = args_slice.first() {
                stderr_text.push_str(&printf_excess_arguments_warning(first_excess));
            }
            break;
        }
        if consumed >= args_slice.len() {
            break;
        }

        args_slice = &args_slice[consumed..];
    }

    Ok(stderr_text)
}

fn printf_excess_arguments_warning(arg: &FormatArgument) -> String {
    format!(
        "printf: warning: ignoring excess arguments, starting with {}\n",
        printf_argument_for_warning_with_style(arg, printf_uses_ascii_quotes())
    )
}

fn printf_uses_ascii_quotes() -> bool {
    let locale = unsafe { ctcore::libc::setlocale(ctcore::libc::LC_CTYPE, std::ptr::null()) };
    if locale.is_null() {
        return false;
    }
    matches!(
        unsafe { CStr::from_ptr(locale) }.to_bytes(),
        b"C" | b"POSIX"
    )
}

fn printf_argument_for_warning_with_style(arg: &FormatArgument, ascii_quotes: bool) -> String {
    let value: Cow<'_, OsStr> = match arg {
        FormatArgument::Unparsed(s) | FormatArgument::String(s) => Cow::Borrowed(OsStr::new(s)),
        FormatArgument::Bytes(bytes) => Cow::Borrowed(OsStr::from_bytes(bytes)),
        FormatArgument::Char(c) => Cow::Owned(OsString::from(c.to_string())),
        FormatArgument::UnsignedInt(n) => Cow::Owned(OsString::from(n.to_string())),
        FormatArgument::SignedInt(n) => Cow::Owned(OsString::from(n.to_string())),
        FormatArgument::Float(n) => Cow::Owned(OsString::from(n.to_string())),
    };

    if ascii_quotes {
        escape_name(
            &value,
            &CtQuotingStyle::C {
                quotes: CtQuotes::Single,
            },
        )
    } else {
        format!("‘{}’", value.to_string_lossy())
    }
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
    let argv: Vec<OsString> = args.collect();
    if let Some(output) = printf_special_output(&argv) {
        writer
            .write_all(&output)
            .map_err(|err| CtSimpleError::new(1, err.to_string()))?;
        writer
            .flush()
            .map_err(|err| CtSimpleError::new(1, err.to_string()))?;
        return Ok(());
    }

    let args_match = ct_app().get_matches_from(argv);
    let invocation = printf_invocation_from_matches(&args_match)?;
    let stderr_text = printf_render_to_writer(&invocation, &mut writer)?;
    writer
        .flush()
        .map_err(|err| CtSimpleError::new(1, err.to_string()))?;
    if !stderr_text.is_empty() {
        stderr()
            .lock()
            .write_all(stderr_text.as_bytes())
            .map_err(|err| CtSimpleError::new(1, err.to_string()))?;
    }
    Ok(())
}

pub fn printf_native_semantic(args: impl ctcore::Args) -> CTResult<PrintfSemantic> {
    init_printf_locale();

    let argv: Vec<OsString> = args.collect();
    if let Some(output) = printf_special_output(&argv) {
        let format_string = argv[1].to_string_lossy();
        return Ok(printf_semantic_from_output(
            &format_string,
            output,
            String::new(),
            0,
        ));
    }

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
    let format_string = String::from_utf8_lossy(&invocation.format_string);
    match printf_render_to_writer(&invocation, &mut output) {
        Ok(stderr_text) => Ok(printf_semantic_from_output(
            &format_string,
            output,
            stderr_text,
            0,
        )),
        Err(err) => Ok(printf_semantic_from_output(
            &format_string,
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
        Arg::new(opt_flags::PRINTF_FORMATSTRING).value_parser(OsStringValueParser::new()),
        Arg::new(opt_flags::PRINTF_ARGUMENT)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new()),
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

            assert!(result.is_ok());
        }

        #[test]
        fn test_printf_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = printf_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_printf_main_f() {
            let args = [ctcore::ct_util_name(), "%0.3f", "1.23456"];
            let result = printf_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    mod tests_printf_app {
        use crate::{ct_app, opt_flags};

        #[cfg(unix)]
        use std::os::unix::ffi::OsStringExt;

        #[test]
        fn test_ct_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_one::<std::ffi::OsString>(opt_flags::PRINTF_FORMATSTRING)
                    .unwrap(),
                "--version"
            );
        }

        #[test]
        fn test_ct_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_one::<std::ffi::OsString>(opt_flags::PRINTF_FORMATSTRING)
                    .unwrap(),
                "--help"
            );
        }

        #[cfg(unix)]
        #[test]
        fn accepts_non_utf8_linux_arguments() {
            let args = vec![
                std::ffi::OsString::from("printf"),
                std::ffi::OsString::from("%s"),
                std::ffi::OsString::from_vec(vec![0xff]),
            ];

            assert!(ct_app().try_get_matches_from(args).is_ok());
        }
    }

    mod tests_printf_semantic {
        use crate::{
            FormatArgument, PrintfInvocation, printf_argument_for_warning_with_style,
            printf_native_semantic, printf_render_to_writer,
        };

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
        fn semantic_warns_about_excess_arguments_when_format_consumes_none() {
            let args = [ctcore::ct_util_name(), "foo", "alpha", "beta"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "foo");
            assert_eq!(
                semantic.stderr_text,
                "printf: warning: ignoring excess arguments, starting with ‘alpha’\n"
            );
            assert_eq!(semantic.exit_code, 0);
        }

        #[test]
        fn c_locale_excess_argument_warning_uses_ascii_quotes() {
            let argument = FormatArgument::Bytes(b"a'b".to_vec());

            assert_eq!(
                printf_argument_for_warning_with_style(&argument, true),
                "'a\\'b'"
            );
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

        #[test]
        fn semantic_rejects_hex_escape_without_digits() {
            let args = [ctcore::ct_util_name(), "\\x"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "");
            assert_eq!(
                semantic.stderr_text,
                "printf: missing hexadecimal number in escape\n"
            );
            assert_eq!(semantic.exit_code, 1);
            assert!(semantic.rows.is_empty());
        }

        #[test]
        fn semantic_preserves_partial_output_before_incomplete_hex_escape() {
            let args = [ctcore::ct_util_name(), "alpha\\x"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "alpha");
            assert_eq!(
                semantic.stderr_text,
                "printf: missing hexadecimal number in escape\n"
            );
            assert_eq!(semantic.exit_code, 1);
            assert_eq!(semantic.rows.len(), 1);
            assert_eq!(semantic.rows[0].text, "alpha");
            assert!(!semantic.rows[0].terminated);
        }

        #[test]
        fn semantic_preserves_percent_b_output_before_incomplete_hex_escape() {
            let args = [ctcore::ct_util_name(), "%b", "alpha\\x"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "alpha");
            assert_eq!(
                semantic.stderr_text,
                "printf: missing hexadecimal number in escape\n"
            );
            assert_eq!(semantic.exit_code, 1);
            assert_eq!(semantic.rows.len(), 1);
            assert_eq!(semantic.rows[0].text, "alpha");
            assert!(!semantic.rows[0].terminated);
        }

        #[test]
        fn semantic_percent_b_c_stops_entire_output() {
            let args = [ctcore::ct_util_name(), "%bX", "a\\cb"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "a");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
        }

        #[test]
        fn quoted_string_with_missing_argument_outputs_nothing() {
            let args = [ctcore::ct_util_name(), "%q"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
        }

        #[test]
        fn integer_arguments_accept_plus_and_wrap_negative_unsigned_values() {
            let args = [ctcore::ct_util_name(), "%d|%u", "+3", "-1"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "3|18446744073709551615");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
        }

        #[test]
        fn integer_overflow_outputs_saturated_values() {
            let args = [
                ctcore::ct_util_name(),
                "%d|%d|%u|%u",
                "9223372036854775808",
                "-9223372036854775809",
                "18446744073709551616",
                "-18446744073709551616",
            ];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(
                semantic.classic_text,
                "9223372036854775807|-9223372036854775808|18446744073709551615|18446744073709551615"
            );
        }

        #[test]
        fn integer_fields_place_sign_prefix_and_zero_padding_like_printf() {
            let args = [
                ctcore::ct_util_name(),
                "%05d|%+5.3d|%#08x|%.0d",
                "-42",
                "1",
                "42",
                "0",
            ];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "-0042| +001|0x00002a|");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
        }

        #[test]
        fn float_arguments_accept_exponent_hex_and_infinity_syntax() {
            let args = [
                ctcore::ct_util_name(),
                "%f|%f|%f",
                "1e2",
                "0x1p3",
                "infinity",
            ];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "100.000000|8.000000|inf");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
        }

        #[test]
        fn semantic_zero_pads_float_after_explicit_sign() {
            let args = [ctcore::ct_util_name(), "%+08.2f", "1.25"];

            let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.classic_text, "+0001.25");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
        }

        #[test]
        fn help_and_version_are_formats_when_followed_by_arguments() {
            for format in ["--help", "--version"] {
                let args = [ctcore::ct_util_name(), format, "x"];

                let semantic = printf_native_semantic(args.iter().map(OsString::from)).unwrap();

                assert_eq!(semantic.classic_text, format);
                assert_eq!(
                    semantic.stderr_text,
                    "printf: warning: ignoring excess arguments, starting with ‘x’\n"
                );
                assert_eq!(semantic.exit_code, 0);
            }
        }

        #[test]
        fn string_precision_truncates_at_a_utf8_byte_boundary() {
            let invocation = PrintfInvocation {
                format_string: b"%.1s".to_vec(),
                arguments: vec![FormatArgument::Unparsed("é".to_string())],
            };
            let mut output = Vec::new();

            let stderr_text = printf_render_to_writer(&invocation, &mut output).unwrap();

            assert_eq!(output, vec![0xc3]);
            assert_eq!(stderr_text, "");
        }

        #[test]
        fn non_utf8_character_constant_uses_its_first_byte() {
            let invocation = PrintfInvocation {
                format_string: b"%d".to_vec(),
                arguments: vec![FormatArgument::Bytes(vec![b'\'', 0xff])],
            };
            let mut output = Vec::new();

            let stderr_text = printf_render_to_writer(&invocation, &mut output).unwrap();

            assert_eq!(output, b"255");
            assert_eq!(stderr_text, "");
        }
    }
}
