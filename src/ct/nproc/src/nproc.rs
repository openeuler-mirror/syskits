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

// 用于显示与当前进程相关的可用 CPU 数目

use crate::opt_flags::OPT_ALL;

use crate::opt_flags::OPT_IGNORE;
use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
use std::fmt::Display;
use std::{env, thread};

// 根据操作系统的不同，定义 _SC_NPROCESSORS_CONF 常量以获取系统上配置的处理器数量
#[cfg(any(target_os = "linux", target_os = "android"))]
pub const _SC_NUM_PROCESSORS_CONF: libc::c_int = 83;
#[cfg(target_vendor = "apple")]
pub const _SC_NUM_PROCESSORS_CONF: libc::c_int = libc::_SC_NPROCESSORS_CONF;
#[cfg(target_os = "freebsd")]
pub const _SC_NUM_PROCESSORS_CONF: libc::c_int = 57;
#[cfg(target_os = "netbsd")]
pub const _SC_NUM_PROCESSORS_CONF: libc::c_int = 1001;

// 定义静态字符串常量用于命令行参数解析

mod opt_flags {
    pub const OPT_ALL: &str = "all";
    pub const OPT_IGNORE: &str = "ignore";
}

// 从文档 "nproc.md" 中提取的关于信息和用法信息
const NPROC_ABOUT: &str = ct_help_about!("nproc.md");
const NPROC_USAGE: &str = ct_help_usage!("nproc.md");

#[derive(Debug)]
struct NprocInfo {
    cores_num: usize,
}

impl Display for NprocInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.cores_num)
    }
}

/**
 * 程序的主入口函数。
 *
 * 参数:
 *  - args: 实现了 ctcore::Args 接口的参数对象，代表命令行传入的参数。
 *
 * 返回值:
 *  - CTResult<()>: 表示操作成功或失败的结果。成功时返回 ()，失败时返回错误信息。
 */
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let result = nproc_main(args);
    match result {
        Ok(nproc_info) => {
            println!("{}", nproc_info);

            Ok(())
        }
        _ => {
            // 如果出现错误，则打印错误信息并返回错误
            eprint!("{}", result.err().unwrap());
            Err(125.into())
        }
    }
}

fn nproc_main(args: impl ctcore::Args) -> CTResult<NprocInfo> {
    let args_match = ct_app().try_get_matches_from(args)?;

    // 解析 --ignore 参数，决定忽略多少核心
    let ignore_num = match nproc_parse_ignore_num(&args_match) {
        Ok(value) => value,
        Err(_) => {
            return Err(CtSimpleError::new(
                1,
                "Failed to get the ignore num".to_string(),
            ))
        }
    };

    // 解析环境变量 OMP_THREAD_LIMIT 以限制线程数量
    let limit_thread = nproc_parse_limit_thread();

    // 根据命令行参数确定要计算的核心数量
    let cores_num = nproc_parse_cores_num(args_match);

    // 应用限制和忽略的核心数量
    let cores_num = nproc_cores_num_process(ignore_num, limit_thread, cores_num);

    match cores_num {
        Ok(cores_num) => {
            let nproc_info = NprocInfo { cores_num };
            Ok(nproc_info)
        }
        _ => Err(CtSimpleError::new(
            1,
            "Failed to get the number of cores".to_string(),
        )),
    }
}

fn nproc_cores_num_process(
    ignore_num: usize,
    limit_thread: usize,
    mut cores_num: usize,
) -> Result<usize, Box<dyn CTError>> {
    cores_num = std::cmp::min(limit_thread, cores_num);
    if cores_num <= ignore_num {
        cores_num = 1;
    } else {
        cores_num -= ignore_num;
    }

    Ok(cores_num)
}

