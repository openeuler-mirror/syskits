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

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError};
use ctcore::ct_format::{ArgCursor, FormatArgument, parse_spec_and_escape};
use std::ffi::OsString;
use sys_locale::get_locale;

const PRINTF_VERSION: &str = "version";
const PRINTF_HELP: &str = "help";

mod opt_flags {
    pub const PRINTF_FORMATSTRING: &str = "FORMATSTRING";
    pub const PRINTF_ARGUMENT: &str = "ARGUMENT";
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

/// 主函数，用于处理命令行输入并格式化输出。
pub fn printf_main(args: impl ctcore::Args) -> CTResult<()> {
    unsafe {
        // Follow GNU printf semantics: initialize C locale from environment
        // so localeconv() exposes LC_NUMERIC grouping/thousands_sep.
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr() as *const _);
    }

    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app().get_matches_from(args);

    let format_string = args_match
        .get_one::<String>(opt_flags::PRINTF_FORMATSTRING)
        .ok_or_else(|| CTsageError::new(1, "missing operand"))?;

    let var: Vec<_> = match args_match.get_many::<String>(opt_flags::PRINTF_ARGUMENT) {
        Some(s) => s.map(|s| FormatArgument::Unparsed(s.to_string())).collect(),
        None => vec![],
    };

    let mut args_slice = var.as_slice();
    let stdout = stdout();
    let mut handle = stdout.lock();

    // 核心大循环：不断重复 format 字符串，直到所有参数耗尽
    loop {
        let mut cursor = ArgCursor::new(args_slice);

        for item in parse_spec_and_escape(format_string.as_bytes()) {
            // 这里改用 CtSimpleError
            let item = item.map_err(|e| CtSimpleError::new(1, e.to_string()))?;
            match item
                .write(&mut handle, &mut cursor)
                .map_err(|e| CtSimpleError::new(1, e.to_string()))?
            {
                ControlFlow::Break(()) => {
                    handle
                        .flush()
                        .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
                    return Ok(());
                }
                ControlFlow::Continue(()) => {}
            }
        }
        let consumed = cursor.consumed_count();

        // 防死循环：如果这一轮一个参数都没吃掉，证明格式化字符串是个“无底洞”，强制退出
        if consumed == 0 {
            break;
        }

        // 如果吃的参数比剩下的还多，说明参数已经吃干净了
        if consumed >= args_slice.len() {
            break;
        }

        // 往前推进切片窗口
        args_slice = &args_slice[consumed..];
    }

    handle
        .flush()
        .map_err(|e| CtSimpleError::new(1, e.to_string()))?;

    Ok(())
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
}
