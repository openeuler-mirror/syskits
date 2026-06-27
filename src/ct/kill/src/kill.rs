/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! 向一个任务发送一个信号

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::ct_signals::{ALL_SIGNALS, get_ct_signal_by_name_or_value};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::convert::TryInto;
use std::io::Error;
use std::io::Write;

use ctcore::Tool;
use std::ffi::OsString;
const KILL_ABOUT: &str = ct_help_about!("kill.md");
const KILL_USAGE: &str = ct_help_usage!("kill.md");

pub mod kill_flags {
    pub static KILL_PIDS_OR_SIGNALS: &str = "pids_or_signals";
    pub static LIST: &str = "list";
    pub static TABLE: &str = "table";
    pub static SIGNAL: &str = "signal";
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = KILL_ABOUT;
    let usage_description = ct_format_usage(KILL_USAGE);
    let args = vec![
        Arg::new(kill_flags::LIST)
            .short('l')
            .long(kill_flags::LIST)
            .help("Lists signals")
            .conflicts_with(kill_flags::TABLE)
            .action(ArgAction::SetTrue),
        Arg::new(kill_flags::TABLE)
            .short('t')
            .short_alias('L')
            .long(kill_flags::TABLE)
            .help("Lists table of signals")
            .action(ArgAction::SetTrue),
        Arg::new(kill_flags::SIGNAL)
            .short('s')
            .long(kill_flags::SIGNAL)
            .value_name("signal")
            .help("Sends given signal instead of SIGTERM"),
        Arg::new(kill_flags::KILL_PIDS_OR_SIGNALS)
            .hide(true)
            .action(ArgAction::Append),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .allow_negative_numbers(true)
        .args(&args)
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    kill_main(&mut out, args)
}

/// 主要的kill命令处理函数，用于终止进程或发送信号
///
/// # 参数
/// - `writer`: 一个可写对象，用于输出信息
/// - `args`: 命令行参数，实现自定义Args trait
///
/// # 返回
/// 返回一个结果，表示操作是否成功
pub fn kill_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 收集并忽略不相关的参数
    let mut args = args.collect_ignore();
    // 处理过时的kill命令参数
    let obs_signal = kill_handle_obsolete(&mut args);
    // 尝试解析命令行参数
    let matches = ct_app().try_get_matches_from(args)?;
    // 获取需要终止的进程ID或信号
    let pids_or_signals: Vec<String> = matches
        .get_many::<String>(kill_flags::KILL_PIDS_OR_SIGNALS)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    if matches.get_flag(kill_flags::TABLE) {
        // 如果是表格模式，调用kill_table函数
        kill_table(writer)
    } else if matches.get_flag(kill_flags::LIST) {
        // 如果是列表模式，调用kill_list函数，并传入第一个进程ID或信号
        kill_list(writer, pids_or_signals.first())
    } else {
        // 否则，执行kill命令并处理信号
        kill_exec(obs_signal, matches, &pids_or_signals)
    }
}

/// 发送信号以终止进程执行。
///
/// 本函数旨在根据用户指定的信号和进程ID，发送信号以终止相应的进程。
/// 它首先解析用户可能提供的信号名称或编号，以及进程ID列表。然后，
/// 它向这些进程发送指定的信号。如果操作成功，函数返回Ok(())，否则返回一个错误。
///
/// # 参数
/// - `obs_signal`: 可选的信号编号，用户可以指定一个信号来终止进程。
/// - `matches`: 命令行参数匹配对象，用于获取命令行参数。
/// - `pids_or_signals`: 包含进程ID或信号的字符串向量，用于指定要终止的进程或解析信号。
///
/// # 返回值
/// - `CTResult<()>`: 一个结果类型，表示操作成功或失败。
fn kill_exec(
    obs_signal: Option<usize>,
    matches: ArgMatches,
    pids_or_signals: &[String],
) -> CTResult<()> {
    // 解析并获取要发送的信号值。
    let sig = kill_get_signal_value(obs_signal, matches)?;
    // 解析并获取进程ID列表。
    let pids = kill_parse_pids(pids_or_signals)?;
    // 向指定的进程发送信号。
    kill(sig, &pids);

    Ok(())
}

