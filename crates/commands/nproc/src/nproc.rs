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

// 用于显示与当前进程相关的可用 CPU 数目

extern crate rust_i18n;
use crate::opt_flags::OPT_ALL;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use crate::opt_flags::OPT_IGNORE;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo};
use std::ffi::OsString;
use std::fmt::Display;
use std::io::{self, Write};
use std::{env, thread};
use sys_locale::get_locale;

// 根据操作系统的不同，定义 _SC_NPROCESSORS_CONF 常量以获取系统上配置的处理器数量
#[cfg(target_os = "linux")]
pub const _SC_NUM_PROCESSORS_CONF: libc::c_int = 83;

// 定义静态字符串常量用于命令行参数解析

mod opt_flags {
    pub const OPT_ALL: &str = "all";
    pub const OPT_IGNORE: &str = "ignore";
}

#[derive(Debug)]
struct NprocInfo {
    cores_num: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NprocQuery {
    Available,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NprocSemantic {
    pub query: NprocQuery,
    pub selected: usize,
    pub available: usize,
    pub all: usize,
    pub ignore: usize,
    pub thread_limit: Option<usize>,
}

impl Display for NprocInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.cores_num)
    }
}

pub fn nproc_semantic(args: impl ctcore::Args) -> CTResult<NprocSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app().try_get_matches_from(nproc_args(args)?)?;

    let ignore = nproc_parse_ignore_num(&args_match)?;
    let query = nproc_query_from_matches(&args_match);
    let thread_limit = nproc_parse_limit_thread();
    let available = nproc_available();
    let all = nproc_all();
    let selected_base = match query {
        NprocQuery::Available => available,
        NprocQuery::All => all,
    };
    let selected = nproc_cores_num_process(
        ignore,
        nproc_effective_limit(query, thread_limit),
        selected_base,
    )?;

    Ok(NprocSemantic {
        query,
        selected,
        available,
        all,
        ignore,
        thread_limit,
    })
}

fn nproc_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    let args: Vec<OsString> = args.collect();
    validate_ignore_occurrences(&args)?;
    Ok(args)
}

fn validate_ignore_occurrences(args: &[OsString]) -> CTResult<()> {
    let posix = env::var_os("POSIXLY_CORRECT").is_some();
    let mut index = 1;
    while index < args.len() {
        let bytes = args[index].as_encoded_bytes();
        if bytes == b"--" {
            break;
        }
        if bytes.len() <= 1 || bytes[0] != b'-' {
            if posix {
                break;
            }
            index += 1;
            continue;
        }
        if bytes[1] != b'-' {
            break;
        }

        let option = &bytes[2..];
        let equals = option.iter().position(|byte| *byte == b'=');
        let name = equals.map_or(option, |position| &option[..position]);
        match canonical_long_option(name) {
            Some("ignore") => {
                let value = if let Some(position) = equals {
                    std::str::from_utf8(&option[position + 1..]).ok()
                } else {
                    index += 1;
                    args.get(index).and_then(|value| value.to_str())
                };
                if let Some(value) = value {
                    parse_ignore_value(value)?;
                }
            }
            Some("help" | "version") | None => break,
            Some("all") => {}
            Some(_) => unreachable!(),
        }
        index += 1;
    }
    Ok(())
}

fn canonical_long_option(option: &[u8]) -> Option<&'static str> {
    let mut matches = ["all", "ignore", "help", "version"]
        .into_iter()
        .filter(|candidate| {
            !option.is_empty()
                && option.len() <= candidate.len()
                && candidate.as_bytes().starts_with(option)
        });
    let canonical = matches.next()?;
    matches.next().is_none().then_some(canonical)
}

