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

// 在GNU/Linux系统中，nice命令的主要作用是调整程序的执行优先级，从而影响其对CPU资源的访问

use libc::{PRIO_PROCESS, c_char, c_int, execvp};
use std::ffi::{CString, OsString};
use std::io::{Error, Write};
use std::ptr;

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::{
    Tool,
    ct_error::{CTResult, CTsageError, CtSimpleError, UClapError, set_ct_exit_code},
    ct_format_usage, ct_help_about, ct_help_usage, ct_show_error,
};

pub mod opt_flags {
    pub static ADJUSTMENT: &str = "adjustment";
    pub static COMMAND: &str = "COMMAND";
}

const NICE_ABOUT: &str = ct_help_about!("nice.md");
const NICE_USAGE: &str = ct_help_usage!("nice.md");

fn is_prefix_of(prefix: &str, target: &str, min_match: usize) -> bool {
    if prefix.len() < min_match || prefix.len() > target.len() {
        return false;
    }

    &target[0..prefix.len()] == prefix
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

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    nice_main(args).map(|_| ())
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
    // 标准化命令行参数
    let args = standardize_nice_args(args);

    // 使用 clap 库解析命令行参数
    let args_match = ct_app().try_get_matches_from(args).with_exit_code(125)?;

    // 清除之前的错误信息，并获取当前进程的优先级
    nix::errno::Errno::clear();
    let mut nice_ness = match nice_getprority() {
        Ok(value) => value,
        Err(value) => return value,
    };

    // 解析调整优先级的值，并进行验证
    let nice_adjustment = match nice_adjustment(&args_match, &mut nice_ness) {
        Ok(value) => value,
        Err(value) => return value,
    };

    // 应用优先级调整
    nice_ness += nice_adjustment;

    // 设置新的优先级，若失败则显示警告信息并返回错误码 125
    if let Some(value) = nice_setprority(nice_ness) {
        return value;
    }

    // 准备执行命令需要的参数，并执行命令
    let cstr: Vec<CString> = args_match
        .get_many::<String>(opt_flags::COMMAND)
        .unwrap()
        .map(|x| CString::new(x.as_bytes()).unwrap())
        .collect();

    let mut args: Vec<*const c_char> = cstr.iter().map(|s| s.as_ptr()).collect();
    args.push(ptr::null::<c_char>());
    unsafe {
        execvp(args[0], args.as_mut_ptr());
    }

    // 执行命令失败的处理
    ct_show_error!("execvp: {}", Error::last_os_error());
    let exit_code = if Error::last_os_error().raw_os_error().unwrap() as c_int == libc::ENOENT {
        127
    } else {
        126
    };
    set_ct_exit_code(exit_code);
    Ok(())
}

fn nice_getprority() -> Result<c_int, CTResult<()>> {
    let nice_ness = unsafe { libc::getpriority(PRIO_PROCESS, 0) };
    if Error::last_os_error().raw_os_error().unwrap() != 0 {
        return Err(Err(CtSimpleError::new(
            125,
            format!("getpriority: {}", Error::last_os_error()),
        )));
    }
    Ok(nice_ness)
}

fn nice_setprority(nice_ness: c_int) -> Option<CTResult<()>> {
    if unsafe { libc::setpriority(PRIO_PROCESS, 0, nice_ness) } == -1
        && write!(
            std::io::stderr(),
            "{}: warning: setpriority: {}",
            ctcore::ct_util_name(),
            Error::last_os_error()
        )
        .is_err()
    {
        set_ct_exit_code(125);
        return Some(Ok(()));
    }
    None
}

fn nice_adjustment(args_match: &ArgMatches, nice_ness: &mut c_int) -> Result<i32, CTResult<()>> {
    let nice_adjustment = match args_match.get_one::<String>(opt_flags::ADJUSTMENT) {
        Some(n_str) => {
            if !args_match.contains_id(opt_flags::COMMAND) {
                return Err(Err(CTsageError::new(
                    125,
                    "A command must be given with an adjustment.",
                )));
            }
            match n_str.parse() {
                Ok(num) => num,
                Err(e) => {
                    return Err(Err(CtSimpleError::new(
                        125,
                        format!("\"{n_str}\" is not a valid number: {e}"),
                    )));
                }
            }
        }
        None => {
            if !args_match.contains_id(opt_flags::COMMAND) {
                println!("{nice_ness}");
                return Err(Ok(()));
            }
            10_i32
        }
    };
    Ok(nice_adjustment)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = NICE_ABOUT;
    let usage_description = ct_format_usage(NICE_USAGE);

    let args = vec![
        Arg::new(opt_flags::ADJUSTMENT)
            .short('n')
            .long(opt_flags::ADJUSTMENT)
            .help("add N to the niceness (default is 10)")
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

    mod tests_nice_main {
        use crate::nice_main;

        use std::ffi::OsString;

        #[test]
        fn test_nice_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = nice_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_nice_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = nice_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_nice_main_n() {
            let args = vec![ctcore::ct_util_name(), "-n", "25", "ls"];

            let result = nice_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_nice_main_adjustment() {
            let args = vec![ctcore::ct_util_name(), "--adjustment=20", "ls", "-l"];
            let result = nice_main(args.iter().map(|s| OsString::from(s)));

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
    }
}