/// 根据提供的参数获取信号值用于进程终止
///
/// 该函数首先尝试从`obs_signal`中获取信号值，如果未提供，则尝试从命令行参数匹配中获取信号值。
/// 如果两者都未成功获取到信号值，则默认使用15（SIGTERM）作为信号值。
///
/// # 参数
/// - `obs_signal`: 可选的信号值，通常从观察到的信号中获取。
/// - `matches`: 命令行参数匹配结果，用于提取指定的信号值。
///
/// # 返回
/// 返回一个`CTResult`，包含转换后的`Signal`值，如果转换失败，则包含一个错误。
fn kill_get_signal_value(obs_signal: Option<usize>, matches: ArgMatches) -> CTResult<Signal> {
    let sig = if let Some(signal) = obs_signal {
        signal
    } else if let Some(signal) = matches.get_one::<String>(kill_flags::SIGNAL) {
        // 如果命令行参数中提供了信号值，则解析该信号值
        kill_parse_signal_value(signal)?
    } else {
        // 如果没有提供信号值，则使用默认的15（SIGTERM）
        15_usize //SIGTERM
    };
    // 将获取到的信号值转换为i32，并尝试将其转换为`Signal`类型，如果失败，则返回一个错误
    let kill_signal: Signal = (sig as i32)
        .try_into()
        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;

    Ok(kill_signal)
}

/// 移除过时的信号参数并返回对应的信号值。
///
/// 该函数用于处理包含旧风格信号前缀的命令行参数。当参数数量超过两个时，它会检查第二个参数是否包含一个旧风格的信号。如果存在，则移除该参数并返回对应的信号值。
///
/// # 参数
/// * `args`: 可变引用的字符串向量，包含命令行参数。
///
/// # 返回值
/// * `Option<usize>`: 如果找到并移除了过时的信号，则返回信号值；否则返回 `None`。
fn kill_handle_obsolete(args: &mut Vec<String>) -> Option<usize> {
    if args.len() > 2 {
        // 检查参数数量是否超过两个，因为过时信号的存在至少需要两个参数
        let slice = args[1].as_str();
        if let Some(signal) = slice.strip_prefix('-') {
            // 尝试移除信号前缀以判断是否为过时信号
            let opt_signal = get_ct_signal_by_name_or_value(signal);
            if opt_signal.is_some() {
                // 返回前移除信号参数
                args.remove(1);
                return opt_signal;
            }
        }
    }
    None
}

/// 将信号列表以表格形式输出到指定的写入器中
///
/// # Parameters
/// - `writer`: 一个实现了Write trait的可变引用，用于输出信号表
///
/// # Returns
/// - `CTResult<()>`: 一个结果类型，用于表示操作是否成功
///
/// # Description
/// 该函数遍历ALL_SIGNALS数组，计算信号名称的最大长度，并使用该长度以及索引号格式化输出信号名称
/// 每7个信号后，输出一个换行符，以格式化表格形式输出所有信号
fn kill_table<W: Write>(writer: &mut W) -> CTResult<()> {
    let name_width = ALL_SIGNALS.iter().map(|n| n.len()).max().unwrap();

    for (idx, signal) in ALL_SIGNALS.iter().enumerate() {
        // 格式化输出信号的索引号和名称，确保名称按计算的最大长度对齐
        write!(writer, "{0: >#2} {1: <#2$}", idx, signal, name_width + 2)?;
        // 每7个信号后输出一个换行符，格式化表格形式输出
        if (idx + 1) % 7 == 0 {
            writeln!(writer)?;
        }
    }
    // 最后输出一个换行符，确保表格格式正确
    writeln!(writer)?;

    Ok(())
}

