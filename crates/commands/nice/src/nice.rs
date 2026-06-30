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

// 在GNU/Linux系统中，nice命令的主要作用是调整程序的执行优先级，从而影响其对CPU资源的访问

extern crate rust_i18n;
use libc::{c_char, c_int, execvp, nice as libc_nice};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::ffi::{CString, OsString};
use std::io::{Error, Write};
use std::ptr;

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::{
    Tool,
    ct_display::Quotable,
    ct_error::{CTResult, CtSimpleError, UClapError, set_ct_exit_code},
    ct_show_error,
};
use sys_locale::get_locale;

pub mod opt_flags {
    pub static ADJUSTMENT: &str = "adjustment";
    pub static COMMAND: &str = "COMMAND";
}

fn is_prefix_of(prefix: &str, target: &str, min_match: usize) -> bool {
    if prefix.len() < min_match || prefix.len() > target.len() {
        return false;
    }

    &target[0..prefix.len()] == prefix
}

const NZERO: i32 = 20;
const MIN_ADJUSTMENT: i64 = 1 - 2 * NZERO as i64;
const MAX_ADJUSTMENT: i64 = 2 * NZERO as i64 - 1;

fn clamp_adjustment(value: i64) -> i32 {
    value.clamp(MIN_ADJUSTMENT, MAX_ADJUSTMENT) as i32
}

fn parse_adjustment(value: &str) -> Result<i32, Box<dyn ctcore::ct_error::CTError>> {
    let parsed: i64 = value
        .parse()
        .map_err(|_| CtSimpleError::new(125, format!("invalid adjustment {}", value.quote())))?;
    Ok(clamp_adjustment(parsed))
}

fn perm_related_errno(err: i32) -> bool {
    err == libc::EACCES || err == libc::EPERM
}

fn get_current_niceness() -> Result<c_int, Error> {
    nix::errno::Errno::clear();
    let value = unsafe { libc_nice(0) };
    let err = Error::last_os_error();
    if value == -1 && err.raw_os_error().unwrap_or(0) != 0 {
        return Err(err);
    }
    Ok(value)
}

fn apply_adjustment(adjustment: c_int) -> Result<(), CTResult<()>> {
    nix::errno::Errno::clear();
    let value = unsafe { libc_nice(adjustment) };
    let err = Error::last_os_error();
    if value == -1 && err.raw_os_error().unwrap_or(0) != 0 {
        let errno = err.raw_os_error().unwrap_or(0);
        if perm_related_errno(errno) {
            if writeln!(
                std::io::stderr(),
                "{}: cannot set niceness: {}",
                ctcore::ct_util_name(),
                err
            )
            .is_err()
            {
                set_ct_exit_code(125);
                return Err(Ok(()));
            }
            return Ok(());
        }
        return Err(Err(CtSimpleError::new(
            125,
            format!("cannot set niceness: {err}"),
        )));
    }
    Ok(())
}

