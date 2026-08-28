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
extern crate rust_i18n;
use rust_i18n::t;
use std::io::{ErrorKind, Write, stdout};
rust_i18n::i18n!("locales", fallback = "en-US");

use clap::{Arg, ArgAction, Command, crate_version};
use num_traits::{ToPrimitive, Zero};

use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError};
use ctcore::ct_format::{Format, num_format};
use std::ffi::OsString;
use sys_locale::get_locale;
mod error;
mod extendedbigdecimal;
mod number;
mod numberparse;
use crate::error::SeqError;
use crate::extendedbigdecimal::ExtendedBigDecimal;
use crate::number::PreciseNumber;

const SEQ_SEPARATOR: &str = "separator";
const SEQ_TERMINATOR: &str = "terminator";
const SEQ_EQUAL_WIDTH: &str = "equal-width";
const SEQ_FORMAT: &str = "format";

const SEQ_NUMBERS: &str = "numbers";

// Fast path optimization limit (same as GNU seq)
const SEQ_FAST_STEP_LIMIT: u64 = 200;

#[derive(Clone, Default)]
struct SeqOptions {
    separator: String,
    terminator: String,
    is_equal_width: bool,
    format: Option<String>,
}

impl SeqOptions {
    fn new(matches: &clap::ArgMatches) -> Self {
        let unescape = |s: &str| -> String {
            if let Some(stripped) = s.strip_prefix("CT_NEG_") {
                format!("-{stripped}")
            } else {
                s.to_string()
            }
        };

        Self {
            separator: matches
                .get_one::<String>(SEQ_SEPARATOR)
                .map(|s| unescape(s.as_str()))
                .unwrap_or_else(|| "\n".to_string()),
            terminator: matches
                .get_one::<String>(SEQ_TERMINATOR)
                .map(|s| unescape(s.as_str()))
                .unwrap_or_else(|| "\n".to_string()),
            is_equal_width: matches.get_flag(SEQ_EQUAL_WIDTH),
            format: matches
                .get_one::<String>(SEQ_FORMAT)
                .map(|s| unescape(s.as_str())),
        }
    }
}

/// A range of floats.
///
/// The elements are (first, increment, last).
type RangeFloat = (ExtendedBigDecimal, ExtendedBigDecimal, ExtendedBigDecimal);

/// 序列打印的配置参数
struct PrintConfig<'a> {
    largest_dec: usize,
    separator: &'a str,
    terminator: &'a str,
    pad: bool,
    padding: usize,
    format: &'a Option<Format<num_format::Float>>,
    format_str: Option<&'a str>,
    buffer: Option<&'a mut Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqRow {
    pub index: usize,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqSemantic {
    pub rows: Vec<SeqRow>,
    pub classic_text: String,
}

pub fn seq_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    // 核心拦截器：在参数送入 clap 之前进行“易容伪装”。
    // 将所有形如负数的参数（如 -1e-3, -.1）伪装成 CT_NEG_xxx，完美绕过 clap 的死板校验。
    let mut modified_args: Vec<OsString> = Vec::new();
    for arg in args {
        let arg_str = arg.to_string_lossy();
        if arg_str.starts_with('-') && arg_str.len() > 1 {
            let second_char = arg_str.chars().nth(1).unwrap();
            // 如果破折号后面紧跟的是数字或小数点，认定它是负数值
            if second_char.is_ascii_digit() || second_char == '.' {
                let mut safe_arg = "CT_NEG_".to_string();
                safe_arg.push_str(&arg_str[1..]);
                modified_args.push(safe_arg.into());
                continue;
            }
        }
        modified_args.push(arg);
    }

    let matches = ct_app().try_get_matches_from(modified_args)?;
    let options = SeqOptions::new(&matches);

    let numbers = parse_number_args(&matches)?;
    let (first, increment, last) = get_sequence_range(&numbers)?;

    // Try fast path optimization first
    if let Some((first_u64, last_u64, step_u64)) =
        can_use_fast_path(&first, &increment, &last, &options)
    {
        return match seq_fast(
            first_u64,
            last_u64,
            step_u64,
            &options.separator,
            &options.terminator,
        ) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(()),
            Err(e) => Err(CtSimpleError::new(1, format!("write error: {e}"))),
        };
    }

    let padding = calculate_padding(&first, &increment, &last);
    let largest_dec = calculate_largest_decimal(&first, &increment);
    let format = parse_format_option(options.format.as_deref())?;

    let config = PrintConfig {
        largest_dec,
        separator: &options.separator,
        terminator: &options.terminator,
        pad: options.is_equal_width,
        padding,
        format: &format,
        format_str: options.format.as_deref(),
        buffer: None,
    };

    match print_seq((first.number, increment.number, last.number), config) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(CtSimpleError::new(1, format!("write error: {e}"))),
    }
}

