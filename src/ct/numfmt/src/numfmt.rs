/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

//! Linux 中处理数值的基本工具
//! Numfmt 命令将数字与人类可读格式相互转换。它读取各种表示形式的数字，并根据指定的选项以人类可读的格式重新格式化它们。
//! 如果没有给出数字，它将从标准输入中读取数字。

use std::io::{BufRead, Write};
use std::str::FromStr;

use clap::{crate_version, parser::ValueSource, Arg, ArgAction, ArgMatches, Command};

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTResult;
use ctcore::ct_format_usage;
use ctcore::ct_help_about;
use ctcore::ct_help_section;
use ctcore::ct_help_usage;
use ctcore::ct_ranges::CtRange;
use ctcore::ct_show;
use ctcore::ct_show_error;
use units::{NUMFMT_IEC_BASES, NUMFMT_SI_BASES};

use crate::errors::*;
use crate::flags::*;
use crate::format::numfmt_format_and_print;
use crate::units::{NumfmtUnit, Result};

pub mod errors;
pub mod flags;
pub mod format;
mod units;

const NUMFMT_ABOUT: &str = ct_help_about!("numfmt.md");
const NUMFMT_AFTER_HELP: &str = ct_help_section!("after help", "numfmt.md");
const NUMFMT_USAGE: &str = ct_help_usage!("numfmt.md");

pub mod numfmt_flags {
    pub const NUMFMT_DELIMITER: &str = "delimiter";
    pub const NUMFMT_FIELD: &str = "field";
    pub const NUMFMT_FIELD_DEFAULT: &str = "1";
    pub const NUMFMT_FORMAT: &str = "format";
    pub const NUMFMT_FROM: &str = "from";
    pub const NUMFMT_FROM_DEFAULT: &str = "none";
    pub const NUMFMT_FROM_UNIT: &str = "from-unit";
    pub const NUMFMT_FROM_UNIT_DEFAULT: &str = "1";
    pub const NUMFMT_HEADER: &str = "header";
    pub const NUMFMT_HEADER_DEFAULT: &str = "1";
    pub const NUMFMT_INVALID: &str = "invalid";
    pub const NUMFMT_NUMBER: &str = "NUMBER";
    pub const NUMFMT_PADDING: &str = "padding";
    pub const NUMFMT_ROUND: &str = "round";
    pub const NUMFMT_SUFFIX: &str = "suffix";
    pub const NUMFMT_TO: &str = "to";
    pub const NUMFMT_TO_DEFAULT: &str = "none";
    pub const NUMFMT_TO_UNIT: &str = "to-unit";
    pub const NUMFMT_TO_UNIT_DEFAULT: &str = "1";
}

fn numfmt_handle_args<'a>(
    args: impl Iterator<Item = &'a str>,
    numfmt_configs: &NumfmtConfigs,
) -> CTResult<()> {
    for l in args {
        numfmt_format_and_handle_validation(l, numfmt_configs)?;
    }
    Ok(())
}

fn numfmt_handle_buffer<R>(input: R, numfmt_configs: &NumfmtConfigs) -> CTResult<()>
where
    R: BufRead,
{
    for (idx, line_result) in input.lines().by_ref().enumerate() {
        match line_result {
            Ok(line) if idx < numfmt_configs.header => {
                println!("{line}");
                Ok(())
            }
            Ok(line) => numfmt_format_and_handle_validation(line.as_ref(), numfmt_configs),
            Err(err) => return Err(Box::new(NumfmtError::NumfmtIoError(err.to_string()))),
        }?;
    }
    Ok(())
}

fn numfmt_format_and_handle_validation(
    input_line: &str,
    numfmt_configs: &NumfmtConfigs,
) -> CTResult<()> {
    let handled_line = numfmt_format_and_print(input_line, numfmt_configs);

    if let Err(error_msg) = handled_line {
        match numfmt_configs.invalid {
            NumfmtInvalidModes::Abort => {
                return Err(Box::new(NumfmtError::NumfmtFormattingError(error_msg)));
            }
            NumfmtInvalidModes::Fail => {
                ct_show!(NumfmtError::NumfmtFormattingError(error_msg));
            }
            NumfmtInvalidModes::Warn => {
                ct_show_error!("{}", error_msg);
            }
            NumfmtInvalidModes::Ignore => {}
        };
        println!("{}", input_line);
    }

    Ok(())
}

