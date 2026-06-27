/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! 因式分解的命令行接口

use ctcore::Tool;
use std::ffi::OsString;
use std::io::BufRead;
use std::io::{self, Write, stdin, stdout};
mod factor_algorithm;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, FromIo, set_ct_exit_code};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show_error, ct_show_warning};
pub use factor_algorithm::*;

pub mod miller_rabin;
pub mod numeric;
pub mod rho;
pub mod table;

const FACTOR_ABOUT: &str = ct_help_about!("factor.md");
const FACTOR_USAGE: &str = ct_help_usage!("factor.md");

/// 定义配置标志常量
pub mod factor_flags {
    /// 使用指数表示法标志
    pub const EXPONENTS: &str = "exponents";
    /// 帮助标志
    pub const HELP: &str = "help";
    /// 要分解的数字参数
    pub const NUMBER: &str = "NUMBER";
}

/// 因式分解命令的配置结构体
struct FactorFlags {
    /// 是否使用指数表示法
    print_exponents: bool,
    /// 要分解的数字列表
    numbers: Vec<String>,
}

/// 为 `FactorFlags` 实现默认值
impl Default for FactorFlags {
    fn default() -> Self {
        Self {
            print_exponents: false,
            numbers: Vec::new(),
        }
    }
}

impl FactorFlags {
    /// 从命令行参数创建配置结构体
    fn new(matches: &ArgMatches) -> Self {
        // 布尔标志提取模式
        let print_exponents = matches.get_flag(factor_flags::EXPONENTS);

        // 向量类型参数提取模式
        let numbers = matches
            .get_many::<String>(factor_flags::NUMBER)
            .map_or_else(Vec::new, |v| v.cloned().collect());

        // 构建并返回结构体
        Self {
            print_exponents,
            numbers,
        }
    }
}

/// 处理单个数字的因式分解并输出结果
fn factors_print_str(
    num_str: &str,
    w: &mut io::BufWriter<impl io::Write>,
    is_print_exponents: bool,
) -> io::Result<()> {
    let x = match num_str.trim().parse::<u64>() {
        Ok(x) => x,
        Err(e) => {
            // We return Ok() instead of Err(), because it's non-fatal and we should try the next
            // number.
            ct_show_warning!("{}: {}", num_str.maybe_quote(), e);
            set_ct_exit_code(1);
            return Ok(());
        }
    };

    if is_print_exponents {
        writeln!(w, "{}:{:#}", x, factor(x))?;
    } else {
        writeln!(w, "{}:{}", x, factor(x))?;
    }

    w.flush()
}

/// 处理从标准输入读取的数字
fn process_stdin(w: &mut io::BufWriter<impl io::Write>, print_exponents: bool) -> CTResult<()> {
    let stdin = stdin();
    let lines = stdin.lock().lines();
    for line in lines {
        match line {
            Ok(line) => {
                for number in line.split_whitespace() {
                    factors_print_str(number, w, print_exponents)
                        .map_err_context(|| "write error".into())?;
                }
            }
            Err(e) => {
                set_ct_exit_code(1);
                ct_show_error!("error reading input: {}", e);
                return Ok(());
            }
        }
    }
    Ok(())
}

/// 处理命令行参数中提供的数字
fn process_numbers(
    numbers: &[String],
    w: &mut io::BufWriter<impl io::Write>,
    print_exponents: bool,
) -> CTResult<()> {
    for number in numbers {
        factors_print_str(number, w, print_exponents).map_err_context(|| "write error".into())?;
    }
    Ok(())
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let stdout = stdout();
    let mut w = io::BufWriter::with_capacity(4 * 1024, stdout.lock());
    factor_main(args, &mut w)
}

#[derive(Default)]
pub struct Factor;
impl Tool for Factor {
    fn name(&self) -> &'static str {
        "factor"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let stdout = stdout();
        let mut w = io::BufWriter::with_capacity(4 * 1024, stdout.lock());
        factor_main(args.iter().cloned(), &mut w)
    }
}
/// 因式分解命令的主函数
///
/// # Errors
///
/// 返回错误如果：
/// - 命令行参数解析失败
/// - 写入输出时发生 I/O 错误
/// - 处理输入数字时发生错误
pub fn factor_main(args: impl ctcore::Args, w: &mut io::BufWriter<impl io::Write>) -> CTResult<()> {
    // 1. 解析命令行参数
    let matches = ct_app().try_get_matches_from(args)?;

    // 2. 创建配置对象
    let settings = FactorFlags::new(&matches);

    // 3. 使用配置执行主要逻辑
    if settings.numbers.is_empty() {
        // 处理从标准输入读取的数字
        process_stdin(w, settings.print_exponents)?;
    } else {
        // 处理命令行参数中提供的数字
        process_numbers(&settings.numbers, w, settings.print_exponents)?;
    }

    // 确保所有输出都被刷新
    if let Err(e) = w.flush() {
        ct_show_error!("{}", e);
    }

    Ok(())
}