pub fn seq_native_semantic(args: impl ctcore::Args) -> CTResult<SeqSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let mut modified_args: Vec<OsString> = Vec::new();
    for arg in args {
        let arg_str = arg.to_string_lossy();
        if arg_str.starts_with('-') && arg_str.len() > 1 {
            let second_char = arg_str.chars().nth(1).unwrap();
            if second_char.is_ascii_digit() || second_char == '.' {
                let mut safe_arg = "CT_NEG_".to_string();
                safe_arg.push_str(&arg_str[1..]);
                modified_args.push(safe_arg.into());
                continue;
            }
        }
        modified_args.push(arg);
    }

    let matches = ct_app().try_get_matches_from(modified_args)?;
    let options = SeqOptions::new(&matches);
    let numbers = parse_number_args(&matches)?;
    let (first, increment, last) = get_sequence_range(&numbers)?;

    let padding = calculate_padding(&first, &increment, &last);
    let largest_dec = calculate_largest_decimal(&first, &increment);
    let format = parse_format_option(options.format.as_deref())?;

    let mut classic_buffer = Vec::new();
    let mut rows = Vec::new();
    collect_seq_rows(
        (
            first.number.clone(),
            increment.number.clone(),
            last.number.clone(),
        ),
        &SeqRenderConfig {
            largest_dec,
            separator: &options.separator,
            terminator: &options.terminator,
            pad: options.is_equal_width,
            padding,
            format: &format,
            format_str: options.format.as_deref(),
        },
        &mut rows,
        &mut classic_buffer,
    )
    .map_err(|e| CtSimpleError::new(1, format!("write error: {e}")))?;

    Ok(SeqSemantic {
        rows,
        classic_text: String::from_utf8(classic_buffer).expect("seq output should be utf-8"),
    })
}

fn parse_number_args(matches: &clap::ArgMatches) -> CTResult<Vec<String>> {
    Ok(matches
        .get_many::<String>(SEQ_NUMBERS)
        .ok_or(SeqError::NoArguments)?
        .map(|s| {
            if let Some(stripped) = s.strip_prefix("CT_NEG_") {
                format!("-{stripped}")
            } else {
                s.to_string()
            }
        })
        .collect::<Vec<_>>())
}

fn get_sequence_range(
    numbers: &[String],
) -> CTResult<(PreciseNumber, PreciseNumber, PreciseNumber)> {
    let first = if numbers.len() > 1 {
        numbers[0]
            .parse()
            .map_err(|e| SeqError::ParseError(numbers[0].clone(), e))?
    } else {
        PreciseNumber::one()
    };

    let increment = if numbers.len() > 2 {
        let inc: PreciseNumber = numbers[1]
            .parse()
            .map_err(|e| SeqError::ParseError(numbers[1].clone(), e))?;
        if inc.is_zero() {
            return Err(SeqError::ZeroIncrement(numbers[1].clone()).into());
        }
        inc
    } else {
        PreciseNumber::one()
    };

    let last = numbers
        .last()
        .unwrap()
        .parse()
        .map_err(|e| SeqError::ParseError(numbers.last().unwrap().clone(), e))?;

    Ok((first, increment, last))
}

fn calculate_padding(
    first: &PreciseNumber,
    increment: &PreciseNumber,
    last: &PreciseNumber,
) -> usize {
    first
        .num_integral_digits
        .max(increment.num_integral_digits)
        .max(last.num_integral_digits)
}

fn calculate_largest_decimal(first: &PreciseNumber, increment: &PreciseNumber) -> usize {
    first
        .num_fractional_digits
        .max(increment.num_fractional_digits)
}

