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

// mkfifo 是一个 Linux 和类 Unix 系统中的命令，它用于创建一个命名管道（named pipe），也称为FIFO（First In First Out）。
// 命名管道是一种特殊类型的文件，允许不同的进程之间通过它进行数据通信，而无需事先知道对方的进程ID。
// 与无名管道（匿名管道）不同，命名管道可以在进程之间持久存在，即使创建它的进程已经结束。
//
// 命名管道的使用通常涉及到两个或多个进程，其中一个进程将数据写入管道，另一个或多个进程从管道中读取数据。它们在文件系统中有一个名称，因此可以被多个进程引用。

use clap::{crate_version, Arg, ArgAction, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show};
use libc::mkfifo;
use std::ffi::CString;

// 定义了用于创建FIFO（命名管道）的命令行工具的主逻辑。

static MKFIFO_USAGE: &str = ct_help_usage!("mkfifo.md"); // 命令使用说明
static MKFIFO_ABOUT: &str = ct_help_about!("mkfifo.md"); // 命令简介

// 用于命令行选项的常量模块
mod opt_flags {
    pub const MODE: &str = "mode"; // 文件权限模式选项
    pub const SE_LINUX_SECURITY_CONTEXT: &str = "Z"; // 设置SELinux安全上下文选项
    pub const CONTEXT: &str = "context"; // 安全上下文选项
    pub const FIFO: &str = "fifo"; // FIFO路径参数
}

// 主函数，负责解析命令行参数并执行创建FIFO的操作。
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    mkfifo_main(args).map(|_| ())
}

pub fn mkfifo_main(args: impl ctcore::Args) -> CTResult<()> {
    let args_match = ct_app().try_get_matches_from(args)?;

    // 检查不支持的选项
    if args_match.contains_id(opt_flags::CONTEXT) {
        return Err(CtSimpleError::new(1, "--context is not implemented"));
    }
    if args_match.get_flag(opt_flags::SE_LINUX_SECURITY_CONTEXT) {
        return Err(CtSimpleError::new(1, "-Z is not implemented"));
    }

    // 解析文件权限模式
    let fifo_mode = match args_match.get_one::<String>(opt_flags::MODE) {
        Some(m) => match usize::from_str_radix(m, 8) {
            Ok(m) => m,
            Err(e) => return Err(CtSimpleError::new(1, format!("invalid mode: {e}"))),
        },
        None => 0o666,
    };

    // 解析FIFO路径列表
    let fifo_strs: Vec<String> = match args_match.get_many::<String>(opt_flags::FIFO) {
        Some(v) => v.cloned().collect(),
        None => return Err(CtSimpleError::new(1, "missing operand")),
    };

    // 创建FIFO
    for fifo in fifo_strs {
        let e = unsafe {
            let fifo_name = CString::new(fifo.as_bytes()).unwrap();
            mkfifo(fifo_name.as_ptr(), fifo_mode as libc::mode_t)
        };
        if e == -1 {
            ct_show!(CtSimpleError::new(
                1,
                format!("cannot create fifo {}: File exists", fifo.quote())
            ));
        }
    }

    Ok(())
}

// 构建命令行解析器
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = MKFIFO_ABOUT;
    let usage_description = ct_format_usage(MKFIFO_USAGE);
    let args = vec![
        Arg::new(opt_flags::MODE)
            .short('m')
            .long(opt_flags::MODE)
            .help("file permissions for the fifo")
            .default_value("0666")
            .value_name("MODE"),
        Arg::new(opt_flags::SE_LINUX_SECURITY_CONTEXT)
            .short('Z')
            .help("set the SELinux security context to default type")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CONTEXT)
            .long(opt_flags::CONTEXT)
            .value_name("CTX")
            .help(
                "like -Z, or if CTX is specified then set the SELinux \
                    or SMACK security context to CTX",
            ),
        Arg::new(opt_flags::FIFO)
            .hide(true) // 隐藏此参数
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath), // 提示参数类型为路径
    ];

    Command::new(utility_name)
        .version(command_version)
        .override_usage(usage_description)
        .about(application_info)
        .infer_long_args(true)
        .args(&args)
}

#[cfg(test)]
mod tests {

    mod tests_mkfio_main {
        use crate::mkfifo_main;

        use std::ffi::OsString;

        #[test]
        fn test_false_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = mkfifo_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_false_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = mkfifo_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }
    }

    mod tests_mkfio_app {
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