/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */
#![allow(dead_code)]
// 在Linux或类Unix系统中，printf 是一个内置的命令，它基于C语言的printf 函数，用于格式化输出数据。
// printf 命令允许你控制输出的布局，包括数值的宽度、精度、对齐方式等

use std::io::stdout;
use std::ops::ControlFlow;

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CTsageError};
use ctcore::ct_format::{FormatArgument, FormatItem, parse_spec_and_escape};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
use std::ffi::OsString;

const PRINTF_VERSION: &str = "version";
const PRINTF_HELP: &str = "help";
const PRINTF_USAGE: &str = ct_help_usage!("printf.md");
const PRINTF_ABOUT: &str = ct_help_about!("printf.md");
const PRINTF_AFTER_HELP: &str = ct_help_section!("after help", "printf.md");

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

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    printf_main(args).map(|_| ())
}

/// 主函数，用于处理命令行输入并格式化输出。
///
/// # 参数
/// `args`: 实现了 `ctcore::Args` 接口的对象，代表命令行参数。
///
/// # 返回值
/// 返回一个 `CTResult<()>`，成功时为 `Ok(())`，错误时为 `Err(CTsageError)`。
pub fn printf_main(args: impl ctcore::Args) -> CTResult<()> {
    // 从命令行参数中获取匹配项
    let args_match = ct_app().get_matches_from(args);

    // 获取格式化字符串参数
    let format_string = args_match
        .get_one::<String>(opt_flags::PRINTF_FORMATSTRING)
        .ok_or_else(|| CTsageError::new(1, "missing operand"))?;

    // 解析额外的参数，并准备格式化参数列表
    let var: Vec<_> = match args_match.get_many::<String>(opt_flags::PRINTF_ARGUMENT) {
        Some(s) => s.map(|s| FormatArgument::Unparsed(s.to_string())).collect(),
        None => vec![],
    };

    // 标记是否在格式化字符串中发现了格式化规范
    let mut is_format_seen = false;

    // 第一次遍历：处理格式化字符串中的所有项目
    let mut format_args = var.iter().peekable();
    for item in parse_spec_and_escape(format_string.as_ref()) {
        if let Ok(FormatItem::Spec(_)) = item {
            is_format_seen = true;
        }
        match item?.write(stdout(), &mut format_args)? {
            ControlFlow::Continue(()) => {}
            ControlFlow::Break(()) => return Ok(()),
        };
    }
    // 如果格式化字符串中没有格式化规范，则提前退出，避免无限循环
    if !is_format_seen {
        return Ok(());
    }

    // 第二次遍历：处理剩余的参数
    while format_args.peek().is_some() {
        for format_arg in parse_spec_and_escape(format_string.as_ref()) {
            match format_arg?.write(stdout(), &mut format_args)? {
                ControlFlow::Continue(()) => {}
                ControlFlow::Break(()) => return Ok(()),
            };
        }
    }

    Ok(())
}

/// 构建命令行解析器对象。
///
/// # 返回值
/// 返回一个配置好的 `Command` 对象，用于解析命令行参数。
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = PRINTF_ABOUT;
    let usage_description = ct_format_usage(PRINTF_USAGE);
    let after_help = PRINTF_AFTER_HELP;

    let args = vec![
        Arg::new(PRINTF_HELP)
            .long(PRINTF_HELP)
            .help("Print help information")
            .action(ArgAction::Help),
        Arg::new(PRINTF_VERSION)
            .long(PRINTF_VERSION)
            .help("Print version information")
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

    mod tests_printf_main {
        use crate::printf_main;

        use std::ffi::OsString;

        #[test]
        fn test_printf_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = printf_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_printf_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = printf_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_printf_main_f() {
            let args = vec![ctcore::ct_util_name(), "%0.3f", "1.23456"];
            let result = printf_main(args.iter().map(|s| OsString::from(s)));

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