fn parse_format_option(format_str: Option<&str>) -> CTResult<Option<Format<num_format::Float>>> {
    match format_str {
        Some(f) => Format::<num_format::Float>::parse(f)
            .map(Some)
            .map_err(|e| Box::new(e) as Box<dyn CTError>),
        None => Ok(None),
    }
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(SEQ_SEPARATOR)
            .short('s')
            .long("separator")
            .help(t!("seq.clap.seq_separator")),
        Arg::new(SEQ_TERMINATOR)
            .short('t')
            .long("terminator")
            .help(t!("seq.clap.seq_terminator")),
        Arg::new(SEQ_EQUAL_WIDTH)
            .short('w')
            .long("equal-width")
            .help(t!("seq.clap.seq_equal_width"))
            .action(ArgAction::SetTrue),
        Arg::new(SEQ_FORMAT)
            .short('f')
            .long(SEQ_FORMAT)
            .help(t!("seq.clap.seq_format")),
        Arg::new(SEQ_NUMBERS)
            .action(ArgAction::Append)
            .num_args(1..=3),
    ];

    Command::new(ctcore::ct_util_name())
        .trailing_var_arg(true)
        .allow_negative_numbers(true)
        .infer_long_args(true)
        .version(crate_version!())
        .about(t!("seq.about"))
        .override_usage(t!("seq.usage"))
        .args(args)
}

fn done_printing<T: Zero + PartialOrd>(next: &T, increment: &T, last: &T) -> bool {
    if increment >= &T::zero() {
        next > last
    } else {
        next < last
    }
}

/// Fast path for integer sequences with small steps
/// This uses string operations instead of floating point arithmetic for better performance
fn seq_fast(
    first: u64,
    last: u64,
    step: u64,
    separator: &str,
    terminator: &str,
) -> std::io::Result<()> {
    use std::io::BufWriter;

    let stdout = stdout();
    let mut writer = BufWriter::with_capacity(8192, stdout.lock());
    let mut current = first;
    let mut is_first = true;

    while current <= last {
        if !is_first {
            write!(writer, "{separator}")?;
        }
        write!(writer, "{current}")?;

        // Check for overflow before adding
        if let Some(next) = current.checked_add(step) {
            current = next;
        } else {
            break;
        }
        is_first = false;
    }

    if !is_first {
        write!(writer, "{terminator}")?;
    }
    writer.flush()
}

/// Check if we can use the fast path optimization
fn can_use_fast_path(
    first: &PreciseNumber,
    increment: &PreciseNumber,
    last: &PreciseNumber,
    options: &SeqOptions,
) -> Option<(u64, u64, u64)> {
    // Fast path conditions (same as GNU seq):
    // 1. No format string
    // 2. No equal-width
    // 3. Separator is single character (typically newline)
    // 4. All numbers are non-negative integers
    // 5. Step is positive and <= SEQ_FAST_STEP_LIMIT

    if options.format.is_some() || options.is_equal_width || options.separator.len() != 1 {
        return None;
    }

    // Check if all are integers (precision == 0)
    if first.num_fractional_digits != 0
        || increment.num_fractional_digits != 0
        || last.num_fractional_digits != 0
    {
        return None;
    }

    // Check if all are non-negative
    if first.number < ExtendedBigDecimal::zero() || last.number < ExtendedBigDecimal::zero() {
        return None;
    }

    // Try to convert to u64
    let first_u64 = match &first.number {
        ExtendedBigDecimal::BigDecimal(bd) => bd.to_u64()?,
        _ => return None,
    };

    let last_u64 = match &last.number {
        ExtendedBigDecimal::BigDecimal(bd) => bd.to_u64()?,
        _ => return None,
    };

    let step_u64 = match &increment.number {
        ExtendedBigDecimal::BigDecimal(bd) => bd.to_u64()?,
        _ => return None,
    };

    // Check step limit
    if step_u64 == 0 || step_u64 > SEQ_FAST_STEP_LIMIT {
        return None;
    }

    Some((first_u64, last_u64, step_u64))
}

