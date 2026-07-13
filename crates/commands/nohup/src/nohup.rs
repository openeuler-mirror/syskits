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

// nohup命令的作用是在Unix/Linux系统中允许一个命令在用户退出终端后继续在后台运行

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, UClapError, set_ct_exit_code};

use libc::{SIG_IGN, SIGHUP};
use libc::{c_char, dup2, execvp, signal};

use ctcore::Tool;
use std::env;
use std::ffi::CString;
use std::ffi::OsString;
use std::fmt::{Display, Formatter};
use std::fs::{File, OpenOptions};
use std::io::{self, Error, IsTerminal, Write, stderr};
use std::os::unix::prelude::*;
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

// 定义常量和模块，用于处理nohup命令的逻辑。
static NOHUP_OUT: &str = "nohup.out"; // 默认的nohup输出文件名

use crate::exit_codes::EXIT_CANCELED;
use crate::exit_codes::EXIT_CANNOT_INVOKE;
use crate::exit_codes::EXIT_ENOENT;
use crate::exit_codes::POSIX_NOHUP_FAILURE;
// 与GNU实现相匹配的退出码
mod exit_codes {
    pub static EXIT_CANCELED: i32 = 125;
    pub static EXIT_CANNOT_INVOKE: i32 = 126;
    pub static EXIT_ENOENT: i32 = 127;
    pub static POSIX_NOHUP_FAILURE: i32 = 127;
}

mod options {
    pub const CMD: &str = "cmd"; // 命令参数的标识符
}

// 定义NohupError枚举，处理可能出现的错误类型
#[derive(Debug)]
enum NohupError {
    CannotDetach,                           // 无法从控制台分离
    CannotReplace(&'static str, Error),     // 无法替换指定的文件描述符
    OpenFailed(i32, Error),                 // 打开文件失败
    OpenFailed2(i32, Error, String, Error), // 打开文件失败（备选路径）
}

impl std::error::Error for NohupError {}

impl CTError for NohupError {
    fn code(&self) -> i32 {
        match self {
            Self::OpenFailed(code, _) | Self::OpenFailed2(code, _, _, _) => *code,
            _ => 2,
        }
    }
}

impl Display for NohupError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            Self::CannotDetach => write!(f, "Cannot detach from console"),
            Self::CannotReplace(s, e) => write!(f, "Cannot replace {s}: {e}"),
            Self::OpenFailed(_, e) => {
                write!(f, "failed to open {}: {}", NOHUP_OUT.quote(), e)
            }
            Self::OpenFailed2(_, e1, s, e2) => write!(
                f,
                "failed to open {}: {}\nfailed to open {}: {}",
                NOHUP_OUT.quote(),
                e1,
                s.quote(),
                e2
            ),
        }
    }
}

fn write_nohup_msg(msg: &str) -> io::Result<()> {
    let mut handle = stderr();
    writeln!(handle, "nohup: {msg}")?;
    handle.flush()
}

fn nohup_append_msg(path: &str) -> String {
    format!("ignoring input and appending output to {}", path.quote())
}

pub fn nohup_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_error_code = if env::var("POSIXLY_CORRECT").is_ok() {
        EXIT_ENOENT
    } else {
        EXIT_CANCELED
    };

    let args_match = ct_app()
        .try_get_matches_from(args)
        .with_exit_code(arg_error_code)?;

    nohup_replace_fds()?;

    unsafe { signal(SIGHUP, SIG_IGN) }; // 忽略SIGHUP信号

    if unsafe { !_vprocmgr_detach_from_console(0).is_null() } {
        return Err(NohupError::CannotDetach.into());
    };

    let cstrings: Vec<CString> = args_match
        .get_many::<String>(options::CMD)
        .unwrap()
        .map(|x| CString::new(x.as_bytes()).unwrap())
        .collect();
    let mut args: Vec<*const c_char> = cstrings.iter().map(|s| s.as_ptr()).collect();
    args.push(std::ptr::null());

    let result = unsafe { execvp(args[0], args.as_mut_ptr()) };
    if result == -1 {
        let err = std::io::Error::last_os_error();
        // 获取命令名用于错误信息
        let cmd_name = std::str::from_utf8(cstrings[0].to_bytes())
            .unwrap_or("<unknown>")
            .to_string();
        let err_msg = format!("cannot run command '{cmd_name}': {err}");
        // 尝试输出错误，如果 stderr 写入失败则退出 125
        if write_nohup_msg(&err_msg).is_err() {
            std::process::exit(125);
        }
        match err.raw_os_error() {
            Some(libc::ENOENT) => set_ct_exit_code(EXIT_ENOENT),
            _ => set_ct_exit_code(EXIT_CANNOT_INVOKE),
        }
    }
    Ok(())
}