/// 将传统的参数转换为标准化形式。
///
/// 下面是GNU nice命令合法的参数序列：
/// - "-1"
/// - "-n1"
/// - "-+1"
/// - "--1"
/// - "-n -1"
///
/// 最初看起来，我们可以在处理"-{i}"、"--{i}"和"-+{i}"形式的整数{i}时，
/// 使用clap进行正常处理。然而，"-1"等参数的意义取决于其在传统参数解析中的上下文。
/// clap会将连字符值优先匹配为已知参数，而不是将其解释为前一个参数的值。因此，在这种情况下，
/// "-n" "-1"会被解释为两个参数，而不是一个带有值的参数。
///
/// 由于这种上下文依赖性，以及在这种情况下使用clap所带来的深层次问题，
/// 最简单的方法是在clap开始工作之前，将nice的参数标准化。在这里，
/// 我们将所有形式为"-{i}"、"--{i}"和"-+{i}"的参数（如果不已经被"-n"预置）前面插入"-n"前缀。
fn standardize_nice_args(mut args: impl ctcore::Args) -> impl ctcore::Args {
    let mut vec = Vec::<OsString>::new(); // 存储标准化后的参数
    let mut is_saw_n = false; // 标记是否已看到"-n"参数
    let mut is_saw_command = false; // 标记是否已看到命令参数

    // 处理第一个参数，通常是命令名
    if let Some(cmd) = args.next() {
        vec.push(cmd);
    }

    // 遍历剩余的参数
    for str in args {
        if is_saw_command {
            vec.push(str);
        } else if str.to_str() == Some("--") {
            // "--" 结束选项解析，后续均视为命令参数
            vec.push(str);
            is_saw_command = true;
        } else if is_saw_n {
            // 如果已看到"-n"，则将当前参数与"-n"合并
            let mut new_arg: OsString = "-n".into();
            new_arg.push(str);
            vec.push(new_arg);
            is_saw_n = false; // 重置"-n"标记
        } else if str.to_str() == Some("-n")
            || str
                .to_str()
                .map(|s| is_prefix_of(s, "--adjustment", "--a".len()))
                .unwrap_or_default()
        {
            // 处理"-n"和"--adjustment"参数
            is_saw_n = true;
        } else if let Ok(s) = str.clone().into_string() {
            // 尝试将参数解析为可能的数值调整
            if let Some(stripped) = s.strip_prefix('-') {
                match stripped.parse::<i64>() {
                    Ok(ix) => {
                        // 如果是数值，添加"-n"前缀并添加到结果中
                        let mut new_arg: OsString = "-n".into();
                        new_arg.push(ix.to_string());
                        vec.push(new_arg);
                    }
                    Err(_) => {
                        // 如果不能解析为数值，将其作为普通参数添加
                        vec.push(s.into());
                    }
                }
            } else {
                // 如果参数不以连字符开头，视为命令并添加
                is_saw_command = true;
                vec.push(s.into());
            }
        } else {
            // 添加非字符串可转换的参数作为命令
            is_saw_command = true;
            vec.push(str);
        }
    }

    // 如果最后未看到"-n"，则添加
    if is_saw_n {
        vec.push("-n".into());
    }

    vec.into_iter() // 返回标准化后的参数迭代器
}

/**
 * 主要功能：根据提供的参数调整当前进程的优先级，并执行指定的命令。
 *
 * 参数：
 * args - 实现了 `ctcore::Args` 接口的对象，代表命令行参数。
 *
 * 返回值：
 * `CTResult<()>` - 成功时返回 `Ok(())`，错误时返回包含错误信息的 `Err`。
 *
 * 注意：
 * 此函数会根据提供的参数调整当前进程的优先级，如果调整失败或无法执行指定的命令，
 * 会返回相应的错误代码和错误信息。
 */
pub fn nice_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 标准化命令行参数
    let args = standardize_nice_args(args);

    // 使用 clap 库解析命令行参数
    let args_match = ct_app().try_get_matches_from(args).with_exit_code(125)?;

    let has_command = args_match.contains_id(opt_flags::COMMAND);
    let adjustment_value = args_match.get_one::<String>(opt_flags::ADJUSTMENT);

    if !has_command {
        if let Some(value) = adjustment_value {
            let _ = parse_adjustment(value)?;
            ct_show_error!("a command must be given with an adjustment");
            eprintln!("Try 'nice --help' for more information.");
            set_ct_exit_code(125);
            return Ok(());
        }

        match get_current_niceness() {
            Ok(value) => {
                println!("{value}");
                return Ok(());
            }
            Err(err) => {
                ct_show_error!("cannot get niceness: {}", err);
                set_ct_exit_code(125);
                return Ok(());
            }
        }
    }

    let adjustment = match adjustment_value {
        Some(value) => parse_adjustment(value)?,
        None => 10,
    };

    if let Err(value) = apply_adjustment(adjustment) {
        return value;
    }

    // 准备执行命令需要的参数，并执行命令
    let command_args: Vec<String> = args_match
        .get_many::<String>(opt_flags::COMMAND)
        .unwrap()
        .map(|x| x.to_string())
        .collect();
    let cstr: Vec<CString> = command_args
        .iter()
        .map(|x| CString::new(x.as_bytes()).unwrap())
        .collect();

    let mut args: Vec<*const c_char> = cstr.iter().map(|s| s.as_ptr()).collect();
    args.push(ptr::null::<c_char>());
    unsafe {
        execvp(args[0], args.as_mut_ptr());
    }

    // 执行命令失败的处理
    let exec_error = Error::last_os_error();
    ct_show_error!("{}: {}", command_args[0].quote(), exec_error);
    let exit_code = if exec_error.raw_os_error().unwrap_or(0) as c_int == libc::ENOENT {
        127
    } else {
        126
    };
    set_ct_exit_code(exit_code);
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("nice.about");
    let usage_description = t!("nice.usage");

    let args = vec![
        Arg::new(opt_flags::ADJUSTMENT)
            .short('n')
            .long(opt_flags::ADJUSTMENT)
            .help(t!("nice.clap.adjustment"))
            .action(ArgAction::Set)
            .overrides_with(opt_flags::ADJUSTMENT)
            .allow_hyphen_values(true),
        Arg::new(opt_flags::COMMAND)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::CommandName),
    ];

    Command::new(utility_name)
        .about(application_info)
        .override_usage(usage_description)
        .trailing_var_arg(true)
        .infer_long_args(true)
        .version(command_version)
        .args(&args)
}