/// Write a big decimal formatted according to the given parameters.
fn write_value_float(
    writer: &mut impl Write,
    value: &ExtendedBigDecimal,
    width: usize,
    precision: usize,
) -> std::io::Result<()> {
    let s = if *value == ExtendedBigDecimal::Infinity {
        "inf".to_string()
    } else if *value == ExtendedBigDecimal::MinusInfinity {
        "-inf".to_string()
    } else if *value == ExtendedBigDecimal::Nan {
        "nan".to_string()
    } else if precision > 0 {
        // 保留小数精度
        format!("{value:.precision$}")
    } else {
        // 模拟 C 语言 %g 的智能截断
        let mut s = value.to_string();
        if s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        s
    };

    // 手动进行前导 0 填充，避开原生 format! 宏对大数类型填充支持不佳的坑
    if s.len() < width {
        let pad_len = width - s.len();
        if let Some(stripped) = s.strip_prefix('-') {
            write!(writer, "-{}{stripped}", "0".repeat(pad_len))
        } else {
            write!(writer, "{}{s}", "0".repeat(pad_len))
        }
    } else {
        write!(writer, "{s}")
    }
}

struct SeqRenderConfig<'a> {
    largest_dec: usize,
    separator: &'a str,
    terminator: &'a str,
    pad: bool,
    padding: usize,
    format: &'a Option<Format<num_format::Float>>,
    format_str: Option<&'a str>,
}

fn render_seq_value(
    value: &ExtendedBigDecimal,
    config: &SeqRenderConfig<'_>,
) -> std::io::Result<String> {
    let padding = if config.pad {
        config.padding
            + if config.largest_dec > 0 {
                config.largest_dec + 1
            } else {
                0
            }
    } else {
        0
    };

    let mut buffer = Vec::new();
    match config.format {
        Some(f) => {
            let float = match value {
                ExtendedBigDecimal::BigDecimal(bd) => bd.to_f64().unwrap(),
                ExtendedBigDecimal::Infinity => f64::INFINITY,
                ExtendedBigDecimal::MinusInfinity => f64::NEG_INFINITY,
                ExtendedBigDecimal::MinusZero => -0.0,
                ExtendedBigDecimal::Nan => f64::NAN,
            };
            format_with_zero_padding(&mut buffer, f, float, config.format_str)?;
        }
        None => write_value_float(&mut buffer, value, padding, config.largest_dec)?,
    }
    Ok(String::from_utf8(buffer).expect("seq rendered value should be utf-8"))
}

fn collect_seq_rows(
    range: RangeFloat,
    config: &SeqRenderConfig<'_>,
    rows: &mut Vec<SeqRow>,
    writer: &mut Vec<u8>,
) -> std::io::Result<()> {
    let (first, increment, last) = range;
    let mut value = first;
    let mut is_first_iteration = true;
    let mut index = 0usize;

    while !done_printing(&value, &increment, &last) {
        let rendered = render_seq_value(&value, config)?;
        if !is_first_iteration {
            write!(writer, "{}", config.separator)?;
        }
        write!(writer, "{rendered}")?;
        rows.push(SeqRow {
            index,
            value: rendered,
        });
        value = value + increment.clone();
        is_first_iteration = false;
        index += 1;
    }
    if !is_first_iteration {
        write!(writer, "{}", config.terminator)?;
    }
    Ok(())
}

