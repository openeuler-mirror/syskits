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
use clap::{Arg, ArgAction, Command};
use ctcore::Tool;
use ctcore::ct_error::{CTResult, set_ct_exit_code};
use ctcore::ct_help_about;
use std::{ffi::OsString, io::Write};

const FALSE_ABOUT: &str = ct_help_about!("false.md");

#[derive(Default)]
pub struct False;
impl Tool for False {
    fn name(&self) -> &'static str {
        "false"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        false_main(args.iter().cloned())
    }
}

/// 主函数，负责处理命令行输入并调用相应的操作。
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    false_main(args).map(|_| ())
}

pub fn false_main(args: impl ctcore::Args) -> CTResult<()> {
    let mut command = ct_app(); // 创建命令行解析器实例

    // 设置退出码为1，遵循GNU规范，即使在成功的情况下也返回1。
    set_ct_exit_code(1);

    let input_args: Vec<OsString> = args.collect(); // 收集命令行参数
    if input_args.len() > 2 {
        // 如果参数数量超过2个，直接返回成功，不进行进一步解析。
        return Ok(());
    }

    // 尝试从参数中获取匹配项，若失败则根据错误类型处理。
    args_process(&mut command, input_args)
}

fn args_process(command: &mut Command, args: Vec<OsString>) -> CTResult<()> {
    if let Err(e) = command.try_get_matches_from_mut(args) {
        let error = match e.kind() {
            // 如果是显示帮助信息的错误，则显示帮助信息。
            clap::error::ErrorKind::DisplayHelp => command.print_help(),
            // 如果是显示版本信息的错误，则显示版本信息。
            clap::error::ErrorKind::DisplayVersion => {
                writeln!(std::io::stdout(), "{}", command.render_version())
            }
            // 对于其他类型的错误，不进行处理。
            _ => Ok(()),
        };

        // 尝试显示错误信息，如果显示失败，则将错误信息写入标准错误输出。
        if let Err(print_fail) = error {
            let _ = writeln!(
                std::io::stderr(),
                "{}: {}",
                ctcore::ct_util_name(),
                print_fail
            );
        }
    }

    Ok(())
}

/// 创建并配置命令行解析器实例。
pub fn ct_app() -> Command {
    Command::new(ctcore::ct_util_name())
        .version(clap::crate_version!())
        .about(FALSE_ABOUT)
        // We provide our own help and version options, to ensure maximum compatibility with GNU.
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .long("help")
                .help("Print help information")
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .help("Print version information")
                .action(ArgAction::Version),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_implementation() {
        let tool = False;

        // Test name method
        assert_eq!(tool.name(), "false");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("false"));

        // Test execute method - should return an error since false always exits with non-zero
        let args = vec![OsString::from("false")];
        assert!(tool.execute(&args).is_ok());
    }

    mod tests_echo_main {
        use crate::false_main;

        use std::ffi::OsString;

        #[test]
        fn test_false_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = false_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_false_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = false_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }

    mod tests_false_app {
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
