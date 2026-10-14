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
#[cfg(all(target_os = "linux", not(test)))]
use ctcore::ct_error::CtSimpleError;
#[cfg(all(target_os = "linux", not(test)))]
use ctcore::ct_signals::enable_pipe_errors;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use sys_locale::get_locale;

const SYSKITS_FALSE_LONG_VERSION: &str = concat!(
    "(syskits) ",
    env!("CARGO_PKG_VERSION"),
    "\nCopyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.\n",
    "License Mulan PSL v2: <http://license.coscl.org.cn/MulanPSL2>.\n",
    "This is free software: you are free to change and redistribute it.\n",
    "There is NO WARRANTY, to the extent permitted by law."
);

const SYSKITS_FALSE_AFTER_HELP: &str =
    "syskits online help: <https://www.ctyun.cn/developer>";

#[cfg(all(target_os = "linux", not(test)))]
static INHERITED_SIGPIPE_IGNORED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(all(target_os = "linux", not(test)))]
extern "C" fn record_inherited_sigpipe_disposition() {
    unsafe {
        let mut action: ctcore::libc::sigaction = std::mem::zeroed();
        if ctcore::libc::sigaction(ctcore::libc::SIGPIPE, std::ptr::null(), &mut action) == 0
            && action.sa_sigaction == ctcore::libc::SIG_IGN
        {
            INHERITED_SIGPIPE_IGNORED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[cfg(all(target_os = "linux", not(test)))]
#[used]
#[unsafe(link_section = ".preinit_array")]
static RECORD_INHERITED_SIGPIPE_DISPOSITION: extern "C" fn() = record_inherited_sigpipe_disposition;

#[cfg(all(target_os = "linux", not(test)))]
fn inherited_sigpipe_is_ignored() -> bool {
    INHERITED_SIGPIPE_IGNORED.load(std::sync::atomic::Ordering::Relaxed)
}

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

pub fn false_main(args: impl ctcore::Args) -> CTResult<()> {
    #[cfg(all(target_os = "linux", not(test)))]
    if !inherited_sigpipe_is_ignored() {
        enable_pipe_errors().map_err(|error| CtSimpleError::new(1, error.to_string()))?;
    }

    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
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
    let program_name = args
        .first()
        .cloned()
        .unwrap_or_else(|| OsString::from(ctcore::ct_util_name()));

    if let Err(e) = command.try_get_matches_from_mut(args) {
        let error = match e.kind() {
            // 如果是显示帮助信息的错误，则显示帮助信息。
            clap::error::ErrorKind::DisplayHelp => command.print_help(),
            // 如果是显示版本信息的错误，则显示版本信息。
            clap::error::ErrorKind::DisplayVersion => {
                writeln!(std::io::stdout(), "{}", command.render_long_version())
            }
            // 对于其他类型的错误，不进行处理。
            _ => Ok(()),
        };

        // Help and version still return false's normal exit status after write failures.
        if let Err(print_fail) = error {
            report_false_write_error(&program_name, print_fail);
        }
    }

    Ok(())
}

fn report_false_write_error(program_name: &OsStr, error: io::Error) {
    #[cfg(target_os = "linux")]
    let error = if ctcore::ct_stdout_was_closed() {
        io::Error::from_raw_os_error(ctcore::libc::EBADF)
    } else {
        error
    };

    let detail = strip_errno(&error);
    #[cfg(unix)]
    let program_name = program_name.as_bytes();
    #[cfg(not(unix))]
    let program_name = program_name.to_string_lossy().as_bytes();

    let stderr = io::stderr();
    let mut stderr = stderr.lock();
    let _ = stderr
        .write_all(program_name)
        .and_then(|_| stderr.write_all(b": write error: "))
        .and_then(|_| stderr.write_all(detail.as_bytes()))
        .and_then(|_| stderr.write_all(b"\n"));
}

/// 创建并配置命令行解析器实例。
pub fn ct_app() -> Command {
    Command::new(ctcore::ct_util_name())
        .version(clap::crate_version!())
        .long_version(SYSKITS_FALSE_LONG_VERSION)
        .about(t!("false.about"))
        .after_help(SYSKITS_FALSE_AFTER_HELP)
        // Keep only the exact long forms as terminal options.
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .long("help")
                .help(t!("false.clap.help"))
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .help(t!("false.clap.version"))
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
            let args = [ctcore::ct_util_name(), "--version"];

            let result = false_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_false_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = false_main(args.iter().map(OsString::from));

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