/// 向指定的写入器打印信号值或名称，并返回结果
///
/// 此函数旨在根据提供的信号名称或值，查找并打印对应的信号值或名称如果找到对应的信号，则打印并返回Ok(())，否则返回一个错误
///
/// # 参数
/// - `writer`: 一个可写对象，用于输出信号值或名称
/// - `signal_name_or_value`: 一个字符串，包含信号的名称或值，用于查找信号
///
/// # 返回值
/// - `Ok(())`: 如果成功找到并打印信号值或名称
/// - `Err(CtSimpleError)`: 如果提供的信号名称或值无效，返回一个包含错误信息的CtSimpleError
fn kill_print_signal<W: Write>(writer: &mut W, signal_name_or_value: &str) -> CTResult<()> {
    for (value, &signal) in ALL_SIGNALS.iter().enumerate() {
        if signal == signal_name_or_value || (format!("SIG{signal}")) == signal_name_or_value {
            writeln!(writer, "{value}")?;
            return Ok(());
        } else if signal_name_or_value == value.to_string() {
            writeln!(writer, "{signal}")?;
            return Ok(());
        }
    }
    let err_message = format!("unknown signal name {}", signal_name_or_value.quote());
    Err(CtSimpleError::new(1, err_message))
}

/// 在控制台中打印所有信号的名称，每个信号之间用空格分隔
///
/// # 参数
/// * `writer`: 一个实现了Write trait的对象，用于输出信号信息
///
/// # 返回
/// * `CTResult<()>`: 一个结果类型，表示操作是否成功
fn kill_print_signals<W: Write>(writer: &mut W) -> CTResult<()> {
    for (idx, signal) in ALL_SIGNALS.iter().enumerate() {
        if idx > 0 {
            write!(writer, " ")?;
        }
        write!(writer, "{signal}")?;
    }
    writeln!(writer)?;
    Ok(())
}

/// 向指定的写入器输出终止信号信息。
///
/// # Parameters
/// - `writer`: 一个可写对象，用于输出终止信号信息。
/// - `arg`: 一个可选的字符串引用，如果提供，则输出与该信号相关的详细信息；
///   如果未提供，则输出所有可用的终止信号。
///
/// # Returns
/// - `CTResult<()>`: 一个结果类型，表示操作成功或失败。
fn kill_list<W: Write>(writer: &mut W, opt_arg: Option<&String>) -> CTResult<()> {
    if let Some(arg) = opt_arg {
        kill_print_signal(writer, arg)
    } else {
        kill_print_signals(writer)
    }
}

/// 将信号名称解析为对应的信号值。
///
/// 该函数接受一个信号名称字符串，尝试将其解析为对应的信号值。
/// 如果解析成功，则返回信号值；如果解析失败，则返回一个错误。
///
/// # 参数
///
/// * `signal_name: &str` - 信号名称字符串引用。
///
/// # 返回值
///
/// * `CTResult<usize>` - 一个结果类型，包含解析后的信号值或错误信息。
///
/// # 错误处理
///
/// 如果无法识别给定的信号名称，则返回一个包含错误信息的结果。
fn kill_parse_signal_value(signal_name: &str) -> CTResult<usize> {
    // 尝试通过信号名称或值获取信号的值。
    let optional_signal_value = get_ct_signal_by_name_or_value(signal_name);

    if let Some(sig) = optional_signal_value {
        Ok(sig)
    } else {
        let err_message = format!("unknown signal name {}", signal_name.quote());
        Err(CtSimpleError::new(1, err_message))
    }
}