fn nproc_parse_cores_num(args_match: ArgMatches) -> usize {
    let cores_num = if args_match.get_flag(OPT_ALL) {
        nproc_all()
    } else {
        // 尝试使用环境变量 OMP_NUM_THREADS 强制设置线程数
        match env::var("OMP_NUM_THREADS") {
            // 解析并处理 OMP_NUM_THREADS，特殊处理 "x,y,z" 格式的情况
            Ok(thread_str) => {
                let thread: Vec<&str> = thread_str.split_terminator(',').collect();
                match &thread[..] {
                    [] => available_parallelism(),
                    [s, ..] => match s.parse() {
                        Ok(0) | Err(_) => available_parallelism(),
                        Ok(n) => n,
                    },
                }
            }
            // OMP_NUM_THREADS 环境变量不存在，退回到默认的核心检测
            Err(_) => available_parallelism(),
        }
    };
    cores_num
}

fn nproc_parse_limit_thread() -> usize {
    match env::var("OMP_THREAD_LIMIT") {
        // 使用 OpenMP 变量限制线程数；解析失败时取最大值，OMP_THREAD_LIMIT=0 时也取最大值
        Ok(thread_str) => match thread_str.parse() {
            Ok(0) | Err(_) => usize::MAX,
            Ok(n) => n,
        },
        // OMP_THREAD_LIMIT 环境变量不存在，取最大值
        Err(_) => usize::MAX,
    }
}

fn nproc_parse_ignore_num(args_match: &ArgMatches) -> Result<usize, CTResult<()>> {
    let ignore_num = match args_match.get_one::<String>(OPT_IGNORE) {
        Some(num_str) => match num_str.trim().parse() {
            Ok(num) => num,
            Err(e) => {
                return Err(Err(CtSimpleError::new(
                    1,
                    format!("{} is not a valid number: {}", num_str.quote(), e),
                )));
            }
        },
        None => 0,
    };
    Ok(ignore_num)
}

/**
 * 构建命令行解析器。
 *
 * 返回值:
 *  - Command: 配置好的命令行解析器对象。
 */
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = NPROC_ABOUT;
    let usage_description = ct_format_usage(NPROC_USAGE);

    let args = vec![
        Arg::new(OPT_ALL)
            .long(OPT_ALL)
            .help("print the number of cores available to the system")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_IGNORE)
            .long(OPT_IGNORE)
            .value_name("N")
            .help("ignore up to N cores"),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

/**
 * 获取系统上所有可用的核心数量。
 *
 * 根据不同的操作系统，使用不同的方法获取核心数。
 *
 * 返回值:
 *  - usize: 系统上的核心数量。
 */
#[cfg(any(
    target_os = "linux",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "netbsd"
))]
fn nproc_all() -> usize {
    let nprocs_num = unsafe { libc::sysconf(_SC_NUM_PROCESSORS_CONF) };
    if nprocs_num == 1 {
        // 在某些情况下，/proc 和 /sys 未被挂载，sysconf 返回 1。但我们希望 `nproc --all` >= `nproc`。
        available_parallelism()
    } else if nprocs_num > 0 {
        nprocs_num as usize
    } else {
        1
    }
}

// 在其他平台上，直接调用 available_parallelism()
#[cfg(not(any(
    target_os = "linux",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "netbsd"
)))]
fn num_cpus_all() -> usize {
    available_parallelism()
}

/**
 * 获取系统当前可用的并行线程数。
 *
 * 如果 thread::available_parallelism() 返回错误，则默认返回 1。
 *
 * 返回值:
 *  - usize: 系统上可用的并行线程数。
 */
fn available_parallelism() -> usize {
    match thread::available_parallelism() {
        Ok(n) => n.get(),
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {

    mod tests_nproc_main {
        use crate::nproc_main;

        use std::ffi::OsString;

        #[test]
        fn test_nproc_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = nproc_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_nproc_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = nproc_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_nproc_main_all() {
            let args = vec![ctcore::ct_util_name(), "--all"];

            let result = nproc_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_nproc_main_ignore() {
            let args = vec![ctcore::ct_util_name(), "--ignore=1"];
            let result = nproc_main(args.iter().map(|s| OsString::from(s)));

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
        #[test]
        fn test_cp_all_all() {
            let args = vec![ctcore::ct_util_name(), "--all"];

            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_all_ignore() {
            let args = vec![ctcore::ct_util_name(), "--ignore=1"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
    }
}