fn numfmt_parse_unit(unit: &str) -> Result<NumfmtUnit> {
    match unit {
        "auto" => Ok(NumfmtUnit::Auto),
        "si" => Ok(NumfmtUnit::Si),
        "iec" => Ok(NumfmtUnit::Iec(false)),
        "iec-i" => Ok(NumfmtUnit::Iec(true)),
        "none" => Ok(NumfmtUnit::None),
        _ => Err("Unsupported unit is specified".to_owned()),
    }
}

// 解析单位大小。后缀被转换成整数表示。例如，"K
// 将返回 `Ok(1000)`，'2K'将返回 `Ok(2000)`。
fn numfmt_parse_unit_size(unit: &str) -> Result<usize> {
    let number: String = unit.chars().take_while(char::is_ascii_digit).collect();
    let suffix = &unit[number.len()..];

    if number.is_empty() || "0".repeat(number.len()) != number {
        if let Some(multiplier) = numfmt_parse_unit_size_suffix(suffix) {
            if number.is_empty() {
                return Ok(multiplier);
            }

            if let Ok(n) = number.parse::<usize>() {
                return Ok(n * multiplier);
            }
        }
    }

    Err(format!("invalid unit size: {}", unit.quote()))
}

// 解析单位大小的后缀并返回相应的乘数。例如
// 后缀 "K "将返回 "Some(1000)"，"Ki "将返回 "Some(1024)"。
//
// 如果后缀为空，则返回 `Some(1)`。
//
// 如果后缀未知，则返回 `None`。
fn numfmt_parse_unit_size_suffix(unit: &str) -> Option<usize> {
    if unit.is_empty() {
        return Some(1);
    }

    let unit_suffix = unit.chars().next().unwrap();

    if let Some(i) = ['K', 'M', 'G', 'T', 'P', 'E']
        .iter()
        .position(|&ch| ch == unit_suffix)
    {
        return match unit.len() {
            1 => Some(NUMFMT_SI_BASES[i + 1] as usize),
            2 if unit.ends_with('i') => Some(NUMFMT_IEC_BASES[i + 1] as usize),
            _ => None,
        };
    }

    None
}

fn numfmt_parse_options(arg_matches: &ArgMatches) -> Result<NumfmtConfigs> {
    let transform = parse_transform(arg_matches)?;
    let format = parse_format(arg_matches)?;
    if format.is_grouping && transform.to != NumfmtUnit::None {
        return Err("grouping cannot be combined with --to".to_string());
    }

    Ok(NumfmtConfigs {
        transform,
        padding: parse_padding(arg_matches)?,
        header: parse_header(arg_matches)?,
        fields: parse_fields(arg_matches)?,
        delimiter: parse_delimiter(arg_matches)?,
        round: parse_round(arg_matches),
        suffix: parse_suffix(arg_matches),
        format,
        invalid: parse_invalid(arg_matches),
    })
}

fn parse_format(arg_matches: &ArgMatches) -> Result<NumfmtFormatOptions> {
    let format = match arg_matches.get_one::<String>(numfmt_flags::NUMFMT_FORMAT) {
        Some(s) => s.parse()?,
        None => NumfmtFormatOptions::default(),
    };
    Ok(format)
}

fn parse_transform(arg_matches: &ArgMatches) -> Result<NumfmtTransformOptions> {
    let from = numfmt_parse_unit(
        arg_matches
            .get_one::<String>(numfmt_flags::NUMFMT_FROM)
            .unwrap(),
    )?;
    let to = numfmt_parse_unit(
        arg_matches
            .get_one::<String>(numfmt_flags::NUMFMT_TO)
            .unwrap(),
    )?;
    let from_unit = numfmt_parse_unit_size(
        arg_matches
            .get_one::<String>(numfmt_flags::NUMFMT_FROM_UNIT)
            .unwrap(),
    )?;
    let to_unit = numfmt_parse_unit_size(
        arg_matches
            .get_one::<String>(numfmt_flags::NUMFMT_TO_UNIT)
            .unwrap(),
    )?;

    let transform = NumfmtTransformOptions {
        from,
        from_unit,
        to,
        to_unit,
    };
    Ok(transform)
}

