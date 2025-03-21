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
use std::io::{ErrorKind, Write, stdout};

use clap::{Arg, ArgAction, Command, crate_version};
use num_traits::{ToPrimitive, Zero};

use ctcore::ct_error::{CTError, CTResult, FromIo};
use ctcore::ct_format::{Format, num_format};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};

mod error;
mod extendedbigdecimal;
mod number;
mod numberparse;
use crate::error::SeqError;
use crate::extendedbigdecimal::ExtendedBigDecimal;
use crate::number::PreciseNumber;

const SEQ_ABOUT: &str = ct_help_about!("seq.md");
const SEQ_USAGE: &str = ct_help_usage!("seq.md");

const SEQ_SEPARATOR: &str = "separator";
const SEQ_TERMINATOR: &str = "terminator";
const SEQ_EQUAL_WIDTH: &str = "equal-width";
const SEQ_FORMAT: &str = "ct_format";

const SEQ_NUMBERS: &str = "numbers";

#[derive(Clone, Default)]
struct SeqOptions<'a> {
    separator: String,
    terminator: String,
    is_equal_width: bool,
    format: Option<&'a str>,
}

impl<'a> SeqOptions<'a> {
    fn new(matches: &'a clap::ArgMatches) -> Self {
        Self {
            separator: matches
                .get_one::<String>(SEQ_SEPARATOR)
                .map(|s| s.as_str())
                .unwrap_or("\n")
                .to_string(),
            terminator: matches
                .get_one::<String>(SEQ_TERMINATOR)
                .map(|s| s.as_str())
                .unwrap_or("\n")
                .to_string(),
            is_equal_width: matches.get_flag(SEQ_EQUAL_WIDTH),
            format: matches.get_one::<String>(SEQ_FORMAT).map(|s| s.as_str()),
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
    buffer: Option<&'a mut Vec<u8>>,
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    seq_main(args)
}

pub fn seq_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;
    let options = SeqOptions::new(&matches);

    let numbers = parse_number_args(&matches)?;
    let (first, increment, last) = get_sequence_range(&numbers)?;
    
    let padding = calculate_padding(&first, &increment, &last);
    let largest_dec = calculate_largest_decimal(&first, &increment);
    let format = parse_format_option(options.format)?;

    let config = PrintConfig {
        largest_dec,
        separator: &options.separator,
        terminator: &options.terminator,
        pad: options.is_equal_width,
        padding,
        format: &format,
        buffer: None,
    };

    match print_seq((first.number, increment.number, last.number), config) {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e.map_err_context(|| "write error".into())),
    }
}

fn parse_number_args(matches: &clap::ArgMatches) -> CTResult<Vec<String>> {
    Ok(matches
        .get_many::<String>(SEQ_NUMBERS)
        .ok_or(SeqError::NoArguments)?
        .map(ToString::to_string)
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
            .help("Separator character (defaults to \\n)"),
        Arg::new(SEQ_TERMINATOR)
            .short('t')
            .long("terminator")
            .help("Terminator character (defaults to \\n)"),
        Arg::new(SEQ_EQUAL_WIDTH)
            .short('w')
            .long("equal-width")
            .help("Equalize widths of all numbers by padding with zeros")
            .action(ArgAction::SetTrue),
        Arg::new(SEQ_FORMAT)
            .short('f')
            .long(SEQ_FORMAT)
            .help("use printf style floating-point FORMAT"),
        Arg::new(SEQ_NUMBERS)
            .action(ArgAction::Append)
            .num_args(1..=3),
    ];

    Command::new(ctcore::ct_util_name())
        .trailing_var_arg(true)
        .allow_negative_numbers(true)
        .infer_long_args(true)
        .version(crate_version!())
        .about(SEQ_ABOUT)
        .override_usage(ct_format_usage(SEQ_USAGE))
        .args(args)
}

fn done_printing<T: Zero + PartialOrd>(next: &T, increment: &T, last: &T) -> bool {
    if increment >= &T::zero() {
        next > last
    } else {
        next < last
    }
}

/// Write a big decimal formatted according to the given parameters.
fn write_value_float(
    writer: &mut impl Write,
    value: &ExtendedBigDecimal,
    width: usize,
    precision: usize,
) -> std::io::Result<()> {
    let value_as_str =
        if *value == ExtendedBigDecimal::Infinity || *value == ExtendedBigDecimal::MinusInfinity {
            format!("{value:>width$.precision$}")
        } else {
            format!("{value:>0width$.precision$}")
        };
    write!(writer, "{value_as_str}")
}

/// Floating point based code path
fn print_seq(
    range: RangeFloat,
    config: PrintConfig,
) -> std::io::Result<()> {
    let (first, increment, last) = range;
    let mut value = first;
    let padding = if config.pad {
        config.padding + if config.largest_dec > 0 { config.largest_dec + 1 } else { 0 }
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
                f.fmt(&mut writer, float)?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
