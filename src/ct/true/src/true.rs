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
use clap::{Arg, ArgAction, Command};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, set_ct_exit_code};
use std::{ffi::OsString, io::Write};
use sys_locale::get_locale;
#[derive(Default)]
pub struct True;
impl Tool for True {
    fn name(&self) -> &'static str {
        "true"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        true_main(args.iter().cloned())
    }
}

/// 主程序入口，处理命令行参数并执行相应操作。
///
/// # 参数
/// `args`: 实现了 `ctcore::Args` 接口的对象，代表命令行传入的参数。
///
/// # 返回值
///
/// 返回一个 `CTResult<()>`，成功时为 `Ok(())`，错误时为 `Err(_)`。
pub fn true_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut command = ct_app(); // 创建命令行解析器

    let input_args: Vec<OsString> = args.collect(); // 从 `ctcore::Args` 收集命令行参数
    if input_args.len() > 2 {
        // 如果参数数量超过2个，直接返回成功，不进行进一步的解析
        return Ok(());
    }

    args_process(&mut command, input_args)
}

fn args_process(command: &mut Command, args: Vec<OsString>) -> CTResult<()> {
    if let Err(e) = command.try_get_matches_from_mut(args) {
        // 尝试从参数列表中获取匹配项，如果失败则根据错误类型处理
        let error = match e.kind() {
            clap::error::ErrorKind::DisplayHelp => command.print_help(), // 显示帮助信息
            clap::error::ErrorKind::DisplayVersion => {
                writeln!(std::io::stdout(), "{}", command.render_version()) // 显示版本信息
            }
            _ => Ok(()), // 其他错误类型不处理，直接返回成功
        };

        if let Err(print_fail) = error {
            // 如果错误信息打印失败，则在标准错误输出打印错误，并设置退出码
            let _ = writeln!(
                std::io::stderr(),
                "{}: {}",
                ctcore::ct_util_name(),
                print_fail
            );
            set_ct_exit_code(1); // 设置退出码为1，表示错误
        }
    }
    Ok(())
}

/// 创建并配置命令行解析器。
///
/// # 返回值
/// 返回一个已配置的 `Command` 对象，用于进一步的命令行参数解析。
pub fn ct_app() -> Command {
    Command::new(ctcore::ct_util_name())
        .version(clap::crate_version!()) // 设置程序版本
        .about(t!("true.about")) // 设置程序简介
        // 禁用默认的帮助和版本标志，以确保与 GNU 最大程度的兼容
        .disable_help_flag(true)
        .disable_version_flag(true)
        // 添加自定义的帮助和版本选项
        .arg(
            Arg::new("help")
                .long("help")
                .help(t!("true.clap.help"))
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .help(t!("true.clap.version"))
                .action(ArgAction::Version),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = True::default();

        // Test name method
        assert_eq!(tool.name(), "true");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("true"));

        // Test execute method (should always succeed)
        let args: Vec<OsString> = vec![OsString::from("true"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_ok());
    }

    mod tests_true_main {
        use crate::true_main;

        use std::ffi::OsString;

        #[test]
        fn test_true_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = true_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_true_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = true_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }

    mod tests_true_app {
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