fn parse_padding(arg_matches: &ArgMatches) -> Result<isize> {
    let padding = match arg_matches.get_one::<String>(numfmt_flags::NUMFMT_PADDING) {
        Some(s) => s
            .parse::<isize>()
            .map_err(|_| s)
            .and_then(|n| match n {
                0 => Err(s),
                _ => Ok(n),
            })
            .map_err(|s| format!("invalid padding value {}", s.quote())),
        None => Ok(0),
    }?;
    Ok(padding)
}

fn parse_header(arg_matches: &ArgMatches) -> Result<usize> {
    let header = if arg_matches.value_source(numfmt_flags::NUMFMT_HEADER)
        == Some(ValueSource::CommandLine)
    {
        let value = arg_matches
            .get_one::<String>(numfmt_flags::NUMFMT_HEADER)
            .unwrap();

        value
            .parse::<usize>()
            .map_err(|_| value)
            .and_then(|n| match n {
                0 => Err(value),
                _ => Ok(n),
            })
            .map_err(|value| format!("invalid header value {}", value.quote()))
    } else {
        Ok(0)
    }?;
    Ok(header)
}

fn parse_fields(arg_matches: &ArgMatches) -> Result<Vec<CtRange>> {
    let fields = arg_matches
        .get_one::<String>(numfmt_flags::NUMFMT_FIELD)
        .unwrap()
        .as_str();
    // a lone "-" means "all fields", even as part of a list of fields
    let fields = if fields.split(&[',', ' ']).any(|x| x == "-") {
        vec![CtRange {
            low: 1,
            high: std::usize::MAX,
        }]
    } else {
        CtRange::from_list(fields)?
    };
    Ok(fields)
}

fn parse_delimiter(arg_matches: &ArgMatches) -> Result<Option<String>> {
    let delimiter = arg_matches
        .get_one::<String>(numfmt_flags::NUMFMT_DELIMITER)
        .map_or(Ok(None), |arg| {
            if arg.len() == 1 {
                Ok(Some(arg.to_string()))
            } else {
                Err("the delimiter must be a single character".to_string())
            }
        })?;
    Ok(delimiter)
}

fn parse_invalid(arg_matches: &ArgMatches) -> NumfmtInvalidModes {
    let invalid = NumfmtInvalidModes::from_str(
        arg_matches
            .get_one::<String>(numfmt_flags::NUMFMT_INVALID)
            .unwrap(),
    )
    .unwrap();
    invalid
}

fn parse_suffix(arg_matches: &ArgMatches) -> Option<String> {
    let suffix = arg_matches
        .get_one::<String>(numfmt_flags::NUMFMT_SUFFIX)
        .cloned();
    suffix
}

fn parse_round(arg_matches: &ArgMatches) -> NumfmtRoundMethod {
    // 因为参数有一个默认值，所以解包没有问题
    let round = match arg_matches
        .get_one::<String>(numfmt_flags::NUMFMT_ROUND)
        .unwrap()
        .as_str()
    {
        "up" => NumfmtRoundMethod::Up,
        "down" => NumfmtRoundMethod::Down,
        "from-zero" => NumfmtRoundMethod::FromZero,
        "towards-zero" => NumfmtRoundMethod::TowardsZero,
        "nearest" => NumfmtRoundMethod::Nearest,
        _ => unreachable!("Should be restricted by clap"),
    };
    round
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    numfmt_main(args)
}