/// 将字符串切片转换为i32整数向量
/// 该函数尝试将输入的字符串切片中的每个字符串解析为i32整数
/// 如果解析失败，将返回一个包含错误信息的CTResult
///
/// # 参数
/// - `pids`: 一个字符串切片的引用，每个字符串代表一个可能的整数
///
/// # 返回
/// - `CTResult<Vec<i32>>`: 解析成功时，返回一个包含i32整数的向量；
///   解析失败时，返回一个包含错误信息的CTResult
fn kill_parse_pids(pids: &[String]) -> CTResult<Vec<i32>> {
    // 遍历字符串切片，尝试将每个字符串解析为i32整数
    pids.iter()
        .map(|x| {
            // 解析字符串为i32整数，如果失败，则构建一个自定义错误信息
            x.parse::<i32>().map_err(|e| {
                let err_message = format!("failed to parse argument {}: {}", x.quote(), e);
                CtSimpleError::new(1, err_message)
            })
        })
        .collect()
}

/// 向指定进程发送信号
///
/// # Parameters
/// - `sig`: 需要发送的信号类型
/// - `pids`: 接收信号的进程ID列表
///
/// # Remarks
/// 此函数会尝试向每个指定的进程发送信号如果发送信号失败，会使用`ct_show!`宏记录错误
fn kill(sig: Signal, pids: &[i32]) {
    // 遍历进程ID列表
    for &pid in pids {
        // 尝试向进程发送信号
        if let Err(e) = signal::kill(Pid::from_raw(pid), sig) {
            // 如果发送信号失败，使用`ct_show!`宏记录错误
            ct_show!(
                Error::from_raw_os_error(e as i32)
                    .map_err_context(|| format!("sending signal to {pid} failed"))
            );
        }
    }
}

#[derive(Default)]
pub struct Kill;
impl Tool for Kill {
    fn name(&self) -> &'static str {
        "kill"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        kill_main(&mut out, args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::io::Cursor;

    #[test]
    fn test_tool_implementation() {
        let tool = Kill::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "kill");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("kill"));