#[derive(Default)]
pub struct Nice;
impl Tool for Nice {
    fn name(&self) -> &'static str {
        "nice"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        nice_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Nice;

        // 测试 name 方法
        assert_eq!(tool.name(), "nice");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("nice"));

        // 测试 execute 方法
        let args = vec![OsString::from("nice"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    #[test]
    fn test_is_prefix_of() {
        // 测试前缀判断函数
        assert!(is_prefix_of("--a", "--adjustment", "--a".len()));
        assert!(is_prefix_of("--adj", "--adjustment", "--a".len()));
        assert!(is_prefix_of("--adjustment", "--adjustment", "--a".len()));

        // 不满足最小匹配长度
        assert!(!is_prefix_of("--", "--adjustment", "--a".len()));

        // 前缀长度超过目标字符串
        assert!(!is_prefix_of("--adjustmentx", "--adjustment", "--a".len()));

        // 非前缀字符串
        assert!(!is_prefix_of("--other", "--adjustment", "--a".len()));
    }

    #[test]
    fn test_standardize_nice_args() {
        // 测试基本场景
        let args = vec!["nice", "-5", "command", "arg1"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-n5"),
                OsString::from("command"),
                OsString::from("arg1"),
            ]
        );

        // 测试 -n 参数
        let args = vec!["nice", "-n", "10", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-n10"),
                OsString::from("command"),
            ]
        );

        // 测试 --adjustment 参数
        let args = vec!["nice", "--adjustment", "15", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-n15"),
                OsString::from("command"),
            ]
        );

        // 测试没有指定数值的 -n 参数
        let args = vec!["nice", "-n"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(result, vec![OsString::from("nice"), OsString::from("-n"),]);

        // 测试非数值类型的参数
        let args = vec!["nice", "-invalid", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-invalid"),
                OsString::from("command"),
            ]
        );

        // 测试 "--" 结束选项解析
        let args = vec!["nice", "--", "-1", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("--"),
                OsString::from("-1"),
                OsString::from("command"),
            ]
        );
    }

    #[test]
    fn test_parse_adjustment() {
        assert_eq!(parse_adjustment("5").unwrap(), 5);
        assert_eq!(parse_adjustment("+1").unwrap(), 1);
        assert_eq!(parse_adjustment("-1").unwrap(), -1);
        assert_eq!(parse_adjustment("100").unwrap(), MAX_ADJUSTMENT as i32);
        assert_eq!(parse_adjustment("-100").unwrap(), MIN_ADJUSTMENT as i32);
        assert!(parse_adjustment("invalid").is_err());
    }

    // 以下测试需要使用 mock 或集成测试才能完全测试，
    // 因为它们涉及系统调用和进程优先级的修改
    #[test]
    fn test_nice_getprority_mock() {
        // 这个测试主要测试函数的结构性，
        // 真正的系统调用测试需要在集成测试中完成
        let result = get_current_niceness();

        // 在大多数系统上，应该能成功获取当前进程的优先级
        if result.is_ok() {
            let nice_ness = result.unwrap();
            // 优先级通常在-20到19之间
            assert!((-20..=19).contains(&nice_ness));
        }
    }

    mod tests_nice_main {
        use crate::nice_main;

        use std::ffi::OsString;

        #[test]
        fn test_nice_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = nice_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_nice_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = nice_main(args.iter().map(OsString::from));

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
        #[test]
        fn test_ct_app_adjustment() {
            let args = vec![ctcore::ct_util_name(), "--adjustment=20", "ls", "-l"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_n() {
            let args = vec![ctcore::ct_util_name(), "-n", "20", "ls", "-l"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_hyphen_values() {
            // 测试 -n 参数允许以连字符开头的值
            let args = vec![ctcore::ct_util_name(), "-n", "-5", "ls"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>("adjustment").unwrap(), "-5");
        }
    }
}