pub fn numfmt_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;
    let options = numfmt_parse_options(&matches).map_err(NumfmtError::NumfmtIllegalArgument)?;

    let result = match matches.get_many::<String>(numfmt_flags::NUMFMT_NUMBER) {
        Some(values) => numfmt_handle_args(values.map(|s| s.as_str()), &options),
        None => {
            let stdin = std::io::stdin();
            let mut locked_stdin = stdin.lock();
            numfmt_handle_buffer(&mut locked_stdin, &options)
        }
    };

    if let Err(e) = result {
        std::io::stdout().flush().expect("error flushing stdout");
        Err(e)
    } else {
        Ok(())
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = NUMFMT_ABOUT;
    let usage_description = ct_format_usage(NUMFMT_USAGE);
    let args = vec![
        Arg::new(numfmt_flags::NUMFMT_DELIMITER)
            .short('d')
            .long(numfmt_flags::NUMFMT_DELIMITER)
            .value_name("X")
            .help("use X instead of whitespace for field delimiter"),
        Arg::new(numfmt_flags::NUMFMT_FIELD)
            .long(numfmt_flags::NUMFMT_FIELD)
            .help("replace the numbers in these input fields; see FIELDS below")
            .value_name("FIELDS")
            .allow_hyphen_values(true)
            .default_value(numfmt_flags::NUMFMT_FIELD_DEFAULT),
        Arg::new(numfmt_flags::NUMFMT_FORMAT)
            .long(numfmt_flags::NUMFMT_FORMAT)
            .help("use printf style floating-point FORMAT; see FORMAT below for details")
            .value_name("FORMAT")
            .allow_hyphen_values(true),
        Arg::new(numfmt_flags::NUMFMT_FROM)
            .long(numfmt_flags::NUMFMT_FROM)
            .help("auto-scale input numbers to UNITs; see UNIT below")
            .value_name("UNIT")
            .default_value(numfmt_flags::NUMFMT_FROM_DEFAULT),
        Arg::new(numfmt_flags::NUMFMT_FROM_UNIT)
            .long(numfmt_flags::NUMFMT_FROM_UNIT)
            .help("specify the input unit size")
            .value_name("N")
            .default_value(numfmt_flags::NUMFMT_FROM_UNIT_DEFAULT),
        Arg::new(numfmt_flags::NUMFMT_TO)
            .long(numfmt_flags::NUMFMT_TO)
            .help("auto-scale output numbers to UNITs; see UNIT below")
            .value_name("UNIT")
            .default_value(numfmt_flags::NUMFMT_TO_DEFAULT),
        Arg::new(numfmt_flags::NUMFMT_TO_UNIT)
            .long(numfmt_flags::NUMFMT_TO_UNIT)
            .help("the output unit size")
            .value_name("N")
            .default_value(numfmt_flags::NUMFMT_TO_UNIT_DEFAULT),
        Arg::new(numfmt_flags::NUMFMT_PADDING)
            .long(numfmt_flags::NUMFMT_PADDING)
            .help(
                "pad the output to N characters; positive N will \
                     right-align; negative N will left-align; padding is \
                     ignored if the output is wider than N; the default is \
                     to automatically pad if a whitespace is found",
            )
            .value_name("N"),
        Arg::new(numfmt_flags::NUMFMT_HEADER)
            .long(numfmt_flags::NUMFMT_HEADER)
            .help(
                "print (without converting) the first N header lines; \
                     N defaults to 1 if not specified",
            )
            .num_args(..=1)
            .value_name("N")
            .default_missing_value(numfmt_flags::NUMFMT_HEADER_DEFAULT)
            .hide_default_value(true),
        Arg::new(numfmt_flags::NUMFMT_ROUND)
            .long(numfmt_flags::NUMFMT_ROUND)
            .help("use METHOD for rounding when scaling")
            .value_name("METHOD")
            .default_value("from-zero")
            .value_parser(["up", "down", "from-zero", "towards-zero", "nearest"]),
        Arg::new(numfmt_flags::NUMFMT_SUFFIX)
            .long(numfmt_flags::NUMFMT_SUFFIX)
            .help(
                "print SUFFIX after each formatted number, and accept \
                    inputs optionally ending with SUFFIX",
            )
            .value_name("SUFFIX"),
        Arg::new(numfmt_flags::NUMFMT_INVALID)
            .long(numfmt_flags::NUMFMT_INVALID)
            .help("set the failure mode for invalid input")
            .default_value("abort")
            .value_parser(["abort", "fail", "warn", "ignore"])
            .value_name("INVALID"),
        Arg::new(numfmt_flags::NUMFMT_NUMBER)
            .hide(true)
            .action(ArgAction::Append),
    ];
    Command::new(utility_name)
        .version(command_version)
        .override_usage(usage_description)
        .about(application_info)
        .infer_long_args(true)
        .allow_negative_numbers(true)
        .args(args)
        .after_help(NUMFMT_AFTER_HELP)
}