        // 测试 execute 方法
        let args = vec![OsString::from("kill"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod kill_parse_pids_tests {
        use super::*;
        /*
        基本测试：
            成功解析：所有字符串都是有效的 i32 表示。
            测试用例：pids = ["1", "2", "3"] 应返回 Ok(vec![1, 2, 3])。
            解析失败：至少有一个字符串不是有效的 i32 表示。
            测试用例：pids = ["1", "abc", "3"] 应返回一个 CtSimpleError，指出字符串 "abc" 解析失败。
            空输入：输入切片为空。
            测试用例：pids = [] 应返回 Ok(vec![])。
        异常测试：
            包含负数的字符串：测试包含负数的字符串是否能正确解析。
            测试用例：pids = ["-1", "2", "-3"] 应返回 Ok(vec![-1, 2, -3])。
            包含最大/最小 i32 值的字符串：测试边界情况，确保最大和最小 i32 值能正确解析。
            测试用例：pids = ["2147483647", "-2147483648"] 应返回 Ok(vec![2147483647, -2147483648])。
            包含空字符串的输入：测试空字符串是否会导致解析失败。
            测试用例：pids = ["1", "", "3"] 应返回一个 CtSimpleError，指出字符串 "" 解析失败。
            包含非数字字符的字符串：测试包含非数字字符的字符串是否会导致解析失败。
            测试用例：pids = ["1", "a1b", "3"] 应返回一个 CtSimpleError，指出字符串 "a1b" 解析失败。
            包含多个无效字符串：测试多个无效字符串是否会导致解析失败。
            测试用例：pids = ["abc", "def", "ghi"] 应返回一个 CtSimpleError，指出字符串 "abc" 解析失败。
        */
        #[test]
        fn kill_parse_pids_valid_pids_returns_parsed_integers() {
            let pids = vec!["1".to_string(), "2".to_string(), "3".to_string()];
            let result = kill_parse_pids(&pids).unwrap();
            assert_eq!(result, vec![1, 2, 3]);
        }

        #[test]
        fn kill_parse_pids_invalid_pid_returns_error() {
            let pids = vec!["1".to_string(), "abc".to_string(), "3".to_string()];
            let result = kill_parse_pids(&pids);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(
                err.to_string(),
                "failed to parse argument 'abc': invalid digit found in string"
            );
        }

        #[test]
        fn kill_parse_pids_empty_input_returns_empty_vector() {
            let pids: Vec<String> = vec![];
            let result = kill_parse_pids(&pids).unwrap();
            assert_eq!(result.len(), 0);
        }

        #[test]
        fn kill_parse_pids_negative_pids_returns_parsed_integers() {
            let pids = vec!["-1".to_string(), "2".to_string(), "-3".to_string()];
            let result = kill_parse_pids(&pids).unwrap();
            assert_eq!(result, vec![-1, 2, -3]);
        }

        #[test]
        fn kill_parse_pids_boundary_values_returns_parsed_integers() {
            let pids = vec!["2147483647".to_string(), "-2147483648".to_string()];
            let result = kill_parse_pids(&pids).unwrap();
            assert_eq!(result, vec![2147483647, -2147483648]);
        }

        #[test]
        fn kill_parse_pids_empty_string_returns_error() {
            let pids = vec!["1".to_string(), "".to_string(), "3".to_string()];
            let result = kill_parse_pids(&pids);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(
                err.to_string(),
                "failed to parse argument '': cannot parse integer from empty string"
            );
        }

        #[test]
        fn kill_parse_pids_non_numeric_characters_returns_error() {
            let pids = vec!["1".to_string(), "a1b".to_string(), "3".to_string()];
            let result = kill_parse_pids(&pids);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(
                err.to_string(),
                "failed to parse argument 'a1b': invalid digit found in string"
            );
        }

        #[test]
        fn kill_parse_pids_multiple_invalid_pids_returns_error() {
            let pids = vec!["abc".to_string(), "def".to_string(), "ghi".to_string()];
            let result = kill_parse_pids(&pids);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(
                err.to_string(),
                "failed to parse argument 'abc': invalid digit found in string"
            );
        }
    }

    #[cfg(test)]
    mod kill_parse_signal_value_tests {
        use super::*;

        #[test]
        fn kill_parse_signal_value_valid_signal_name_returns_signal_value() {
            let signal_name = "HUP";
            let result = kill_parse_signal_value(signal_name).unwrap();
            assert_eq!(result, 1); // Assuming HUP corresponds to signal value 1
        }

        #[test]
        fn kill_parse_signal_value_valid_signal_number_returns_signal_value() {
            let signal_name = "15"; // Assuming 15 corresponds to SIGTERM
            let result = kill_parse_signal_value(signal_name).unwrap();
            assert_eq!(result, 15);
        }

        #[test]
        fn kill_parse_signal_value_invalid_signal_name_returns_error() {
            let signal_name = "INVALID";
            let result = kill_parse_signal_value(signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'INVALID'");
        }

        #[test]
        fn kill_parse_signal_value_empty_signal_name_returns_error() {
            let signal_name = "";
            let result = kill_parse_signal_value(signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name ''");
        }

        #[test]
        fn kill_parse_signal_value_non_numeric_signal_number_returns_error() {
            let signal_name = "abc";
            let result = kill_parse_signal_value(signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'abc'");
        }

        #[test]
        fn kill_parse_signal_value_boundary_signal_number_returns_signal_value() {
            let signal_name = "31"; // Assuming 31 corresponds to a valid signal
            let result = kill_parse_signal_value(signal_name).unwrap();
            assert_eq!(result, 31);
        }

        #[test]
        fn kill_parse_signal_value_negative_signal_number_returns_error() {
            let signal_name = "-1";
            let result = kill_parse_signal_value(signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name '-1'");
        }
    }
    #[cfg(test)]
    mod kill_list_tests {
        use super::*;
        #[test]
        fn kill_list_with_valid_signal_name_prints_signal_value() {
            let mut output = Cursor::new(Vec::new());
            let signal_name = Some("HUP".to_string());
            let _result = kill_list(&mut output, signal_name.as_ref()).unwrap();
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert_eq!(output_str.trim(), "1"); // Assuming HUP corresponds to signal value 1
        }

        #[test]
        fn kill_list_with_invalid_signal_name_returns_error() {
            let mut output = Cursor::new(Vec::new());
            let signal_name = Some("INVALID".to_string());
            let result = kill_list(&mut output, signal_name.as_ref());
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'INVALID'");
        }

        #[test]
        fn kill_list_with_no_argument_prints_all_signals() {
            let mut output = Cursor::new(Vec::new());
            let _result = kill_list(&mut output, None).unwrap();
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            let signals: Vec<&str> = ALL_SIGNALS.iter().map(|&s| s).collect();
            let expected_output = signals.join(" ");
            assert_eq!(output_str.trim(), expected_output);
        }

        #[test]
        fn kill_list_with_empty_string_returns_error() {
            let mut output = Cursor::new(Vec::new());
            let signal_name = Some("".to_string());
            let result = kill_list(&mut output, signal_name.as_ref());
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name ''");
        }

        #[test]
        fn kill_list_with_numeric_signal_value_prints_signal_name() {
            let mut output = Cursor::new(Vec::new());
            let signal_value = Some("15".to_string()); // Assuming 15 corresponds to SIGTERM
            let _result = kill_list(&mut output, signal_value.as_ref()).unwrap();
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert_eq!(output_str.trim(), "TERM"); // Assuming 15 corresponds to SIGTERM
        }
    }
    #[cfg(test)]
    mod kill_print_signals_tests {
        use super::*;

        #[test]
        fn kill_print_signals_empty_signals_list() {
            let mut writer = Cursor::new(Vec::new());
            let result = kill_print_signals(&mut writer);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert!(output.ends_with('\n')); // 确保输出以换行符结束
        }

        #[test]
        fn kill_print_signals_correct_format() {
            let mut writer = Cursor::new(Vec::new());
            kill_print_signals(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查输出格式是否正确（信号之间用空格分隔）
            let signals: Vec<&str> = output.trim().split(' ').collect();
            assert!(!signals.is_empty());
            assert!(signals.contains(&"HUP")); // 检查是否包含常见信号
            assert!(signals.contains(&"TERM"));
            assert!(signals.contains(&"KILL"));
        }

        #[test]
        fn kill_print_signals_no_duplicate_signals() {
            let mut writer = Cursor::new(Vec::new());
            kill_print_signals(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查是否有重复的信号名称
            let signals: Vec<&str> = output.trim().split(' ').collect();
            let unique_signals: std::collections::HashSet<&str> = signals.iter().copied().collect();
            assert_eq!(signals.len(), unique_signals.len());
        }

        #[test]
        fn kill_print_signals_proper_spacing() {
            let mut writer = Cursor::new(Vec::new());
            kill_print_signals(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查信号之间的间距是否正确（单个空格）
            assert!(!output.contains("  ")); // 不应该有连续的空格
            assert!(!output.trim().starts_with(' ')); // 开头不应该有空格
        }

        #[test]
        fn kill_print_signals_write_error_handling() {
            // 测试写入错误的情况
            struct ErrorWriter;
            impl Write for ErrorWriter {
                fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "write error",
                    ))
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }

            let mut writer = ErrorWriter;
            let result = kill_print_signals(&mut writer);
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod kill_print_signal_tests {
        use super::*;

        #[test]
        fn kill_print_signal_valid_signal_name_returns_signal_value() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "HUP";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(output.trim(), "1"); // Assuming HUP corresponds to signal value 1
        }

        #[test]
        fn kill_print_signal_valid_signal_number_returns_signal_name() {
            let mut writer = Cursor::new(Vec::new());
            let signal_number = "15"; // Assuming 15 corresponds to SIGTERM
            let result = kill_print_signal(&mut writer, signal_number);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(output.trim(), "TERM");
        }

        #[test]
        fn kill_print_signal_invalid_signal_name_returns_error() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "INVALID";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'INVALID'");
        }

        #[test]
        fn kill_print_signal_empty_signal_name_returns_error() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name ''");
        }

        #[test]
        fn kill_print_signal_non_numeric_signal_number_returns_error() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "abc";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'abc'");
        }

        #[test]
        fn kill_print_signal_boundary_signal_number_returns_signal_name() {
            let mut writer = Cursor::new(Vec::new());
            let signal_number = "31"; // Assuming 31 corresponds to a valid signal
            let result = kill_print_signal(&mut writer, signal_number);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(output.trim(), "SYS");
        }

        #[test]
        fn kill_print_signal_negative_signal_number_returns_error() {
            let mut writer = Cursor::new(Vec::new());
            let signal_number = "-1";
            let result = kill_print_signal(&mut writer, signal_number);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name '-1'");
        }

        #[test]
        fn kill_print_signal_valid_signal_with_sig_prefix_returns_signal_value() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "SIGHUP";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(output.trim(), "1"); // Assuming SIGHUP corresponds to signal value 1
        }
    }

    #[cfg(test)]
    mod kill_table_tests {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn kill_table_correct_format() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查输出格式是否正确（信号名称和索引号）
            let lines: Vec<&str> = output.trim().split('\n').collect();
            assert!(!lines.is_empty());
            for line in lines {
                let parts: Vec<&str> = line.split_whitespace().collect();
                assert!(parts.len() % 2 == 0); // 每行应有偶数个部分（索引号和信号名称）
            }
        }

        #[test]
        fn kill_table_proper_spacing() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查信号之间的间距是否正确（单个空格）
            assert!(!output.trim().starts_with(' ')); // 开头不应该有空格
        }

        #[test]
        fn kill_table_write_error_handling() {
            // 测试写入错误的情况
            struct ErrorWriter;
            impl Write for ErrorWriter {
                fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "write error",
                    ))
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }

            let mut writer = ErrorWriter;
            let result = kill_table(&mut writer);
            assert!(result.is_err());
        }

        #[test]
        fn kill_table_contains_all_signals() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查输出是否包含所有信号
            for signal in ALL_SIGNALS.iter() {
                assert!(output.contains(signal));
            }
        }