/// 创建命令行应用程序
#[must_use]
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = FACTOR_ABOUT;
    let usage_description = ct_format_usage(FACTOR_USAGE);
    let args = vec![
        Arg::new(factor_flags::NUMBER).action(ArgAction::Append),
        Arg::new(factor_flags::EXPONENTS)
            .short('h')
            .long(factor_flags::EXPONENTS)
            .help("Print factors in the form p^e")
            .action(ArgAction::SetTrue),
        Arg::new(factor_flags::HELP)
            .long(factor_flags::HELP)
            .help("Print help information.")
            .action(ArgAction::Help),
    ];

    // 构建并配置命令行解析器
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufWriter;

    /// 测试 FactorFlags 的默认值
    #[test]
    fn test_factor_flags_default() {
        let flags = FactorFlags::default();
        assert!(!flags.print_exponents);
        assert!(flags.numbers.is_empty());
    }

    /// 测试从 ArgMatches 创建 FactorFlags
    #[test]
    fn test_factor_flags_new() {
        // 创建一个带有指数标志的 ArgMatches
        let matches = ct_app().try_get_matches_from(vec!["factor", "-h"]).unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(flags.print_exponents);
        assert!(flags.numbers.is_empty());

        // 创建一个带有数字参数的 ArgMatches
        let matches = ct_app()
            .try_get_matches_from(vec!["factor", "12", "24"])
            .unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(!flags.print_exponents);
        assert_eq!(flags.numbers, vec!["12", "24"]);

        // 创建一个同时带有指数标志和数字参数的 ArgMatches
        let matches = ct_app()
            .try_get_matches_from(vec!["factor", "-h", "12", "24"])
            .unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(flags.print_exponents);
        assert_eq!(flags.numbers, vec!["12", "24"]);
    }

    /// 测试 factors_print_str 函数 - 普通格式
    #[test]
    fn test_factors_print_str_normal_format() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试数字 12 的因式分解（普通格式）
        factors_print_str("12", &mut writer, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "12: 2 2 3\n");
    }

    /// 测试 factors_print_str 函数 - 指数格式
    #[test]
    fn test_factors_print_str_exponent_format() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试数字 12 的因式分解（指数格式）
        factors_print_str("12", &mut writer, true).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "12: 2^2 3\n");
    }

    /// 测试 process_numbers 函数
    #[test]
    fn test_process_numbers() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试处理多个数字
        let numbers = vec!["12".to_string(), "24".to_string()];
        process_numbers(&numbers, &mut writer, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "12: 2 2 3\n24: 2 2 2 3\n");
    }

    /// 测试无效输入的处理
    #[test]
    fn test_invalid_input() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试无效输入（非数字）
        // 注意：这个测试会产生警告消息，但不会失败
        factors_print_str("abc", &mut writer, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, ""); // 无效输入不会产生输出
    }

    /// 测试边界情况
    #[test]
    fn test_edge_cases() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试 0
        factors_print_str("0", &mut writer, false).unwrap();

        // 测试 1
        factors_print_str("1", &mut writer, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "0: 0\n1:\n"); // 0 的因子是 0，1 没有素因子
    }

    /// 测试大数
    #[test]
    fn test_large_numbers() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试较大的数字
        factors_print_str("1234567", &mut writer, true).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        // 1234567 = 127 * 9721
        assert_eq!(output, "1234567: 127 9721\n");
    }

    /// 测试命令行应用程序配置
    #[test]
    fn test_ct_app_configuration() {
        let app = ct_app();

        // 验证应用程序名称和版本
        assert_eq!(app.get_name(), ctcore::ct_util_name());

        // 验证参数配置
        let args: Vec<_> = app.get_arguments().collect();
        assert!(args.iter().any(|arg| arg.get_id() == factor_flags::NUMBER));
        assert!(
            args.iter()
                .any(|arg| arg.get_id() == factor_flags::EXPONENTS)
        );
        assert!(args.iter().any(|arg| arg.get_id() == factor_flags::HELP));
    }
}
