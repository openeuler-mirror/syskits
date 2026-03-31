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

use std::io::BufRead;
use std::io::{self, Write, stdin, stdout};

mod factor;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, FromIo, set_ct_exit_code};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show_error, ct_show_warning};
pub use factor::*;

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
