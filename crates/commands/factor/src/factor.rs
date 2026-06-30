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

//! 因式分解的命令行接口

extern crate rust_i18n;
use ctcore::Tool;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::BufRead;
use std::io::{self, Write, stdin, stdout};
mod factor_algorithm;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo, set_ct_exit_code};
use ctcore::ct_show_error;
pub use factor_algorithm::*;
use num_bigint::BigUint;
use num_prime::nt_funcs::factorize;
use num_traits::{One, ToPrimitive, Zero};
use sys_locale::get_locale;

pub mod miller_rabin;
pub mod numeric;
pub mod rho;
pub mod table;

/// 定义配置标志常量
pub mod factor_flags {
    /// 使用指数表示法标志
    pub const EXPONENTS: &str = "exponents";
    /// 帮助标志
    pub const HELP: &str = "help";
    /// 版本标志
    pub const VERSION: &str = "version";
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

#[derive(Debug)]
enum ParsedNumber {
    Small(u64),
    Big(BigUint),
}

impl ParsedNumber {}

fn parse_number_token(token: &str) -> Option<ParsedNumber> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }

    let stripped = trimmed.strip_prefix('+').unwrap_or(trimmed);
    if stripped.is_empty() || stripped.starts_with('-') {
        return None;
    }

    if !stripped.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let value = BigUint::parse_bytes(stripped.as_bytes(), 10)?;
    if let Some(n) = value.to_u64() {
        return Some(ParsedNumber::Small(n));
    }

    Some(ParsedNumber::Big(value))
}

fn format_big_factors(factors: &BTreeMap<BigUint, usize>, print_exponents: bool) -> String {
    let mut output = String::new();
    for (prime, exp) in factors {
        if print_exponents && *exp > 1 {
            output.push(' ');
            output.push_str(&prime.to_string());
            output.push('^');
            output.push_str(&exp.to_string());
        } else {
            let prime_str = prime.to_string();
            for _ in 0..*exp {
                output.push(' ');
                output.push_str(&prime_str);
            }
        }
    }

    output
}

fn factorize_biguint(value: &BigUint) -> BTreeMap<BigUint, usize> {
    let mut n = value.clone();
    let mut factors = BTreeMap::new();
    if n.is_zero() || n.is_one() {
        return factors;
    }

    let one = BigUint::one();
    let two = BigUint::from(2u8);
    let mut count = 0usize;
    while (&n & &one).is_zero() {
        n >>= 1;
        count += 1;
    }
    if count > 0 {
        factors.insert(two, count);
    }

    if n.is_one() {
        return factors;
    }

    let rest = factorize(n);
    for (prime, exp) in rest {
        *factors.entry(prime).or_insert(0) += exp;
    }

    factors
}

fn validate_options(args: &[OsString]) -> CTResult<()> {
    let mut options_done = false;
    for arg in args.iter().skip(1) {
        let arg_str = arg.to_string_lossy();
        if options_done {
            continue;
        }

        if arg_str == "--" {
            options_done = true;
            continue;
        }

        if arg_str.starts_with("--") {
            if arg_str == "--exponents" || arg_str == "--help" || arg_str == "--version" {
                continue;
            }
            ct_show_error!("unrecognized option '{}'", arg_str);
            eprintln!(
                "Try '{} --help' for more information.",
                ctcore::ct_util_name()
            );
            return Err(CtSimpleError::new(1, ""));
        }

        if arg_str.starts_with('-') && arg_str != "-" {
            if arg_str == "-h" || arg_str == "-V" {
                continue;
            }
            let invalid = arg_str.chars().nth(1).unwrap_or('-');
            ct_show_error!("invalid option -- '{}'", invalid);
            eprintln!(
                "Try '{} --help' for more information.",
                ctcore::ct_util_name()
            );
            return Err(CtSimpleError::new(1, ""));
        }
    }

    Ok(())
}

/// 处理单个数字的因式分解并输出结果
fn factors_print_str(
    num_str: &str,
    w: &mut io::BufWriter<impl io::Write>,
    is_print_exponents: bool,
) -> io::Result<()> {
    let display_token = num_str.trim();
    let parsed = match parse_number_token(display_token) {
        Some(parsed) => parsed,
        None => {
            ct_show_error!("{} is not a valid positive integer", display_token.quote());
            set_ct_exit_code(1);
            return Ok(());
        }
    };

    match parsed {
        ParsedNumber::Small(x) => {
            if is_print_exponents {
                writeln!(w, "{}:{:#}", x, factor(x))?;
            } else {
                writeln!(w, "{}:{}", x, factor(x))?;
            }
        }
        ParsedNumber::Big(x) => {
            let output = if x.is_zero() || x.is_one() {
                String::new()
            } else {
                let factors = factorize_biguint(&x);
                format_big_factors(&factors, is_print_exponents)
            };
            writeln!(w, "{x}:{output}")?;
        }
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
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 1. 解析命令行参数
    let args_vec: Vec<OsString> = args.into_iter().collect();
    validate_options(&args_vec)?;
    let matches = ct_app().try_get_matches_from(&args_vec)?;

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
    let application_info = t!("factor.about");
    let usage_description = t!("factor.usage");
    let args = vec![
        Arg::new(factor_flags::NUMBER).action(ArgAction::Append),
        Arg::new(factor_flags::EXPONENTS)
            .short('h')
            .long(factor_flags::EXPONENTS)
            .help(t!("factor.clap.exponents"))
            .action(ArgAction::SetTrue),
        Arg::new(factor_flags::HELP)
            .long(factor_flags::HELP)
            .help(t!("factor.clap.help"))
            .action(ArgAction::Help),
        Arg::new(factor_flags::VERSION)
            .short('V')
            .long(factor_flags::VERSION)
            .help(t!("factor.clap.version"))
            .action(ArgAction::Version),
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
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Factor;

        // Test name method
        assert_eq!(tool.name(), "factor");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("factor"));

        // Test execute method - should work with default arguments
        let args = vec![OsString::from("factor"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());

        // Test with specific number
        let args = vec![OsString::from("factor"), OsString::from("42")];
        assert!(tool.execute(&args).is_ok());
    }

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

    use std::io::BufWriter;

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
        assert_eq!(output, "0:\n1:\n"); // 0 和 1 都没有素因子
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