fn nproc_main(args: impl ctcore::Args) -> CTResult<NprocInfo> {
    let semantic = nproc_semantic(args)?;
    Ok(NprocInfo {
        cores_num: semantic.selected,
    })
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

fn nproc_query_from_matches(args_match: &ArgMatches) -> NprocQuery {
    if args_match.get_flag(OPT_ALL) {
        NprocQuery::All
    } else {
        NprocQuery::Available
    }
}

fn nproc_available() -> usize {
    let omp_threads = env::var_os("OMP_NUM_THREADS")
        .as_deref()
        .map(parse_omp_threads)
        .unwrap_or(0);
    if omp_threads == 0 {
        available_parallelism()
    } else {
        omp_threads
    }
}

fn nproc_parse_limit_thread() -> Option<usize> {
    let limit = env::var_os("OMP_THREAD_LIMIT")
        .as_deref()
        .map(parse_omp_threads)
        .unwrap_or(0);
    (limit != 0).then_some(limit)
}

fn parse_omp_threads(threads: &std::ffi::OsStr) -> usize {
    let bytes = threads.as_encoded_bytes();
    let mut index = 0;
    while bytes.get(index).is_some_and(|byte| is_ascii_space(*byte)) {
        index += 1;
    }
    if !bytes.get(index).is_some_and(u8::is_ascii_digit) {
        return 0;
    }

    let mut value = 0usize;
    while let Some(digit) = bytes.get(index).filter(|byte| byte.is_ascii_digit()) {
        value = value
            .saturating_mul(10)
            .saturating_add(usize::from(*digit - b'0'));
        index += 1;
    }

    while bytes.get(index).is_some_and(|byte| is_ascii_space(*byte)) {
        index += 1;
    }
    if index == bytes.len() || bytes[index] == b',' {
        value
    } else {
        0
    }
}

fn is_ascii_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn nproc_effective_limit(query: NprocQuery, thread_limit: Option<usize>) -> usize {
    match query {
        NprocQuery::Available => thread_limit.unwrap_or(usize::MAX),
        NprocQuery::All => usize::MAX,
    }
}

fn nproc_parse_ignore_num(args_match: &ArgMatches) -> CTResult<usize> {
    match args_match.get_one::<String>(OPT_IGNORE) {
        Some(num_str) => parse_ignore_value(num_str),
        None => Ok(0),
    }
}

fn parse_ignore_value(value: &str) -> CTResult<usize> {
    value
        .trim()
        .parse()
        .map_err(|_| CtSimpleError::new(1, format!("invalid number: {}", value.quote())))
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
    let application_info = t!("nproc.about");
    let usage_description = t!("nproc.usage");

    let args = vec![
        Arg::new(OPT_ALL)
            .long(OPT_ALL)
            .help(t!("nproc.clap.opt_all"))
            .action(ArgAction::SetTrue)
            .overrides_with(OPT_ALL),
        Arg::new(OPT_IGNORE)
            .long(OPT_IGNORE)
            .value_name("N")
            .help(t!("nproc.clap.opt_ignore"))
            .overrides_with(OPT_IGNORE),
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
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "windows")]
fn nproc_all() -> usize {
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

#[derive(Default)]
pub struct Nproc;
impl Tool for Nproc {
    fn name(&self) -> &'static str {
        "nproc"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let nproc_info = nproc_main(args.iter().cloned())?;
        let stdout = io::stdout();
        let mut output = stdout.lock();
        writeln!(output, "{nproc_info}").map_err_context(|| String::from("write error"))?;
        output
            .flush()
            .map_err_context(|| String::from("write error"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    mod tests_tool_implementation {
        use crate::Nproc;
        use ctcore::Tool;
        use std::ffi::OsString;

        #[test]
        fn test_tool_implementation() {
            let tool = Nproc;

            // 测试 name 方法
            assert_eq!(tool.name(), "nproc");

            // 测试 command 方法
            let command = tool.command();
            assert!(command.get_name().contains("nproc"));

            // 测试 execute 方法
            let args = vec![OsString::from("nproc")];
            assert!(tool.execute(&args).is_ok()); // nproc不需要参数
        }
    }

    mod tests_nproc_process {
        use crate::{
            NprocQuery, nproc_cores_num_process, nproc_effective_limit, parse_omp_threads,
        };
        use std::ffi::OsStr;

        #[test]
        fn test_nproc_cores_num_process_normal() {
            // 正常情况：系统有4个核心，忽略1个，限制100个
            let result = nproc_cores_num_process(1, 100, 4);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 3);
        }

        #[test]
        fn test_nproc_cores_num_process_limit() {
            // 限制生效：系统有8个核心，忽略0个，限制6个
            let result = nproc_cores_num_process(0, 6, 8);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 6);
        }

        #[test]
        fn test_nproc_cores_num_process_ignore_all() {
            // 忽略所有核心：系统有4个核心，忽略4个或更多
            let result = nproc_cores_num_process(4, 100, 4);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 1); // 返回最少1个核心

            let result = nproc_cores_num_process(5, 100, 4);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 1); // 返回最少1个核心
        }

        #[test]
        fn test_nproc_all_ignores_openmp_thread_limit() {
            assert_eq!(nproc_effective_limit(NprocQuery::Available, Some(1)), 1);
            assert_eq!(nproc_effective_limit(NprocQuery::All, Some(1)), usize::MAX);
        }

        #[test]
        fn test_parse_omp_threads_matches_gnu_rules() {
            for (input, expected) in [
                ("", 0),
                (" 2 ", 2),
                ("2,ignored", 2),
                ("2 ,ignored", 2),
                ("+2", 0),
                ("-2", 0),
                ("2bad", 0),
                ("0", 0),
                ("18446744073709551616", usize::MAX),
            ] {
                assert_eq!(parse_omp_threads(OsStr::new(input)), expected, "{input:?}");
            }
        }
    }

    mod tests_nproc_main {
        use crate::{NprocQuery, nproc_main, nproc_semantic};

        use std::ffi::OsString;

        #[test]
        fn test_nproc_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = nproc_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_nproc_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = nproc_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_nproc_main_all() {
            let args = [ctcore::ct_util_name(), "--all"];

            let result = nproc_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_nproc_main_ignore() {
            let args = [ctcore::ct_util_name(), "--ignore=1"];
            let result = nproc_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_nproc_semantic_repeated_ignore_uses_last_value() {
            for (options, expected) in [
                (["--ignore=1", "--ignore=2"], 2),
                (["--ignore=2", "--ignore=1"], 1),
            ] {
                let args = [ctcore::ct_util_name(), options[0], options[1]];
                let result = nproc_semantic(args.iter().map(OsString::from)).expect("semantic");

                assert_eq!(result.ignore, expected, "options: {options:?}");
            }
        }

        #[test]
        fn test_nproc_semantic_validates_each_repeated_ignore() {
            for options in [["--ignore=x", "--ignore=1"], ["--ignore=-1", "--ignore=1"]] {
                let args = [ctcore::ct_util_name(), options[0], options[1]];
                let result = nproc_semantic(args.iter().map(OsString::from));

                assert!(result.is_err(), "options: {options:?}");
            }
        }

        #[test]
        fn test_nproc_semantic_default_query_is_available() {
            let args = [ctcore::ct_util_name()];

            let result = nproc_semantic(args.iter().map(OsString::from)).expect("semantic");

            assert_eq!(result.query, NprocQuery::Available);
            assert_eq!(result.ignore, 0);
            assert!(result.selected >= 1);
            assert!(result.available >= 1);
            assert!(result.all >= 1);
        }

        #[test]
        fn test_nproc_semantic_all_query_tracks_flag() {
            let args = [ctcore::ct_util_name(), "--all"];

            let result = nproc_semantic(args.iter().map(OsString::from)).expect("semantic");

            assert_eq!(result.query, NprocQuery::All);
            assert!(result.selected >= 1);
            assert!(result.all >= 1);
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
        fn test_ct_app_accepts_repeated_all() {
            let args = vec![ctcore::ct_util_name(), "--all", "--all"];
            let result = ct_app().try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.expect("matches").get_flag("all"));
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