// 构建命令行解析器
pub fn ct_app() -> Command {
    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("nohup.about"))
        .after_help(t!("nohup.after_help"))
        .override_usage(t!("nohup.usage"))
        .arg(
            Arg::new(options::CMD)
                .hide(true)
                .required(true)
                .action(ArgAction::Append)
                .value_hint(clap::ValueHint::CommandName),
        )
        .trailing_var_arg(true)
        .infer_long_args(true)
}

// 替换标准输入、输出和错误输出文件描述符
fn nohup_replace_fds() -> CTResult<()> {
    let stdin_is_tty = std::io::stdin().is_terminal();
    let stdout_is_tty = std::io::stdout().is_terminal();

    if stdin_is_tty {
        let new_stdin = File::open(Path::new("/dev/null"))
            .map_err(|e| NohupError::CannotReplace("STDIN", e))?;
        if unsafe { dup2(new_stdin.as_raw_fd(), 0) } != 0 {
            return Err(NohupError::CannotReplace("STDIN", Error::last_os_error()).into());
        }

        if !stdout_is_tty && write_nohup_msg("ignoring input").is_err() {
            std::process::exit(125);
        }
    }

    if stdout_is_tty {
        let new_stdout = nohup_find_stdout()?;
        let raw_fd = new_stdout.as_raw_fd();
        if unsafe { dup2(raw_fd, 1) } != 1 {
            return Err(NohupError::CannotReplace("STDOUT", Error::last_os_error()).into());
        }
    }

    if std::io::stderr().is_terminal() && unsafe { dup2(1, 2) } != 2 {
        return Err(NohupError::CannotReplace("STDERR", Error::last_os_error()).into());
    }
    Ok(())
}

// 查找或创建nohup输出文件
fn nohup_find_stdout() -> CTResult<File> {
    let internal_failure_code = match env::var("POSIXLY_CORRECT") {
        Ok(_) => POSIX_NOHUP_FAILURE,
        Err(_) => EXIT_CANCELED,
    };

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(Path::new(NOHUP_OUT))
    {
        Ok(file) => {
            let msg = nohup_append_msg(NOHUP_OUT);
            if write_nohup_msg(&msg).is_err() {
                std::process::exit(125);
            }
            Ok(file)
        }
        Err(err1) => {
            let home = match env::var("HOME") {
                Err(_) => return Err(NohupError::OpenFailed(internal_failure_code, err1).into()),
                Ok(h) => h,
            };
            let mut path_buf = PathBuf::from(home);
            path_buf.push(NOHUP_OUT);
            let path_buf_str = path_buf.to_str().unwrap();
            match OpenOptions::new().create(true).append(true).open(&path_buf) {
                Ok(file) => {
                    let msg = nohup_append_msg(path_buf_str);
                    if write_nohup_msg(&msg).is_err() {
                        std::process::exit(125);
                    }
                    Ok(file)
                }
                Err(err2) => Err(NohupError::OpenFailed2(
                    internal_failure_code,
                    err1,
                    path_buf_str.to_string(),
                    err2,
                )
                .into()),
            }
        }
    }
}

#[cfg(target_os = "linux")]
unsafe fn _vprocmgr_detach_from_console(_: u32) -> *const libc::c_int {
    std::ptr::null()
}

#[derive(Default)]
pub struct Nohup;
impl Tool for Nohup {
    fn name(&self) -> &'static str {
        "nohup"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        nohup_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    mod tests_messages {
        use crate::nohup_append_msg;

        #[test]
        fn test_nohup_append_msg_uses_actual_path() {
            assert_eq!(
                nohup_append_msg("/tmp/home/nohup.out"),
                "ignoring input and appending output to '/tmp/home/nohup.out'"
            );
        }
    }

    mod tests_tool_implementation {
        use crate::Nohup;
        use ctcore::Tool;
        use std::ffi::OsString;

        #[test]
        fn test_tool_implementation() {
            let tool = Nohup;

            // 测试 name 方法
            assert_eq!(tool.name(), "nohup");

            // 测试 command 方法
            let command = tool.command();
            assert!(command.get_name().contains("nohup"));

            // 测试 execute 方法
            let args = vec![OsString::from("nohup"), OsString::from("--help")];
            assert!(tool.execute(&args).is_err()); // --help参数通常会返回错误
        }
    }

    mod tests_echo_main {
        use crate::nohup_main;

        use std::ffi::OsString;

        #[test]
        fn test_false_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = nohup_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_false_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = nohup_main(args.iter().map(OsString::from));

            assert!(result.is_err());
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