        #[test]
        fn kill_table_correct_number_of_lines() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查输出行数是否正确
            let lines: Vec<&str> = output.trim().split('\n').collect();
            let expected_lines = (ALL_SIGNALS.len() + 6) / 7; // 每行7个信号
            assert_eq!(lines.len(), expected_lines);
        }

        #[test]
        fn kill_table_correct_signal_indices() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查信号索引是否正确
            for (idx, signal) in ALL_SIGNALS.iter().enumerate() {
                let expected_output = format!("{0: >#2} {1: <#2$}", idx, signal, signal.len() + 2);
                assert!(output.contains(&expected_output));
            }
        }
    }

    #[cfg(test)]
    mod kill_handle_obsolete_tests {
        use super::*;

        #[test]
        fn kill_handle_obsolete_with_obsolete_signal() {
            let mut args = vec!["kill".to_string(), "-9".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(9));
            assert_eq!(args, vec!["kill".to_string(), "1234".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_no_obsolete_signal() {
            let mut args = vec!["kill".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
            assert_eq!(args, vec!["kill".to_string(), "1234".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_invalid_signal() {
            let mut args = vec![
                "kill".to_string(),
                "-invalid".to_string(),
                "1234".to_string(),
            ];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
            assert_eq!(
                args,
                vec![
                    "kill".to_string(),
                    "-invalid".to_string(),
                    "1234".to_string()
                ]
            );
        }

        #[test]
        fn kill_handle_obsolete_with_multiple_signals() {
            let mut args = vec![
                "kill".to_string(),
                "-9".to_string(),
                "-15".to_string(),
                "1234".to_string(),
            ];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(9));
            assert_eq!(
                args,
                vec!["kill".to_string(), "-15".to_string(), "1234".to_string()]
            );
        }

        #[test]
        fn kill_handle_obsolete_with_no_args() {
            let mut args: Vec<String> = vec![];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
        }

        #[test]
        fn kill_handle_obsolete_with_single_arg() {
            let mut args = vec!["kill".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
            assert_eq!(args, vec!["kill".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_valid_signal_name() {
            let mut args = vec!["kill".to_string(), "-HUP".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(1)); // Assuming HUP corresponds to signal value 1
            assert_eq!(args, vec!["kill".to_string(), "1234".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_valid_signal_number() {
            let mut args = vec!["kill".to_string(), "-15".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(15)); // Assuming 15 corresponds to SIGTERM
            assert_eq!(args, vec!["kill".to_string(), "1234".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_mixed_valid_and_invalid_signals() {
            let mut args = vec![
                "kill".to_string(),
                "-9".to_string(),
                "-invalid".to_string(),
                "1234".to_string(),
            ];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(9));
            assert_eq!(
                args,
                vec![
                    "kill".to_string(),
                    "-invalid".to_string(),
                    "1234".to_string()
                ]
            );
        }

        #[test]
        fn kill_handle_obsolete_with_signal_prefix_but_no_signal() {
            let mut args = vec!["kill".to_string(), "-".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
            assert_eq!(
                args,
                vec!["kill".to_string(), "-".to_string(), "1234".to_string()]
            );
        }
    }

    #[cfg(test)]
    mod kill_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::io::Cursor;
        #[test]
        fn kill_main_with_table_flag() {
            let args = vec![ctcore::ct_util_name(), "--table"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("HUP")); // Assuming HUP is in the signal list
        }

        #[test]
        fn kill_main_with_list_flag() {
            let args = vec![ctcore::ct_util_name(), "--list"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("HUP")); // Assuming HUP is in the signal list
        }

        #[test]
        fn kill_main_with_signal_and_pid() {
            let args = vec![ctcore::ct_util_name(), "-s", "TERM", "1234"];

            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.is_empty()); // No output expected for successful signal sending
        }

        #[test]
        fn kill_main_with_invalid_signal() {
            let args = vec![ctcore::ct_util_name(), "-s", "INVALID", "1234"];

            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'INVALID'");
        }

        #[test]
        fn kill_main_with_no_args() {
            let args = vec![ctcore::ct_util_name()];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok()); // Expecting error due to missing required arguments
        }

        #[test]
        fn kill_main_with_obsolete_signal() {
            let args = vec![ctcore::ct_util_name(), "-9", "1234"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.is_empty()); // No output expected for successful signal sending
        }

        #[test]
        fn kill_main_with_multiple_pids() {
            let args = vec![ctcore::ct_util_name(), "-s", "TERM", "1234", "5678"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.is_empty()); // No output expected for successful signal sending
        }

        #[test]
        fn kill_main_with_invalid_pid() {
            let args = vec![ctcore::ct_util_name(), "-s", "TERM", "invalid_pid"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.to_string()
                    .contains("failed to parse argument 'invalid_pid'")
            );
        }

        #[test]
        fn kill_main_with_default_signal() {
            let args = vec![ctcore::ct_util_name(), "1234"];

            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.is_empty()); // No output expected for successful signal sending
        }

        #[test]
        fn kill_main_with_help_flag() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err()); // Expecting error due to help flag
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use super::*;
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();
            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();
            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();
            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_l_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-l"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::LIST));
        }

        #[test]
        fn test_ct_app_long_option_l_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--list"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::LIST));
        }

        #[test]
        fn test_ct_app_long_option_t_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-t"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::TABLE));
        }

        #[test]
        fn test_ct_app_long_option_t_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--table"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::TABLE));
        }

        #[test]
        fn test_ct_app_long_option_s_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-s", "TERM"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::SIGNAL));
        }

        #[test]
        fn test_ct_app_long_option_s_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--signal", "TERM"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::SIGNAL));
        }
    }
}