/// Custom format function that handles zero-padding with signs correctly
/// This fixes the issue where ctcore's Format doesn't handle the '0' flag properly with signs
/// 直接使用底层的格式化器，移除之前破坏 printf 标准的魔改逻辑
fn format_with_zero_padding(
    writer: &mut impl Write,
    format: &Format<num_format::Float>,
    float: f64,
    orig_format: Option<&str>,
) -> std::io::Result<()> {
    let mut temp_buf = Vec::new();
    format.fmt(&mut temp_buf, float)?;
    let mut formatted = String::from_utf8_lossy(&temp_buf).to_string();

    if let Some(fmt) = orig_format {
        if let Some(pct) = fmt.find('%') {
            let mut has_minus = false;
            let mut has_plus = false;
            let mut has_space = false;
            let mut has_zero = false;

            for c in fmt[pct + 1..].chars() {
                match c {
                    '-' => has_minus = true,
                    '+' => has_plus = true,
                    ' ' => has_space = true,
                    '0' => has_zero = true,
                    '1'..='9' | '.' | 'f' | 'g' | 'e' | 'E' | 'G' | 'F' => break,
                    _ => {}
                }
            }

            let prefix_len = pct;
            let mut dir_end = pct + 1;
            for (i, c) in fmt[pct + 1..].char_indices() {
                if "fFeEgG".contains(c) {
                    dir_end = pct + 1 + i + c.len_utf8();
                    break;
                }
            }
            let suffix_len = fmt.len() - dir_end;

            if formatted.len() > prefix_len + suffix_len {
                let num_part = &formatted[prefix_len..formatted.len() - suffix_len];

                let is_negative = float < 0.0 || float.is_sign_negative();

                // ctcore Bug: 处理非负数且要求显示 '+' 或 ' ' 标志时，
                // 它会在填充完宽度后，硬塞一个符号，导致总字符串超长 1 个字符。
                if !is_negative
                    && (has_plus || has_space)
                    && !num_part.is_empty()
                    && (num_part.starts_with('+') || num_part.starts_with(' '))
                {
                    if has_minus {
                        // 左对齐：多余的填充字符在尾部，直接削掉最后一个空格
                        if let Some(fixed_num) = num_part.strip_suffix(' ') {
                            formatted =
                                format!("{}{}{}", &fmt[..prefix_len], fixed_num, &fmt[dir_end..]);
                        }
                    } else {
                        // 右对齐：多余的填充字符在头部（但在符号之后），削掉一个字符并重排位置
                        let sign_char = num_part.chars().next().unwrap();
                        let rest = &num_part[1..];
                        if rest.starts_with(' ') || rest.starts_with('0') {
                            let fixed_rest = &rest[1..];
                            if has_zero {
                                // 零填充：将错误的内部空格替换为 0 (如 "+ 1" -> "+01")
                                let spaces_to_zeros = fixed_rest.replace(" ", "0");
                                let fixed_num = format!("{sign_char}{spaces_to_zeros}");
                                formatted = format!(
                                    "{}{}{}",
                                    &fmt[..prefix_len],
                                    fixed_num,
                                    &fmt[dir_end..]
                                );
                            } else {
                                // 空格填充：将空格推到符号外部 (如 "+  1" -> " +1")
                                let trimmed = fixed_rest.trim_start_matches(' ');
                                let spaces = " ".repeat(fixed_rest.len() - trimmed.len());
                                let fixed_num = format!("{spaces}{sign_char}{trimmed}");
                                formatted = format!(
                                    "{}{}{}",
                                    &fmt[..prefix_len],
                                    fixed_num,
                                    &fmt[dir_end..]
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    writer.write_all(formatted.as_bytes())
}

/// Floating point based code path
fn print_seq(range: RangeFloat, config: PrintConfig) -> std::io::Result<()> {
    let (first, increment, last) = range;
    let mut value = first;
    let padding = if config.pad {
        config.padding
            + if config.largest_dec > 0 {
                config.largest_dec + 1
            } else {
                0
            }
    } else {
        0
    };

    let mut writer: Box<dyn Write> = if let Some(buf) = config.buffer {
        Box::new(buf)
    } else {
        Box::new(stdout().lock())
    };

    let mut is_first_iteration = true;
    while !done_printing(&value, &increment, &last) {
        if !is_first_iteration {
            write!(writer, "{}", config.separator)?;
        }
        match config.format {
            Some(f) => {
                let float = match &value {
                    ExtendedBigDecimal::BigDecimal(bd) => bd.to_f64().unwrap(),
                    ExtendedBigDecimal::Infinity => f64::INFINITY,
                    ExtendedBigDecimal::MinusInfinity => f64::NEG_INFINITY,
                    ExtendedBigDecimal::MinusZero => -0.0,
                    ExtendedBigDecimal::Nan => f64::NAN,
                };
                format_with_zero_padding(&mut writer, f, float, config.format_str)?;
            }
            None => write_value_float(&mut writer, &value, padding, config.largest_dec)?,
        }
        value = value + increment.clone();
        is_first_iteration = false;
    }
    if !is_first_iteration {
        write!(writer, "{}", config.terminator)?;
    }
    writer.flush()
}

#[derive(Default)]
pub struct Seq;
impl Tool for Seq {
    fn name(&self) -> &'static str {
        "seq"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        seq_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Seq;

        // 测试 name 方法
        assert_eq!(tool.name(), "seq");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("seq"));

        // 测试 execute 方法
        let args = vec![OsString::from("seq"), OsString::from("1")];
        assert!(tool.execute(&args).is_ok());
    }

    #[test]
    fn test_seq_options_default() {
        let options = SeqOptions::default();
        assert_eq!(options.separator, "");
        assert_eq!(options.terminator, "");
        assert!(!options.is_equal_width);
        assert!(options.format.is_none());
    }

    #[test]
    fn test_seq_options_new() {
        let matches = ct_app()
            .try_get_matches_from(["seq", "-w", "-s", ",", "1", "10"])
            .unwrap();
        let options = SeqOptions::new(&matches);

        assert_eq!(options.separator, ",");
        assert_eq!(options.terminator, "\n");
        assert!(options.is_equal_width);
        assert!(options.format.is_none());
    }

    #[test]
    fn test_ct_app() {
        let mut app = ct_app();

        // 测试基本命令行参数
        assert!(app.get_arguments().any(|arg| arg.get_id() == SEQ_SEPARATOR));
        assert!(
            app.get_arguments()
                .any(|arg| arg.get_id() == SEQ_TERMINATOR)
        );
        assert!(
            app.get_arguments()
                .any(|arg| arg.get_id() == SEQ_EQUAL_WIDTH)
        );
        assert!(app.get_arguments().any(|arg| arg.get_id() == SEQ_FORMAT));

        // 测试帮助信息
        let help_text = app.render_help().to_string();
        assert!(help_text.contains("seq"));
    }

    #[test]
    fn test_done_printing() {
        // 测试正增量
        let result = done_printing(&1, &1, &5);
        assert!(!result, "Expected false for 1 < 5 with increment 1");

        let result = done_printing(&6, &1, &5);
        assert!(result, "Expected true for 6 > 5 with increment 1");

        // 测试负增量
        let result = done_printing(&5, &-1, &1);
        assert!(!result, "Expected false for 5 > 1 with increment -1");

        let result = done_printing(&0, &-1, &1);
        assert!(result, "Expected true for 0 < 1 with increment -1");

        // 测试零增量
        let result = done_printing(&1, &0, &1);
        assert!(!result, "Expected false for zero increment");
    }

    #[test]
    fn test_write_value_float() {
        // 测试普通数值
        let mut output = Vec::new();
        let value = "123.456".parse::<PreciseNumber>().unwrap().number;
        write_value_float(&mut output, &value, 8, 3).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "0123.456");

        // 测试无限值
        let mut output = Vec::new();
        write_value_float(&mut output, &ExtendedBigDecimal::Infinity, 8, 3).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "00000inf");
    }

    #[test]
    fn test_print_seq() {
        let mut output = Vec::new();

        // 测试基本序列
        let range = (
            "1".parse::<PreciseNumber>().unwrap().number,
            "1".parse::<PreciseNumber>().unwrap().number,
            "3".parse::<PreciseNumber>().unwrap().number,
        );
        print_seq(
            range,
            PrintConfig {
                largest_dec: 0,
                separator: ",",
                terminator: "\n",
                pad: false,
                padding: 1,
                format: &None,
                format_str: None,
                buffer: Some(&mut output),
            },
        )
        .unwrap();
        assert_eq!(String::from_utf8(output.clone()).unwrap(), "1,2,3\n");

        output.clear();

        // 测试等宽输出
        let range = (
            "1".parse::<PreciseNumber>().unwrap().number,
            "1".parse::<PreciseNumber>().unwrap().number,
            "10".parse::<PreciseNumber>().unwrap().number,
        );
        print_seq(
            range,
            PrintConfig {
                largest_dec: 0,
                separator: "\n",
                terminator: "\n",
                pad: true,
                padding: 2,
                format: &None,
                format_str: None,
                buffer: Some(&mut output),
            },
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output.clone()).unwrap(),
            "01\n02\n03\n04\n05\n06\n07\n08\n09\n10\n"
        );
    }

    #[test]
    fn test_seq_main() {
        // 测试格式化选项
        let result = seq_main(
            std::iter::once(OsString::from("seq"))
                .chain(["-w", "1", "3"].iter().map(|s| OsString::from(*s))),
        );
        assert!(result.is_ok());

        // 测试分隔符选项
        let result = seq_main(
            std::iter::once(OsString::from("seq"))
                .chain(["-s", ",", "1", "3"].iter().map(|s| OsString::from(*s))),
        );
        assert!(result.is_ok());
    }
}
