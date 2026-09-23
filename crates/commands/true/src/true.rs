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
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, set_ct_exit_code, strip_errno};
use std::{
    ffi::OsString,
    io::{self, Write},
};
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

    #[cfg(unix)]
    true_restore_default_sigpipe(ctcore::ct_sigpipe_was_default());

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
            let print_fail = true_display_output_error(print_fail, ctcore::ct_stdout_was_closed());
            // 如果错误信息打印失败，则在标准错误输出打印错误，并设置退出码
            let _ = writeln!(
                std::io::stderr(),
                "{}: {}",
                ctcore::ct_util_name(),
                true_write_error_message(&print_fail)
            );
            set_ct_exit_code(1); // 设置退出码为1，表示错误
        }
    }
    Ok(())
}

fn true_write_error_message(error: &io::Error) -> String {
    format!("write error: {}", strip_errno(error))
}

fn true_display_output_error(error: io::Error, stdout_was_closed: bool) -> io::Error {
    true_closed_stdout_error(stdout_was_closed).unwrap_or(error)
}

fn true_closed_stdout_error(stdout_was_closed: bool) -> Option<io::Error> {
    #[cfg(unix)]
    {
        stdout_was_closed.then(|| io::Error::from_raw_os_error(libc::EBADF))
    }

    #[cfg(not(unix))]
    {
        let _ = stdout_was_closed;
        None
    }
}

#[cfg(unix)]
fn true_restore_default_sigpipe(was_default: bool) {
    if true_should_restore_default_sigpipe(was_default) {
        let _ = ctcore::ct_signals::enable_pipe_errors();
    }
}

#[cfg(unix)]
fn true_should_restore_default_sigpipe(was_default: bool) -> bool {
    was_default
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
    use std::io;

    #[test]
    fn test_tool_implementation() {
        let tool = True;

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
            let args = [ctcore::ct_util_name(), "--version"];

            let result = true_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_true_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = true_main(args.iter().map(OsString::from));

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

    #[cfg(unix)]
    #[test]
    fn true_display_output_error_preserves_enospc_or_restores_closed_stdout_ebadf() {
        let enospc = io::Error::from_raw_os_error(libc::ENOSPC);
        assert_eq!(
            true_display_output_error(enospc, false).raw_os_error(),
            Some(libc::ENOSPC)
        );

        let remapped = true_display_output_error(io::Error::from_raw_os_error(libc::ENOSPC), true);
        assert_eq!(remapped.raw_os_error(), Some(libc::EBADF));
    }

    #[cfg(unix)]
    #[test]
    fn true_write_error_uses_gnu_errno_diagnostic() {
        let error = io::Error::from_raw_os_error(libc::ENOSPC);
        assert_eq!(
            true_write_error_message(&error),
            "write error: No space left on device"
        );
    }

    #[cfg(unix)]
    #[test]
    fn true_restores_sigpipe_only_for_a_default_initial_disposition() {
        assert!(true_should_restore_default_sigpipe(true));
        assert!(!true_should_restore_default_sigpipe(false));
    }
}
