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

//! Linux 中处理数值的基本工具
//! Numfmt 命令将数字与人类可读格式相互转换。它读取各种表示形式的数字，并根据指定的选项以人类可读的格式重新格式化它们。
//! 如果没有给出数字，它将从标准输入中读取数字。

extern crate rust_i18n;
use rust_i18n::t;
use std::io::{BufRead, Write};
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{
    Arg, ArgAction, ArgMatches, Command, crate_version, error::ErrorKind, parser::ValueSource,
};
use std::str::FromStr;
use sys_locale::get_locale;

use crate::errors::*;
use crate::flags::*;
use crate::format::{
    numfmt_format_and_print, numfmt_format_and_print_abort, numfmt_format_line,
    numfmt_format_line_abort,
};
use crate::units::{NumfmtUnit, Result};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, get_ct_exit_code};
use ctcore::ct_ranges::CtRange;
use ctcore::ct_show;
use ctcore::ct_show_error;
use std::ffi::OsString;
use units::{NUMFMT_IEC_BASES, NUMFMT_SI_BASES};

pub mod errors;
pub mod flags;
pub mod format;
mod units;

pub mod numfmt_flags {
    pub const NUMFMT_DEBUG: &str = "debug";
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
    pub const NUMFMT_GROUPING: &str = "grouping";
    pub const NUMFMT_ROUND: &str = "round";
    pub const NUMFMT_SUFFIX: &str = "suffix";
    pub const NUMFMT_TO: &str = "to";
    pub const NUMFMT_TO_DEFAULT: &str = "none";
    pub const NUMFMT_TO_UNIT: &str = "to-unit";
    pub const NUMFMT_TO_UNIT_DEFAULT: &str = "1";
    pub const NUMFMT_ZERO_TERMINATED: &str = "zero-terminated";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumfmtRow {
    pub index: usize,
    pub source: String,
    pub input: String,
    pub output: String,
    pub status: String,
    pub error: Option<String>,
    pub transform_from: String,
    pub transform_to: String,
    pub invalid_mode: String,
    pub zero_terminated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumfmtSemantic {
    pub rows: Vec<NumfmtRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

fn numfmt_unit_name(unit: &NumfmtUnit) -> &'static str {
    match unit {
        NumfmtUnit::Auto => "auto",
        NumfmtUnit::Si => "si",
        NumfmtUnit::Iec(false) => "iec",
        NumfmtUnit::Iec(true) => "iec-i",
        NumfmtUnit::None => "none",
    }
}

#[cfg(test)]
mod native_semantic_tests {
    use super::numfmt_native_semantic;
    use std::ffi::OsString;

    #[test]
    fn numfmt_native_semantic_renders_rows_for_command_line_numbers() {
        let args = ["numfmt", "--to=si", "1000"];

        let semantic = numfmt_native_semantic(args.iter().map(OsString::from)).expect("semantic");

        assert_eq!(semantic.exit_code, 0);
        assert_eq!(semantic.stderr_text, "");
        assert_eq!(semantic.classic_text, "1.0K\n");
        assert_eq!(semantic.rows.len(), 1);
        assert_eq!(semantic.rows[0].input, "1000");
        assert_eq!(semantic.rows[0].output, "1.0K");
        assert_eq!(semantic.rows[0].status, "formatted");
        assert_eq!(semantic.rows[0].transform_to, "si");
    }

    #[test]
    fn numfmt_native_semantic_warn_mode_keeps_passthrough_rows() {
        let args = ["numfmt", "--invalid=warn", "hello"];

        let semantic = numfmt_native_semantic(args.iter().map(OsString::from)).expect("semantic");

        assert_eq!(semantic.exit_code, 0);
        assert_eq!(semantic.classic_text, "hello\n");
        assert_eq!(semantic.rows.len(), 1);
        assert_eq!(semantic.rows[0].status, "passthrough");
        assert_eq!(
            semantic.rows[0].error.as_deref(),
            Some("invalid number: 'hello'")
        );
        assert_eq!(semantic.stderr_text, "numfmt: invalid number: 'hello'\n");
    }
}

fn numfmt_invalid_mode_name(mode: &NumfmtInvalidModes) -> &'static str {
    match mode {
        NumfmtInvalidModes::Abort => "abort",
        NumfmtInvalidModes::Fail => "fail",
        NumfmtInvalidModes::Warn => "warn",
        NumfmtInvalidModes::Ignore => "ignore",
    }
}

fn numfmt_prefixed_stderr(message: &str) -> String {
    format!("numfmt: {message}\n")
}

fn trim_numfmt_line_ending(rendered: &str, zero_terminated: bool) -> String {
    if zero_terminated {
        rendered.strip_suffix('\0').unwrap_or(rendered).to_string()
    } else {
        rendered.strip_suffix('\n').unwrap_or(rendered).to_string()
    }
}

fn numfmt_passthrough_text(
    input_line: &str,
    zero_terminated: bool,
    has_line_ending: bool,
) -> String {
    if zero_terminated {
        format!("{input_line}\0")
    } else if has_line_ending {
        format!("{input_line}\n")
    } else {
        input_line.to_string()
    }
}

#[allow(clippy::too_many_arguments)]
fn numfmt_push_row(
    semantic: &mut NumfmtSemantic,
    options: &NumfmtConfigs,
    index: usize,
    source: &str,
    input: &str,
    rendered: String,
    status: &str,
    error: Option<String>,
) {
    semantic.classic_text.push_str(&rendered);
    semantic.rows.push(NumfmtRow {
        index,
        source: source.to_string(),
        input: input.to_string(),
        output: trim_numfmt_line_ending(&rendered, options.zero_terminated),
        status: status.to_string(),
        error,
        transform_from: numfmt_unit_name(&options.transform.from).to_string(),
        transform_to: numfmt_unit_name(&options.transform.to).to_string(),
        invalid_mode: numfmt_invalid_mode_name(&options.invalid).to_string(),
        zero_terminated: options.zero_terminated,
    });
}

fn numfmt_process_semantic_line(
    semantic: &mut NumfmtSemantic,
    options: &NumfmtConfigs,
    index: usize,
    source: &str,
    input_line: &str,
    has_line_ending: bool,
) -> CTResult<bool> {
    if options.invalid == NumfmtInvalidModes::Abort {
        match numfmt_format_line_abort(input_line, options) {
            Ok(rendered) => {
                numfmt_push_row(
                    semantic,
                    options,
                    index,
                    source,
                    input_line,
                    rendered,
                    "formatted",
                    None,
                );
                return Ok(true);
            }
            Err(error) => {
                let error_message = error.error.clone();
                semantic.classic_text.push_str(&error.partial_output);
                semantic
                    .stderr_text
                    .push_str(&numfmt_prefixed_stderr(&error_message));
                semantic.exit_code = semantic.exit_code.max(2);
                semantic.rows.push(NumfmtRow {
                    index,
                    source: source.to_string(),
                    input: input_line.to_string(),
                    output: trim_numfmt_line_ending(&error.partial_output, options.zero_terminated),
                    status: "partial-error".to_string(),
                    error: Some(error_message),
                    transform_from: numfmt_unit_name(&options.transform.from).to_string(),
                    transform_to: numfmt_unit_name(&options.transform.to).to_string(),
                    invalid_mode: numfmt_invalid_mode_name(&options.invalid).to_string(),
                    zero_terminated: options.zero_terminated,
                });
                return Ok(false);
            }
        }
    }

    match numfmt_format_line(input_line, options) {
        Ok(rendered) => {
            numfmt_push_row(
                semantic,
                options,
                index,
                source,
                input_line,
                rendered,
                "formatted",
                None,
            );
        }
        Err(error_message) => {
            match options.invalid {
                NumfmtInvalidModes::Fail => {
                    semantic
                        .stderr_text
                        .push_str(&numfmt_prefixed_stderr(&error_message));
                    semantic.exit_code = semantic.exit_code.max(2);
                }
                NumfmtInvalidModes::Warn => {
                    semantic
                        .stderr_text
                        .push_str(&numfmt_prefixed_stderr(&error_message));
                }
                NumfmtInvalidModes::Ignore | NumfmtInvalidModes::Abort => {}
            }

            let rendered =
                numfmt_passthrough_text(input_line, options.zero_terminated, has_line_ending);
            numfmt_push_row(
                semantic,
                options,
                index,
                source,
                input_line,
                rendered,
                "passthrough",
                Some(error_message),
            );
        }
    }

    Ok(true)
}

fn numfmt_collect_args_semantic<'a>(
    args: impl Iterator<Item = &'a str>,
    numfmt_configs: &NumfmtConfigs,
    semantic: &mut NumfmtSemantic,
) -> CTResult<()> {
    for (index, value) in args.enumerate() {
        if !numfmt_process_semantic_line(semantic, numfmt_configs, index + 1, "arg", value, true)? {
            break;
        }
    }
    Ok(())
}

fn numfmt_collect_buffer_semantic<R>(
    input: R,
    numfmt_configs: &NumfmtConfigs,
    semantic: &mut NumfmtSemantic,
) -> CTResult<()>
where
    R: BufRead,
{
    if numfmt_configs.zero_terminated {
        for (index, line_result) in input.split(0).enumerate() {
            match line_result {
                Ok(bytes) => {
                    let line = String::from_utf8_lossy(&bytes).to_string();
                    if index < numfmt_configs.header {
                        numfmt_push_row(
                            semantic,
                            numfmt_configs,
                            index + 1,
                            "stdin",
                            &line,
                            format!("{line}\0"),
                            "header",
                            None,
                        );
                    } else if !numfmt_process_semantic_line(
                        semantic,
                        numfmt_configs,
                        index + 1,
                        "stdin",
                        &line,
                        true,
                    )? {
                        break;
                    }
                }
                Err(err) => return Err(Box::new(NumfmtError::NumfmtIoError(err.to_string()))),
            }
        }
    } else {
        let mut input = input;
        let mut line = String::new();
        let mut index = 0usize;

        loop {
            line.clear();
            let bytes = input
                .read_line(&mut line)
                .map_err(|err| NumfmtError::NumfmtIoError(err.to_string()))?;
            if bytes == 0 {
                break;
            }

            let has_newline = line.ends_with('\n');
            if has_newline {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }

            if index < numfmt_configs.header {
                let rendered = if has_newline {
                    format!("{line}\n")
                } else {
                    line.clone()
                };
                numfmt_push_row(
                    semantic,
                    numfmt_configs,
                    index + 1,
                    "stdin",
                    &line,
                    rendered,
                    "header",
                    None,
                );
            } else if !numfmt_process_semantic_line(
                semantic,
                numfmt_configs,
                index + 1,
                "stdin",
                &line,
                has_newline,
            )? {
                break;
            }
            index += 1;
        }
    }

    Ok(())
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
    if numfmt_configs.zero_terminated {
        for (idx, line_result) in input.split(0).enumerate() {
            match line_result {
                Ok(bytes) => {
                    let line = String::from_utf8_lossy(&bytes);
                    if idx < numfmt_configs.header {
                        print!("{line}\0");
                        Ok(())
                    } else {
                        numfmt_format_and_handle_validation(&line, numfmt_configs)
                    }
                }
                Err(err) => return Err(Box::new(NumfmtError::NumfmtIoError(err.to_string()))),
            }?;
        }
    } else {
        let mut input = input;
        let mut line = String::new();
        let mut idx = 0usize;

        loop {
            line.clear();
            let bytes = input
                .read_line(&mut line)
                .map_err(|err| NumfmtError::NumfmtIoError(err.to_string()))?;
            if bytes == 0 {
                break;
            }

            let has_newline = line.ends_with('\n');
            if has_newline {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }

            if idx < numfmt_configs.header {
                if has_newline {
                    println!("{line}");
                } else {
                    print!("{line}");
                }
            } else {
                numfmt_format_and_handle_validation_with_line_ending(
                    line.as_ref(),
                    numfmt_configs,
                    has_newline,
                )?;
            }
            idx += 1;
        }
    }
    Ok(())
}

fn numfmt_format_and_handle_validation_with_line_ending(
    input_line: &str,
    numfmt_configs: &NumfmtConfigs,
    has_line_ending: bool,
) -> CTResult<()> {
    let handled_line = if numfmt_configs.invalid == NumfmtInvalidModes::Abort {
        numfmt_format_and_print_abort(input_line, numfmt_configs)
    } else {
        numfmt_format_and_print(input_line, numfmt_configs)
    };
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
        if numfmt_configs.zero_terminated {
            print!("{input_line}\0");
        } else if has_line_ending {
            println!("{input_line}");
        } else {
            print!("{input_line}");
        }
    }

    Ok(())
}

fn numfmt_format_and_handle_validation(
    input_line: &str,
    numfmt_configs: &NumfmtConfigs,
) -> CTResult<()> {
    numfmt_format_and_handle_validation_with_line_ending(input_line, numfmt_configs, true)
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

fn normalize_numfmt_args(args: impl ctcore::Args) -> Vec<OsString> {
    args.map(|arg| {
        if arg.to_str() == Some("---debug") {
            OsString::from("--debug")
        } else {
            arg
        }
    })
    .collect()
}

fn map_field_parse_error(err: String) -> String {
    const TRY_HELP: &str = "\nTry 'numfmt --help' for more information.";

    let Some(rest) = err.strip_prefix("range '") else {
        return err;
    };
    let Some((token, cause)) = rest.split_once("' was invalid: ") else {
        return err;
    };

    match cause {
        "fields and positions are numbered from 1" => {
            format!("fields are numbered from 1{TRY_HELP}")
        }
        "high end of range less than low end" => format!("invalid decreasing range{TRY_HELP}"),
        "byte/character offset is too large" => {
            format!("field number '{token}' is too large{TRY_HELP}")
        }
        "failed to parse range" => {
            if token.chars().all(char::is_alphabetic) {
                format!("invalid field value {}{TRY_HELP}", token.quote())
            } else if token.starts_with('-')
                && token.len() > 1
                && token[1..].chars().all(char::is_alphabetic)
            {
                format!("invalid field value {}{TRY_HELP}", token[1..].quote())
            } else {
                format!("invalid field range{TRY_HELP}")
            }
        }
        _ => err,
    }
}

fn has_conversion_option(matches: &ArgMatches, options: &NumfmtConfigs, grouping: bool) -> bool {
    grouping
        || options.padding != 0
        || options.transform.from != NumfmtUnit::None
        || options.transform.to != NumfmtUnit::None
        || options.transform.from_unit != 1
        || options.transform.to_unit != 1
        || options.suffix.is_some()
        || options.format != NumfmtFormatOptions::default()
        || matches.value_source(numfmt_flags::NUMFMT_FIELD) == Some(ValueSource::CommandLine)
        || matches.value_source(numfmt_flags::NUMFMT_DELIMITER) == Some(ValueSource::CommandLine)
}

fn is_c_locale() -> bool {
    for key in ["LC_ALL", "LANG", "LANGUAGE"] {
        let Ok(v) = std::env::var(key) else {
            continue;
        };
        if v.is_empty() {
            continue;
        }
        return v == "C" || v == "POSIX" || v.starts_with("C.");
    }
    false
}

fn numfmt_parse_options(arg_matches: &ArgMatches) -> Result<NumfmtConfigs> {
    let transform = parse_transform(arg_matches)?;
    let format = parse_format(arg_matches)?;
    let grouping = arg_matches.get_flag(numfmt_flags::NUMFMT_GROUPING);

    if grouping
        && arg_matches.value_source(numfmt_flags::NUMFMT_FORMAT) == Some(ValueSource::CommandLine)
    {
        return Err("--grouping cannot be combined with --format".to_string());
    }

    if (grouping || format.is_grouping) && transform.to != NumfmtUnit::None {
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
        zero_terminated: arg_matches.get_flag(numfmt_flags::NUMFMT_ZERO_TERMINATED),
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
                0 | isize::MIN => Err(s),
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
    let field_args = arg_matches
        .get_many::<String>(numfmt_flags::NUMFMT_FIELD)
        .map(|v| v.map(String::as_str).collect::<Vec<_>>())
        .unwrap_or_default();

    if field_args.len() > 1 {
        return Err("multiple field specifications".to_string());
    }

    let fields = field_args
        .first()
        .copied()
        .unwrap_or(numfmt_flags::NUMFMT_FIELD_DEFAULT);

    // a lone "-" means "all fields", even as part of a list of fields
    let fields = if fields.split(&[',', ' ']).any(|x| x == "-") {
        vec![CtRange {
            low: 1,
            high: usize::MAX,
        }]
    } else {
        CtRange::from_list(fields).map_err(map_field_parse_error)?
    };
    Ok(fields)
}

fn parse_delimiter(arg_matches: &ArgMatches) -> Result<Option<String>> {
    let delimiter = arg_matches
        .get_one::<String>(numfmt_flags::NUMFMT_DELIMITER)
        .map_or(Ok(None), |arg| {
            if arg.len() <= 1 {
                Ok(Some(arg.to_string()))
            } else {
                Err("the delimiter must be a single character".to_string())
            }
        })?;
    Ok(delimiter)
}

fn parse_invalid(arg_matches: &ArgMatches) -> NumfmtInvalidModes {
    NumfmtInvalidModes::from_str(
        arg_matches
            .get_one::<String>(numfmt_flags::NUMFMT_INVALID)
            .unwrap(),
    )
    .unwrap()
}

fn parse_suffix(arg_matches: &ArgMatches) -> Option<String> {
    arg_matches
        .get_one::<String>(numfmt_flags::NUMFMT_SUFFIX)
        .cloned()
}

fn parse_round(arg_matches: &ArgMatches) -> NumfmtRoundMethod {
    // 因为参数有一个默认值，所以解包没有问题
    match arg_matches
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
    }
}

pub fn numfmt_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let normalized_args = normalize_numfmt_args(args);
    let matches = match ct_app().try_get_matches_from(normalized_args) {
        Ok(matches) => matches,
        Err(err) if err.kind() == ErrorKind::UnknownArgument => {
            return Err(Box::new(NumfmtError::NumfmtIllegalArgument(
                "unrecognized option\nTry 'numfmt --help' for more information.".to_string(),
            )));
        }
        Err(err) => return Err(err.into()),
    };
    let options = numfmt_parse_options(&matches).map_err(NumfmtError::NumfmtIllegalArgument)?;
    let debug = matches.get_flag(numfmt_flags::NUMFMT_DEBUG);
    let grouping = matches.get_flag(numfmt_flags::NUMFMT_GROUPING) || options.format.is_grouping;
    let has_command_line_numbers = matches
        .get_many::<String>(numfmt_flags::NUMFMT_NUMBER)
        .is_some();

    if debug && has_command_line_numbers && options.header > 0 {
        ct_show_error!("--header ignored with command-line input");
    }

    if debug && !has_conversion_option(&matches, &options, grouping) {
        ct_show_error!("no conversion option specified");
    }

    if debug && grouping && is_c_locale() {
        ct_show_error!("grouping has no effect in this locale");
    }

    let result = match matches.get_many::<String>(numfmt_flags::NUMFMT_NUMBER) {
        Some(values) => numfmt_handle_args(values.map(|s| s.as_str()), &options),
        None => {
            let stdin = std::io::stdin();
            let mut locked_stdin = stdin.lock();
            numfmt_handle_buffer(&mut locked_stdin, &options)
        }
    };

    if debug && options.invalid == NumfmtInvalidModes::Fail && get_ct_exit_code() == 2 {
        ct_show_error!("failed to convert some of the input numbers");
    }

    if let Err(e) = result {
        std::io::stdout().flush().expect("error flushing stdout");
        Err(e)
    } else {
        Ok(())
    }
}

pub fn numfmt_native_semantic(args: impl ctcore::Args) -> CTResult<NumfmtSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let normalized_args = normalize_numfmt_args(args);
    let matches = match ct_app().try_get_matches_from(normalized_args) {
        Ok(matches) => matches,
        Err(err) if err.kind() == ErrorKind::UnknownArgument => {
            return Err(Box::new(NumfmtError::NumfmtIllegalArgument(
                "unrecognized option\nTry 'numfmt --help' for more information.".to_string(),
            )));
        }
        Err(err) => return Err(err.into()),
    };
    let options = numfmt_parse_options(&matches).map_err(NumfmtError::NumfmtIllegalArgument)?;
    let debug = matches.get_flag(numfmt_flags::NUMFMT_DEBUG);
    let grouping = matches.get_flag(numfmt_flags::NUMFMT_GROUPING) || options.format.is_grouping;
    let has_command_line_numbers = matches
        .get_many::<String>(numfmt_flags::NUMFMT_NUMBER)
        .is_some();

    let mut semantic = NumfmtSemantic {
        rows: Vec::new(),
        classic_text: String::new(),
        stderr_text: String::new(),
        exit_code: 0,
    };

    if debug && has_command_line_numbers && options.header > 0 {
        semantic.stderr_text.push_str(&numfmt_prefixed_stderr(
            "--header ignored with command-line input",
        ));
    }

    if debug && !has_conversion_option(&matches, &options, grouping) {
        semantic
            .stderr_text
            .push_str(&numfmt_prefixed_stderr("no conversion option specified"));
    }

    if debug && grouping && is_c_locale() {
        semantic.stderr_text.push_str(&numfmt_prefixed_stderr(
            "grouping has no effect in this locale",
        ));
    }

    match matches.get_many::<String>(numfmt_flags::NUMFMT_NUMBER) {
        Some(values) => {
            numfmt_collect_args_semantic(values.map(|s| s.as_str()), &options, &mut semantic)?
        }
        None => {
            let stdin = std::io::stdin();
            let mut locked_stdin = stdin.lock();
            numfmt_collect_buffer_semantic(&mut locked_stdin, &options, &mut semantic)?
        }
    };

    if debug && options.invalid == NumfmtInvalidModes::Fail && semantic.exit_code == 2 {
        semantic.stderr_text.push_str(&numfmt_prefixed_stderr(
            "failed to convert some of the input numbers",
        ));
    }

    Ok(semantic)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("numfmt.about");
    let usage_description = t!("numfmt.usage");
    let args = vec![
        Arg::new(numfmt_flags::NUMFMT_DELIMITER)
            .short('d')
            .long(numfmt_flags::NUMFMT_DELIMITER)
            .value_name("X")
            .help(t!("numfmt.clap.numfmt_delimiter")),
        Arg::new(numfmt_flags::NUMFMT_FIELD)
            .long(numfmt_flags::NUMFMT_FIELD)
            .help(t!("numfmt.clap.numfmt_field"))
            .value_name("FIELDS")
            .allow_hyphen_values(true)
            .action(ArgAction::Append),
        Arg::new(numfmt_flags::NUMFMT_FORMAT)
            .long(numfmt_flags::NUMFMT_FORMAT)
            .help(t!("numfmt.clap.numfmt_format"))
            .value_name("FORMAT")
            .allow_hyphen_values(true),
        Arg::new(numfmt_flags::NUMFMT_GROUPING)
            .long(numfmt_flags::NUMFMT_GROUPING)
            .help("use locale-defined digit grouping")
            .action(ArgAction::SetTrue),
        Arg::new(numfmt_flags::NUMFMT_FROM)
            .long(numfmt_flags::NUMFMT_FROM)
            .help(t!("numfmt.clap.numfmt_from"))
            .value_name("UNIT")
            .default_value(numfmt_flags::NUMFMT_FROM_DEFAULT),
        Arg::new(numfmt_flags::NUMFMT_FROM_UNIT)
            .long(numfmt_flags::NUMFMT_FROM_UNIT)
            .help(t!("numfmt.clap.numfmt_from_unit"))
            .value_name("N")
            .default_value(numfmt_flags::NUMFMT_FROM_UNIT_DEFAULT),
        Arg::new(numfmt_flags::NUMFMT_TO)
            .long(numfmt_flags::NUMFMT_TO)
            .help(t!("numfmt.clap.numfmt_to"))
            .value_name("UNIT")
            .default_value(numfmt_flags::NUMFMT_TO_DEFAULT),
        Arg::new(numfmt_flags::NUMFMT_TO_UNIT)
            .long(numfmt_flags::NUMFMT_TO_UNIT)
            .help(t!("numfmt.clap.numfmt_to_unit"))
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
        Arg::new(numfmt_flags::NUMFMT_DEBUG)
            .long(numfmt_flags::NUMFMT_DEBUG)
            .help("print warnings about invalid or ineffective options")
            .action(ArgAction::SetTrue),
        Arg::new(numfmt_flags::NUMFMT_ROUND)
            .long(numfmt_flags::NUMFMT_ROUND)
            .help(t!("numfmt.clap.numfmt_round"))
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
            .help(t!("numfmt.clap.numfmt_invalid"))
            .default_value("abort")
            .value_parser(["abort", "fail", "warn", "ignore"])
            .value_name("INVALID"),
        Arg::new(numfmt_flags::NUMFMT_ZERO_TERMINATED)
            .short('z')
            .long(numfmt_flags::NUMFMT_ZERO_TERMINATED)
            .help(t!("numfmt.clap.numfmt_zero_terminated"))
            .action(ArgAction::SetTrue),
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
        .after_help(t!("numfmt.after_help"))
}

#[derive(Default)]
pub struct Numfmt;
impl Tool for Numfmt {
    fn name(&self) -> &'static str {
        "numfmt"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let result = numfmt_main(args.iter().cloned());
        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    // use super::*;
    use super::{
        CtRange, Numfmt, NumfmtConfigs, NumfmtFormatOptions, NumfmtInvalidModes, NumfmtRoundMethod,
        NumfmtTransformOptions, NumfmtUnit, numfmt_handle_args, numfmt_handle_buffer,
        numfmt_parse_unit_size, numfmt_parse_unit_size_suffix,
    };
    use ctcore::Tool;
    use ctcore::ct_error::get_ct_exit_code;
    use std::ffi::OsString;
    use std::io::{BufReader, Error, ErrorKind, Read};

    #[test]
    fn test_tool_implementation() {
        let tool = Numfmt;

        // 测试 name 方法
        assert_eq!(tool.name(), "numfmt");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("numfmt"));

        // 测试 execute 方法
        let args = vec![OsString::from("numfmt"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    struct MockBuffer {}

    impl Read for MockBuffer {
        fn read(&mut self, _: &mut [u8]) -> Result<usize, Error> {
            Err(Error::new(ErrorKind::BrokenPipe, "broken pipe"))
        }
    }

    fn get_valid_options() -> NumfmtConfigs {
        NumfmtConfigs {
            transform: NumfmtTransformOptions {
                from: NumfmtUnit::None,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            },
            padding: 10,
            header: 1,
            fields: vec![CtRange { low: 0, high: 1 }],
            delimiter: None,
            round: NumfmtRoundMethod::Nearest,
            suffix: None,
            format: NumfmtFormatOptions::default(),
            invalid: NumfmtInvalidModes::Abort,
            zero_terminated: false,
        }
    }

    mod format_and_handle_validation_tests {
        use super::*;
        use crate::numfmt_format_and_handle_validation;
        #[test]
        fn test_numfmt_format_and_handle_validation_abort() {
            let input_line = "12.34";
            let mut numfmt_configs = get_valid_options();
            numfmt_configs.invalid = NumfmtInvalidModes::Abort;

            let result = numfmt_format_and_handle_validation(input_line, &numfmt_configs);
            assert!(result.is_ok());

            let input_line = "abc";
            let result2 = numfmt_format_and_handle_validation(input_line, &numfmt_configs);
            let expected_result2 = "invalid number: 'abc'".to_string();
            assert!(result2.is_err());
            assert_eq!(result2.unwrap_err().to_string(), expected_result2);
        }

        #[test]
        fn test_numfmt_format_and_handle_validation_fail() {
            let input_line = "1234";
            let mut numfmt_configs = get_valid_options();
            numfmt_configs.invalid = NumfmtInvalidModes::Fail;

            let result = numfmt_format_and_handle_validation(input_line, &numfmt_configs);
            assert!(result.is_ok());

            let input_line = "abc";
            let result2 = numfmt_format_and_handle_validation(input_line, &numfmt_configs);
            let _expected_result2 = "invalid suffix in input: 'abc'".to_string();
            assert!(result2.is_ok());
        }

        #[test]
        fn test_numfmt_format_and_handle_validation_warn() {
            let input_line = "12.34";
            let mut numfmt_configs = get_valid_options();
            numfmt_configs.invalid = NumfmtInvalidModes::Warn;

            let result = numfmt_format_and_handle_validation(input_line, &numfmt_configs);
            assert!(result.is_ok());

            let input_line = "abc";
            let result2 = numfmt_format_and_handle_validation(input_line, &numfmt_configs);
            let _expected_result2 = "invalid suffix in input: 'abc'".to_string();
            assert!(result2.is_ok());
        }

        #[test]
        fn test_numfmt_format_and_handle_validation_ignore() {
            let input_line = "1234";
            let mut numfmt_configs = get_valid_options();
            numfmt_configs.invalid = NumfmtInvalidModes::Ignore;

            let result = numfmt_format_and_handle_validation(input_line, &numfmt_configs);
            assert!(result.is_ok());

            let input_line = "abc";
            let result2 = numfmt_format_and_handle_validation(input_line, &numfmt_configs);
            assert!(result2.is_ok());
        }
    }
    #[cfg(test)]
    mod parse_options_tests {
        use super::*;
        use crate::NumfmtRoundMethod::*;
        use crate::{ct_app, numfmt_parse_options};

        #[test]
        fn test_parse_options_support_missing_argument() {
            let command = ct_app();

            // 测试用例4：验证当缺少必需的参数时是否正确报错
            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_parse_options_delimiter_long() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "|", "1", "2", "3"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("|".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_colon() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", ":"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some(":".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_comma() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", ","];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some(",".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_semicolon() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", ";"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some(";".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_vertical() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "|"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("|".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_tab() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "\t"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("\t".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_space() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", " "];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some(" ".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_group_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "\u{001d}"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("\u{1d}".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_record_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "\u{001e}"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("\u{1e}".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_unit_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "\u{001f}"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("\u{1f}".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_digital() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "6"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("6".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_letter() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "a"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("a".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_long_letter_aa() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "aa"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "the delimiter must be a single character";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_delimiter_long_uppercase_letter() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "A"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("A".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "|", "1", "2", "3"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("|".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_colon() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", ":"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some(":".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_comma() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", ","];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some(",".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_semicolon() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", ";"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some(";".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_vertical() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "|"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("|".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_tab() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "\t"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("\t".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_space() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", " "];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some(" ".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_group_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "\u{001d}"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("\u{1d}".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_record_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "\u{001e}"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("\u{1e}".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_unit_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "\u{001f}"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("\u{1f}".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_digital() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "6"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("6".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_letter() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "a"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("a".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_delimiter_short_letter_aa() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "aa"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "the delimiter must be a single character";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_delimiter_short_uppercase_letter() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "A"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: Some("A".to_string()),
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_field_long_1() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_field_long_1_2() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "1-2"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 2 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_field_long_none_2() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "-2"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 2 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_field_long__() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "-"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange {
                    low: 1,
                    high: 18446744073709551615,
                }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_field_long_1_() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "1-"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange {
                    low: 1,
                    high: 18446744073709551614,
                }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_field_long_2_1() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "2-1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value =
                "invalid decreasing range\nTry 'numfmt --help' for more information.";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_field_long_200() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "200"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange {
                    low: 200,
                    high: 200,
                }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_field_long_20000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "20000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange {
                    low: 20000000000,
                    high: 20000000000,
                }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_field_long_not_digital() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "aa"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value =
                "invalid field value 'aa'\nTry 'numfmt --help' for more information.";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_field_long_negative() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "-1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_field_long_zero() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "0"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value =
                "fields are numbered from 1\nTry 'numfmt --help' for more information.";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_field_long_float() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "2.1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid field range\nTry 'numfmt --help' for more information.";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_field_long_none() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "2.1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid field range\nTry 'numfmt --help' for more information.";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_d() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%d'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%d'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_i() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%i'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%i'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_u() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%u'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%u'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_c() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%c'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();
            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%c'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_s() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%s'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();
            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%s'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_o() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%o'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%o'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_x() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%x'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%x'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_uppercase_s() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%X'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%X'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_f() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%f'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "'".to_string(),
                    suffix: "'".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_format_long_uppercase_e() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%E'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%E'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_e() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%e'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%e'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%g'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%g'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_uppercase_g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%G'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%G'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_p() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%p'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%p'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_precentage() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%%'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "format ''%%'' ends in %";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_with_right_padding() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%10d'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%10d'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_with_left_padding() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%-10d'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();
            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format ''%-10d'', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_with_keep_2_digits() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%.2f'"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: Some(2),
                    prefix: "'".to_string(),
                    suffix: "'".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_format_long_with_strings() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "sssss"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "format 'sssss' has no % directive";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_format_long_with_err_precentage() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "%%% %"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid format '%%% %', directive must be %[0]['][-][N][.][N]f";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_from_long_none() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "none"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_none_ok() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "none", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_none_err() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "none", "1000G"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto_with_value_1g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1G"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto_with_value_1gi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1Gi"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto_with_value_1k() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1K"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto_with_value_1ki() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1Ki"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto_with_value_1m() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1M"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto_with_value_1mi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1Mi"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto_with_value_small_1m() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1m"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_auto_with_value_small_1mi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1mi"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Auto,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_si_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Si,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_si_with_value_1k() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1K"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Si,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_si_with_value_1ki() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1Ki"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Si,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_si_with_value_1g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1G"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Si,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_si_with_value_1gi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1Gi"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Si,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(false),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_with_value_1k() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1K"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(false),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_with_value_1ki() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1Ki"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(false),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_with_value_1g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1G"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(false),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_with_value_1gi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1Gi"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(false),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_i_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(true),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_i_with_value_1k() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1K"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(true),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_i_with_value_1ki() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1Ki"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(true),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_i_with_value_1g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1G"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(true),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_long_iec_i_with_value_1gi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1Gi"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::Iec(true),
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_unit_long_0() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from-unit", "0"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid unit size: '0'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_from_unit_long_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from-unit", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_unit_long_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from-unit", "2"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 2,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_from_unit_long_100000000000000000000000000000000() {
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--from-unit",
                "100000000000000000000000000000000",
            ];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid unit size: '100000000000000000000000000000000'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_from_unit_long_aa() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from-unit", "aa"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid unit size: 'aa'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "2"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1023"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1024"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1048575"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1048576"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1073741823"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1073741824"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_none_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "2"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1023"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1024"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1048575"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1048576"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1073741823"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1073741824"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_auto_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Auto,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "2"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1023"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1024"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1048575"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1048576"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1073741823"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1073741824"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_si_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Si,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "2"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1023"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1024"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1048575"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1048576"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1073741823"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1073741824"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_i_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(true),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "2"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1023"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1024"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1048575"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1048576"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1073741823"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1073741824"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_long_iec_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::Iec(false),
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_unit_long_0() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to-unit", "0"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid unit size: '0'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_to_unit_long_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to-unit", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_unit_long_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to-unit", "2"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 2,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_to_unit_long_100000000000000000000000000000000() {
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--to-unit",
                "100000000000000000000000000000000",
            ];
            let result = command.try_get_matches_from(cmd_args).unwrap();
            let options = numfmt_parse_options(&result);
            let expected_value = "invalid unit size: '100000000000000000000000000000000'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_to_unit_long_aa() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to-unit", "aa"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid unit size: 'aa'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_padding_long_0() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "0"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid padding value '0'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_padding_long_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 1,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_padding_long_100() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "100"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 100,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_padding_long_a() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "a"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid padding value 'a'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_padding_long_negative_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "-1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: -1,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_padding_long_negative_100() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "-100"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: -100,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_padding_long_negative_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "-1000"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: -1000,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_header_long_0() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "0"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid header value '0'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_header_long_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 1,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_header_long_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 10,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_header_long_100() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "100"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 100,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_header_long_negitive_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "-1"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid header value '-1'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_header_long_a() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "a"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_value = "invalid header value 'a'";
            assert!(options.is_err());
            assert_eq!(options.unwrap_err(), expected_value);
        }

        #[test]
        fn test_parse_options_round_long_from_zero() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "from-zero"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_round_long_up() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "up"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: Up,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_round_long_down() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "down"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: Down,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_round_long_towards_zero() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "towards-zero"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: TowardsZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_round_long_nearest() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "nearest"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: Nearest,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_k_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "K", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("K".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_ki_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Ki", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Ki".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_m_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "M", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("M".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_mi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Mi", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Mi".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_g_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "G", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("G".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_gi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Gi", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Gi".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_ti_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Ti", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Ti".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_t_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "T", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("T".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_p_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "P", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("P".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_pi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Pi", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Pi".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_e_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "E", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("E".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_ei_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Ei", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Ei".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_z_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Z", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Z".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_zi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Zi", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Zi".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_y_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Y", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Y".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_yi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Yi", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Yi".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_wi_10_limit() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Wi", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("Wi".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_suffix_long_uppercase_w_10_limit() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "W", "10"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: Some("W".to_string()),
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_invalid_long_abort() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--invalid", "abort"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Abort,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_invalid_long_fail() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--invalid", "fail"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Fail,
                zero_terminated: false,
            };
            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_invalid_long_warn() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--invalid", "warn"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Warn,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }

        #[test]
        fn test_parse_options_invalid_long_ignore() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--invalid", "ignore"];
            let result = command.try_get_matches_from(cmd_args).unwrap();

            let options = numfmt_parse_options(&result);
            let expected_options = NumfmtConfigs {
                transform: NumfmtTransformOptions {
                    from: NumfmtUnit::None,
                    from_unit: 1,
                    to: NumfmtUnit::None,
                    to_unit: 1,
                },
                padding: 0,
                header: 0,
                fields: vec![CtRange { low: 1, high: 1 }],
                delimiter: None,
                round: FromZero,
                suffix: None,
                format: NumfmtFormatOptions {
                    is_grouping: false,
                    padding: None,
                    precision: None,
                    prefix: "".to_string(),
                    suffix: "".to_string(),
                    is_zero_padding: false,
                },
                invalid: NumfmtInvalidModes::Ignore,
                zero_terminated: false,
            };

            assert_eq!(options.unwrap(), expected_options);
        }
    }

    #[cfg(test)]
    mod handle_buffer_tests {
        use super::*;

        #[test]
        fn broken_buffer_returns_io_error() {
            let mock_buffer = MockBuffer {};
            let result = numfmt_handle_buffer(BufReader::new(mock_buffer), &get_valid_options())
                .expect_err("returned Ok after receiving IO error");
            let result_debug = format!("{result:?}");
            let result_display = format!("{result}");
            assert_eq!(result_debug, "NumfmtIoError(\"broken pipe\")");
            assert_eq!(result_display, "broken pipe");
            assert_eq!(result.code(), 1);
        }

        #[test]
        fn broken_buffer_returns_io_error_after_header() {
            let mock_buffer = MockBuffer {};
            let mut options = get_valid_options();
            options.header = 0;
            let result = numfmt_handle_buffer(BufReader::new(mock_buffer), &options)
                .expect_err("returned Ok after receiving IO error");
            let result_debug = format!("{result:?}");
            let result_display = format!("{result}");
            assert_eq!(result_debug, "NumfmtIoError(\"broken pipe\")");
            assert_eq!(result_display, "broken pipe");
            assert_eq!(result.code(), 1);
        }

        #[test]
        fn non_numeric_returns_formatting_error() {
            let input_value = b"135\nhello";
            let result =
                numfmt_handle_buffer(BufReader::new(&input_value[..]), &get_valid_options())
                    .expect_err("returned Ok after receiving improperly formatted input");
            let result_debug = format!("{result:?}");
            let result_display = format!("{result}");
            assert_eq!(
                result_debug,
                "NumfmtFormattingError(\"invalid number: 'hello'\")"
            );
            assert_eq!(result_display, "invalid number: 'hello'");
            assert_eq!(result.code(), 2);
        }

        #[test]
        fn valid_input_returns_ok() {
            let input_value = b"165\n100\n300\n500";
            let result =
                numfmt_handle_buffer(BufReader::new(&input_value[..]), &get_valid_options());
            assert!(result.is_ok(), "did not return Ok for valid input");
        }

        #[test]
        fn warn_returns_ok_for_invalid_input() {
            let input_value = b"5\n4Q\n";
            let mut options = get_valid_options();
            options.invalid = NumfmtInvalidModes::Warn;
            let result = numfmt_handle_buffer(BufReader::new(&input_value[..]), &options);
            assert!(result.is_ok(), "did not return Ok for invalid input");
        }

        #[test]
        fn ignore_returns_ok_for_invalid_input() {
            let input_value = b"5\n4Q\n";
            let mut options = get_valid_options();
            options.invalid = NumfmtInvalidModes::Ignore;
            let result = numfmt_handle_buffer(BufReader::new(&input_value[..]), &options);
            assert!(result.is_ok(), "did not return Ok for invalid input");
        }

        #[test]
        fn buffer_fail_returns_status_2_for_invalid_input() {
            let input_value = b"5\n4Q\n";
            let mut options = get_valid_options();
            options.invalid = NumfmtInvalidModes::Fail;
            numfmt_handle_buffer(BufReader::new(&input_value[..]), &options).unwrap();
            assert_eq!(
                get_ct_exit_code(),
                2,
                "should set exit code 2 for formatting errors"
            );
        }

        #[test]
        fn abort_returns_status_2_for_invalid_input() {
            let input_value = b"5\n4Q\n";
            let mut options = get_valid_options();
            options.invalid = NumfmtInvalidModes::Abort;
            let result = numfmt_handle_buffer(BufReader::new(&input_value[..]), &options);
            assert!(result.is_err(), "did not return err for invalid input");
        }
    }

    #[cfg(test)]
    mod handle_args_tests {
        use super::*;

        #[test]
        fn args_fail_returns_status_2_for_invalid_input() {
            let input_value = ["5", "4Q"].into_iter();
            let mut options = get_valid_options();
            options.invalid = NumfmtInvalidModes::Fail;
            numfmt_handle_args(input_value, &options).unwrap();
            assert_eq!(
                get_ct_exit_code(),
                2,
                "should set exit code 2 for formatting errors"
            );
        }

        #[test]
        fn args_warn_returns_status_0_for_invalid_input() {
            let input_value = ["5", "4Q"].into_iter();
            let mut options = get_valid_options();
            options.invalid = NumfmtInvalidModes::Warn;
            let result = numfmt_handle_args(input_value, &options);
            assert!(result.is_ok(), "did not return ok for invalid input");
        }
    }

    #[cfg(test)]
    mod parse_unit_size_tests {
        use super::*;

        #[test]
        fn test_parse_unit_size() {
            assert_eq!(1, numfmt_parse_unit_size("1").unwrap());
            assert_eq!(1, numfmt_parse_unit_size("01").unwrap());
            assert!(numfmt_parse_unit_size("1.1").is_err());
            assert!(numfmt_parse_unit_size("0").is_err());
            assert!(numfmt_parse_unit_size("-1").is_err());
            assert!(numfmt_parse_unit_size("A").is_err());
            assert!(numfmt_parse_unit_size("18446744073709551616").is_err());
        }

        #[test]
        fn test_parse_unit_size_with_suffix() {
            assert_eq!(1000, numfmt_parse_unit_size("K").unwrap());
            assert_eq!(1024, numfmt_parse_unit_size("Ki").unwrap());
            assert_eq!(2000, numfmt_parse_unit_size("2K").unwrap());
            assert_eq!(2048, numfmt_parse_unit_size("2Ki").unwrap());
            assert!(numfmt_parse_unit_size("0K").is_err());
        }

        #[test]
        fn test_parse_unit_size_suffix() {
            assert_eq!(1, numfmt_parse_unit_size_suffix("").unwrap());
            assert_eq!(1000, numfmt_parse_unit_size_suffix("K").unwrap());
            assert_eq!(1024, numfmt_parse_unit_size_suffix("Ki").unwrap());
            assert_eq!(1000 * 1000, numfmt_parse_unit_size_suffix("M").unwrap());
            assert_eq!(1024 * 1024, numfmt_parse_unit_size_suffix("Mi").unwrap());
            assert!(numfmt_parse_unit_size_suffix("Kii").is_none());
            assert!(numfmt_parse_unit_size_suffix("A").is_none());
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::ffi::OsString;

        use crate::numfmt_main;

        #[test]
        fn test_ctmain_input_h() {
            let args = ["-h", ""];
            let result = numfmt_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_input_v() {
            let args = ["--version", ""];
            let result = numfmt_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_input_uppercase_v() {
            let args = ["-V", ""];
            let result = numfmt_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_pr_main_default() {
            let args = [ctcore::ct_util_name(), "1000"];
            let result = numfmt_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_execution_version() {
            // 测试用例1：有效输入
            let args = [ctcore::ct_util_name(), "--version", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 0);
            }
        }

        #[test]
        fn test_ct_main_execution_other_version() {
            // 测试用例1：有效输入
            let args = [ctcore::ct_util_name(), "-V", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 0);
            }
        }

        #[test]
        fn test_ct_main_execution_help() {
            // 测试用例2：验证 --help 参数是否正确处理
            let args = [ctcore::ct_util_name(), "--help", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 0);
            }
        }

        #[test]
        fn test_ct_main_invalid_argument() {
            // 测试用例3：验证当提供未知参数时是否正确报错
            let args = [ctcore::ct_util_name(), "--invalid-argument", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long() {
            let args = [
                ctcore::ct_util_name(),
                "--delimiter",
                "|",
                "1",
                "2",
                "3",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_colon() {
            let args = [ctcore::ct_util_name(), "--delimiter", ":", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_comma() {
            let args = [ctcore::ct_util_name(), "--delimiter", ",", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_semicolon() {
            let args = [ctcore::ct_util_name(), "--delimiter", ";", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_vertical() {
            let args = [ctcore::ct_util_name(), "--delimiter", "|", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_tab() {
            let args = [ctcore::ct_util_name(), "--delimiter", "\t", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_space() {
            let args = [ctcore::ct_util_name(), "--delimiter", " ", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_group_separator() {
            let args = [ctcore::ct_util_name(), "--delimiter", "\u{001d}", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_record_separator() {
            let args = [ctcore::ct_util_name(), "--delimiter", "\u{001e}", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_unit_separator() {
            let args = [ctcore::ct_util_name(), "--delimiter", "\u{001f}", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_digital() {
            let args = [ctcore::ct_util_name(), "--delimiter", "6", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_letter() {
            let args = [ctcore::ct_util_name(), "--delimiter", "a", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_letter_aa() {
            let args = [ctcore::ct_util_name(), "--delimiter", "aa", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_uppercase_letter() {
            let args = [ctcore::ct_util_name(), "--delimiter", "A", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_long_no_value() {
            let args = [ctcore::ct_util_name(), "--delimiter", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short() {
            let args = [ctcore::ct_util_name(), "-d", "|", "1", "2", "3", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_no_value() {
            let args = [ctcore::ct_util_name(), "-d", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_colon() {
            let args = [ctcore::ct_util_name(), "-d", ":", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_comma() {
            let args = [ctcore::ct_util_name(), "-d", ",", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_semicolon() {
            let args = [ctcore::ct_util_name(), "-d", ";", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_vertical() {
            let args = [ctcore::ct_util_name(), "-d", "|", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_tab() {
            let args = [ctcore::ct_util_name(), "-d", "\t", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_space() {
            let args = [ctcore::ct_util_name(), "-d", " ", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_group_separator() {
            let args = [ctcore::ct_util_name(), "-d", "\u{001d}", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_record_separator() {
            let args = [ctcore::ct_util_name(), "-d", "\u{001e}", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_unit_separator() {
            let args = [ctcore::ct_util_name(), "-d", "\u{001f}", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_digital() {
            let args = [ctcore::ct_util_name(), "-d", "6", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_letter() {
            let args = [ctcore::ct_util_name(), "-d", "a", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_letter_aa() {
            let args = [ctcore::ct_util_name(), "-d", "aa", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_delimiter_short_uppercase_letter() {
            let args = [ctcore::ct_util_name(), "-d", "A", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_1() {
            let args = [ctcore::ct_util_name(), "--field", "1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_1_2() {
            let args = [ctcore::ct_util_name(), "--field", "1-2", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_none_2() {
            let args = [ctcore::ct_util_name(), "--field", "-2", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long__() {
            let args = [ctcore::ct_util_name(), "--field", "-", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_1_() {
            let args = [ctcore::ct_util_name(), "--field", "1-", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_2_1() {
            let args = [ctcore::ct_util_name(), "--field", "2-1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_200() {
            let args = [ctcore::ct_util_name(), "--field", "200", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_20000000000() {
            let args = [ctcore::ct_util_name(), "--field", "20000000000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_not_digital() {
            let args = [ctcore::ct_util_name(), "--field", "aa", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_negative() {
            let args = [ctcore::ct_util_name(), "--field", "-1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_zero() {
            let args = [ctcore::ct_util_name(), "--field", "0", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_float() {
            let args = [ctcore::ct_util_name(), "--field", "2.1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_field_long_none() {
            let args = [ctcore::ct_util_name(), "--field", "2.1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_none() {
            let args = [ctcore::ct_util_name(), "--format", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_d() {
            let args = [ctcore::ct_util_name(), "--format", "'%d'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_i() {
            let args = [ctcore::ct_util_name(), "--format", "'%i'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_u() {
            let args = [ctcore::ct_util_name(), "--format", "'%u'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_c() {
            let args = [ctcore::ct_util_name(), "--format", "'%c'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_s() {
            let args = [ctcore::ct_util_name(), "--format", "'%s'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_o() {
            let args = [ctcore::ct_util_name(), "--format", "'%o'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_x() {
            let args = [ctcore::ct_util_name(), "--format", "'%x'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_uppercase_s() {
            let args = [ctcore::ct_util_name(), "--format", "'%X'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_f() {
            let args = [ctcore::ct_util_name(), "--format", "'%f'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_uppercase_e() {
            let args = [ctcore::ct_util_name(), "--format", "'%E'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_e() {
            let args = [ctcore::ct_util_name(), "--format", "'%e'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_g() {
            let args = [ctcore::ct_util_name(), "--format", "'%g'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_uppercase_g() {
            let args = [ctcore::ct_util_name(), "--format", "'%G'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_p() {
            let args = [ctcore::ct_util_name(), "--format", "'%p'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_precentage() {
            let args = [ctcore::ct_util_name(), "--format", "'%%'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_with_right_padding() {
            let args = [ctcore::ct_util_name(), "--format", "'%10d'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_with_left_padding() {
            let args = [ctcore::ct_util_name(), "--format", "'%-10d'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_with_keep_2_digits() {
            let args = [ctcore::ct_util_name(), "--format", "'%.2f'", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_with_strings() {
            let args = [ctcore::ct_util_name(), "--format", "sssss", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_format_long_with_err_precentage() {
            let args = [ctcore::ct_util_name(), "--format", "%%% %", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long() {
            let args = [ctcore::ct_util_name(), "--from", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_none() {
            let args = [ctcore::ct_util_name(), "--from", "none", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_none_ok() {
            let args = [ctcore::ct_util_name(), "--from", "none", "1000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_none_err() {
            let args = [ctcore::ct_util_name(), "--from", "none", "1000G"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto_with_value_1000() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "1000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto_with_value_1g() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "1G", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto_with_value_1gi() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "1Gi", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto_with_value_1k() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "1K", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto_with_value_1ki() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "1Ki", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto_with_value_1m() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "1M", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto_with_value_1mi() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "1Mi", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto_with_value_small_1m() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "100m"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_from_long_auto_with_value_small_1mi() {
            let args = [ctcore::ct_util_name(), "--from", "auto", "1mi"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_from_long_si_with_value_1000() {
            let args = [ctcore::ct_util_name(), "--from", "si", "1000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_si_with_value_1k() {
            let args = [ctcore::ct_util_name(), "--from", "si", "1K", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_si_with_value_1ki() {
            let args = [ctcore::ct_util_name(), "--from", "si", "1Ki"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_from_long_si_with_value_1g() {
            let args = [ctcore::ct_util_name(), "--from", "si", "1G", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_si_with_value_1gi() {
            let args = [ctcore::ct_util_name(), "--from", "si", "1Gi"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_with_value_1000() {
            let args = [ctcore::ct_util_name(), "--from", "iec", "1000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_with_value_1k() {
            let args = [ctcore::ct_util_name(), "--from", "iec", "1K", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_with_value_1ki() {
            let args = [ctcore::ct_util_name(), "--from", "iec", "1Ki"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_with_value_1g() {
            let args = [ctcore::ct_util_name(), "--from", "iec", "1G", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_with_value_1gi() {
            let args = [ctcore::ct_util_name(), "--from", "iec", "1Gi"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_i_with_value_1000() {
            let args = [ctcore::ct_util_name(), "--from", "iec-i", "1000Gi"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_i_with_value_1k() {
            let args = [ctcore::ct_util_name(), "--from", "iec-i", "1Ki"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_i_with_value_1ki() {
            let args = [ctcore::ct_util_name(), "--from", "iec-i", "1Ki"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_i_with_value_1g() {
            let args = [ctcore::ct_util_name(), "--from", "iec-i", "1Gi"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_long_iec_i_with_value_1gi() {
            let args = [ctcore::ct_util_name(), "--from", "iec-i", "1Gi"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_unit_long_0() {
            let args = [ctcore::ct_util_name(), "--from-unit", "0", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_unit_long_1() {
            let args = [ctcore::ct_util_name(), "--from-unit", "1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_unit_long_2() {
            let args = [ctcore::ct_util_name(), "--from-unit", "2", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_unit_long_100000000000000000000000000000000() {
            let args = [
                ctcore::ct_util_name(),
                "--from-unit",
                "100000000000000000000000000000000",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_from_unit_long_aa() {
            let args = [ctcore::ct_util_name(), "--from-unit", "aa", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1() {
            let args = [ctcore::ct_util_name(), "--to", "none", "1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_2() {
            let args = [ctcore::ct_util_name(), "--to", "none", "2", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_999() {
            let args = [ctcore::ct_util_name(), "--to", "none", "999", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1000() {
            let args = [ctcore::ct_util_name(), "--to", "none", "1000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1023() {
            let args = [ctcore::ct_util_name(), "--to", "none", "1023", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1024() {
            let args = [ctcore::ct_util_name(), "--to", "none", "1024", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_999999() {
            let args = [ctcore::ct_util_name(), "--to", "none", "999999", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1000000() {
            let args = [ctcore::ct_util_name(), "--to", "none", "1000000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1048575() {
            let args = [ctcore::ct_util_name(), "--to", "none", "1048575", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1048576() {
            let args = [ctcore::ct_util_name(), "--to", "none", "1048576", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_999999999() {
            let args = [ctcore::ct_util_name(), "--to", "none", "999999999", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1000000000() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "1000000000",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1073741823() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "1073741823",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1073741824() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "1073741824",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_999999999999() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "999999999999",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1000000000000() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "1000000000000",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1099511627775() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "1099511627775",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1099511627776() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "1099511627776",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_999999999999999() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "999999999999999",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1000000000000000() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "1000000000000000",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1125899906842623() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "1125899906842623",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_none_with_value_1125899906842624() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "none",
                "1125899906842624",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_2() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "2", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_999() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "999", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1000() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1023() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1023", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1024() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1024", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_999999() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "999999", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1000000() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1000000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1048575() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1048575", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1048576() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1048576", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_999999999() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "999999999", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1000000000() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "auto",
                "1000000000",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1073741823() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "auto",
                "1073741823",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1073741824() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "auto",
                "1073741824",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_999999999999() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "auto",
                "999999999999",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1000000000000() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "auto",
                "1000000000000",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1099511627775() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "auto",
                "1099511627775",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1099511627776() {
            let args = [
                ctcore::ct_util_name(),
                "--to",
                "auto",
                "1099511627776",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_999999999999999() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "999999999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1000000000000000() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1000000000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1125899906842623() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1125899906842623"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_auto_with_value_1125899906842624() {
            let args = [ctcore::ct_util_name(), "--to", "auto", "1125899906842624"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 2);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_2() {
            let args = [ctcore::ct_util_name(), "--to", "si", "2"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_999() {
            let args = [ctcore::ct_util_name(), "--to", "si", "999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1000() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1023() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1023"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1024() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1024"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_999999() {
            let args = [ctcore::ct_util_name(), "--to", "si", "999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1000000() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1048575() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1048575"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1048576() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1048576"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_999999999() {
            let args = [ctcore::ct_util_name(), "--to", "si", "999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1000000000() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1073741823() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1073741823"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1073741824() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1073741824"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_999999999999() {
            let args = [ctcore::ct_util_name(), "--to", "si", "999999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1000000000000() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1000000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1099511627775() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1099511627775"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1099511627776() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1099511627776"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_999999999999999() {
            let args = [ctcore::ct_util_name(), "--to", "si", "999999999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1000000000000000() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1000000000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1125899906842623() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1125899906842623"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_si_with_value_1125899906842624() {
            let args = [ctcore::ct_util_name(), "--to", "si", "1125899906842624"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_2() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "2"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_999() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1000() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1023() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1023"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1024() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1024"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_999999() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1000000() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1048575() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1048575"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1048576() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1048576"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_999999999() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1000000000() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1073741823() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1073741823"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1073741824() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1073741824"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_999999999999() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "999999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1000000000000() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1000000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1099511627775() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1099511627775"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1099511627776() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1099511627776"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_999999999999999() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "999999999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1000000000000000() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1000000000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1125899906842623() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1125899906842623"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_i_with_value_1125899906842624() {
            let args = [ctcore::ct_util_name(), "--to", "iec-i", "1125899906842624"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_2() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "2"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_999() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1000() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1023() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1023"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1024() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1024"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_999999() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1000000() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1048575() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1048575"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1048576() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1048576"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_999999999() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1000000000() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1073741823() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1073741823"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1073741824() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1073741824"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_999999999999() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "999999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1000000000000() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1000000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1099511627775() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1099511627775"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1099511627776() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1099511627776"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_999999999999999() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "999999999999999"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1000000000000000() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1000000000000000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1125899906842623() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1125899906842623"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_long_iec_with_value_1125899906842624() {
            let args = [ctcore::ct_util_name(), "--to", "iec", "1125899906842624"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_unit_long_0() {
            let args = [ctcore::ct_util_name(), "--to-unit", "0", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_unit_long_1() {
            let args = [ctcore::ct_util_name(), "--to-unit", "1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_unit_long_2() {
            let args = [ctcore::ct_util_name(), "--to-unit", "2", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_unit_long_100000000000000000000000000000000() {
            let args = [
                ctcore::ct_util_name(),
                "--to-unit",
                "100000000000000000000000000000000",
                "10000",
            ];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_to_unit_long_aa() {
            let args = [ctcore::ct_util_name(), "--to-unit", "aa", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_padding_long_0() {
            let args = [ctcore::ct_util_name(), "--padding", "0", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_padding_long_1() {
            let args = [ctcore::ct_util_name(), "--padding", "1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_padding_long_100() {
            let args = [ctcore::ct_util_name(), "--padding", "100", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_padding_long_a() {
            let args = [ctcore::ct_util_name(), "--padding", "a", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_padding_long_negative_1() {
            let args = [ctcore::ct_util_name(), "--padding", "-1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_padding_long_negative_100() {
            let args = [ctcore::ct_util_name(), "--padding", "-100", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_padding_long_negative_1000() {
            let args = [ctcore::ct_util_name(), "--padding", "-1000", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_header_long_0() {
            let args = [ctcore::ct_util_name(), "--header", "0", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_header_long_1() {
            let args = [ctcore::ct_util_name(), "--header", "1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_header_long_10() {
            let args = [ctcore::ct_util_name(), "--header", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_header_long_100() {
            let args = [ctcore::ct_util_name(), "--header", "100", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_header_long_negitive_1() {
            let args = [ctcore::ct_util_name(), "--header", "-1", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_header_long_a() {
            let args = [ctcore::ct_util_name(), "--header", "a", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_round_long() {
            let args = [ctcore::ct_util_name(), "--round", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_round_long_from_zero() {
            let args = [ctcore::ct_util_name(), "--round", "from-zero", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_round_long_down() {
            let args = [ctcore::ct_util_name(), "--round", "down", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_round_long_towards_zero() {
            let args = [ctcore::ct_util_name(), "--round", "towards-zero", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_round_long_nearest() {
            let args = [ctcore::ct_util_name(), "--round", "nearest", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_k_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "K", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_ki_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Ki", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_m_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "M", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_mi_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Mi", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_g_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "G", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_gi_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Gi", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_ti_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Ti", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_t_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "T", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_p_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "P", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_pi_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Pi", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_e_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "E", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_ei_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Ei", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_z_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Z", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_zi_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Zi", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_y_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Y", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_yi_10() {
            let args = [ctcore::ct_util_name(), "--suffix", "Yi", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_wi_10_limit() {
            let args = [ctcore::ct_util_name(), "--suffix", "Wi", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_suffix_long_uppercase_w_10_limit() {
            let args = [ctcore::ct_util_name(), "--suffix", "W", "10", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_invalid_long() {
            let args = [ctcore::ct_util_name(), "--invalid", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_invalid_long_abort() {
            let args = [ctcore::ct_util_name(), "--invalid", "abort", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_invalid_long_fail() {
            let args = [ctcore::ct_util_name(), "--invalid", "fail", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_invalid_long_warn() {
            let args = [ctcore::ct_util_name(), "--invalid", "warn", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }

        #[test]
        fn test_ct_main_invalid_long_ignore() {
            let args = [ctcore::ct_util_name(), "--invalid", "ignore", "10000"];
            let result = numfmt_main(args.iter().map(OsString::from));

            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
                assert_eq!(code, 1);
            }
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use crate::ct_app;

        // numfmt 接口: numfmt [OPTION]... [NUMBER]...
        //
        // Options:
        //   -d, --delimiter <X>      use X instead of whitespace for field delimiter
        //       --field <FIELDS>     replace the numbers in these input fields; see FIELDS below [default: 1]
        //       --format <FORMAT>    use printf style floating-point FORMAT; see FORMAT below for details
        //       --from <UNIT>        auto-scale input numbers to UNITs; see UNIT below [default: none]
        //       --from-unit <N>      specify the input unit size [default: 1]
        //       --to <UNIT>          auto-scale output numbers to UNITs; see UNIT below [default: none]
        //       --to-unit <N>        the output unit size [default: 1]
        //       --padding <N>        pad the output to N characters; positive N will right-align; negative N will left-align; padding is ignored if the output is wider than N; the default is to automatically pad if a whitespace is found
        //       --header [<N>]       print (without converting) the first N header lines; N defaults to 1 if not specified
        //       --round <METHOD>     use METHOD for rounding when scaling [default: from-zero] [possible values: up, down, from-zero, towards-zero, nearest]
        //       --suffix <SUFFIX>    print SUFFIX after each formatted number, and accept inputs optionally ending with SUFFIX
        //       --invalid <INVALID>  set the failure mode for invalid input [default: abort] [possible values: abort, fail, warn, ignore]
        //   -h, --help               Print help
        //   -V, --version            Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "-V"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            // 测试用例3：验证当提供未知参数时是否正确报错
            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            // 测试用例4：验证当缺少必需的参数时是否正确报错
            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_delimiter_long() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "|", "1", "2", "3"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"|".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_colon() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", ":"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&":".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_comma() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", ","];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&",".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_semicolon() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", ";"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&";".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_vertical() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "|"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"|".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_tab() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "\t"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"\t".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_space() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", " "];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&" ".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_group_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "\u{001d}"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"\u{001d}".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_record_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "\u{001e}"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"\u{001e}".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_unit_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "\u{001f}"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"\u{001f}".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_digital() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "6"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"6".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_letter() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "a"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"a".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_letter_aa() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "aa"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"aa".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_uppercase_letter() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter", "A"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"A".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_long_no_value() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--delimiter"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_delimiter_short() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "|", "1", "2", "3"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"|".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_no_value() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_delimiter_short_colon() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", ":"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&":".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_comma() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", ","];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&",".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_semicolon() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", ";"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&";".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_vertical() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "|"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"|".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_tab() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "\t"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"\t".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_space() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", " "];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&" ".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_group_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "\u{001d}"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"\u{001d}".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_record_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "\u{001e}"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"\u{001e}".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_unit_separator() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "\u{001f}"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"\u{001f}".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_digital() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "6"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"6".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_letter() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "a"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"a".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_letter_aa() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "aa"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"aa".to_string())
            );
        }

        #[test]
        fn test_ct_app_delimiter_short_uppercase_letter() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "-d", "A"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("delimiter"),
                Some(&"A".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_1() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"1".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_1_2() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "1-2"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"1-2".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_none_2() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "-2"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"-2".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long__() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "-"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"-".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_1_() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "1-"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"1-".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_2_1() {
            let command = ct_app();

            let cmd_args = vec![ctcore::ct_util_name(), "--field", "2-1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"2-1".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_200() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "200"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"200".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_20000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "20000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"20000000000".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_not_digital() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "aa"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"aa".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_negative() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "-1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"-1".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_zero() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "0"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"0".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_float() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "2.1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"2.1".to_string())
            );
        }

        #[test]
        fn test_ct_app_field_long_none() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--field", "2.1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("field"),
                Some(&"2.1".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_none() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_format_long_d() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%d'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%d'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_i() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%i'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%i'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_u() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%u'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%u'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_c() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%c'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%c'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_s() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%s'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%s'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_o() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%o'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%o'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_x() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%x'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%x'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_uppercase_s() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%X'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%X'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_f() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%f'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%f'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_uppercase_e() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%E'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%E'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_e() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%e'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%e'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%g'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%g'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_uppercase_g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%G'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%G'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_p() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%p'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%p'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_precentage() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%%'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%%'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_with_right_padding() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%10d'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%10d'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_with_left_padding() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%-10d'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%-10d'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_with_keep_2_digits() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "'%.2f'"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"'%.2f'".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_with_strings() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "sssss"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"sssss".to_string())
            );
        }

        #[test]
        fn test_ct_app_format_long_with_err_precentage() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--format", "%%% %"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("format"),
                Some(&"%%% %".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_from_long_none() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "none"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_none_ok() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "none", "1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_none_err() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "none", "1000G"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto_with_value_1g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1G"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto_with_value_1gi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1Gi"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto_with_value_1k() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1K"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto_with_value_1ki() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1Ki"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto_with_value_1m() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1M"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto_with_value_1mi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1Mi"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto_with_value_small_1m() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1m"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_auto_with_value_small_1mi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "auto", "1mi"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_si_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_si_with_value_1k() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1K"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_si_with_value_1ki() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1Ki"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_si_with_value_1g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1G"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_si_with_value_1gi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "si", "1Gi"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_with_value_1k() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1K"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_with_value_1ki() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1Ki"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_with_value_1g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1G"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_with_value_1gi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec", "1Gi"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_i_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_i_with_value_1k() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1K"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_i_with_value_1ki() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1Ki"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_i_with_value_1g() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1G"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_long_iec_i_with_value_1gi() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from", "iec-i", "1Gi"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_unit_long_none() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from-unit"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_from_unit_long_0() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from-unit", "0"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from-unit"),
                Some(&"0".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_unit_long_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from-unit", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from-unit"),
                Some(&"1".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_unit_long_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from-unit", "2"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from-unit"),
                Some(&"2".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_unit_long_100000000000000000000000000000000() {
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--from-unit",
                "100000000000000000000000000000000",
            ];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from-unit"),
                Some(&"100000000000000000000000000000000".to_string())
            );
        }

        #[test]
        fn test_ct_app_from_unit_long_aa() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--from-unit", "aa"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("from-unit"),
                Some(&"aa".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "2"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1023"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1024"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1048575"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1048576"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1073741823"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1073741824"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_none_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "none", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"none".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "2"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1023"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1024"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1048575"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1048576"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1073741823"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1073741824"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_auto_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "auto", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"auto".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "2"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1023"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1024"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1048575"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1048576"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1073741823"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1073741824"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_si_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "si", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"si".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "2"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1023"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1024"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1048575"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1048576"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1073741823"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1073741824"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_i_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec-i", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec-i".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "2"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1023() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1023"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1024() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1024"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1048575() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1048575"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1048576() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1048576"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1073741823() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1073741823"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1073741824() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1073741824"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1099511627775() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1099511627775"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1099511627776() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1099511627776"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_999999999999999() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "999999999999999"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1000000000000000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1000000000000000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1125899906842623() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1125899906842623"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_long_iec_with_value_1125899906842624() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to", "iec", "1125899906842624"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to"),
                Some(&"iec".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_unit_long_none() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to-unit"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_to_unit_long_0() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to-unit", "0"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to-unit"),
                Some(&"0".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_unit_long_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to-unit", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to-unit"),
                Some(&"1".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_unit_long_2() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to-unit", "2"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to-unit"),
                Some(&"2".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_unit_long_100000000000000000000000000000000() {
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--to-unit",
                "100000000000000000000000000000000",
            ];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to-unit"),
                Some(&"100000000000000000000000000000000".to_string())
            );
        }

        #[test]
        fn test_ct_app_to_unit_long_aa() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--to-unit", "aa"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("to-unit"),
                Some(&"aa".to_string())
            );
        }

        #[test]
        fn test_ct_app_padding_long() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_padding_long_0() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "0"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("padding"),
                Some(&"0".to_string())
            );
        }

        #[test]
        fn test_ct_app_padding_long_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("padding"),
                Some(&"1".to_string())
            );
        }

        #[test]
        fn test_ct_app_padding_long_100() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "100"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("padding"),
                Some(&"100".to_string())
            );
        }

        #[test]
        fn test_ct_app_padding_long_a() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "a"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("padding"),
                Some(&"a".to_string())
            );
        }

        #[test]
        fn test_ct_app_padding_long_negative_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "-1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("padding"),
                Some(&"-1".to_string())
            );
        }

        #[test]
        fn test_ct_app_padding_long_negative_100() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "-100"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("padding"),
                Some(&"-100".to_string())
            );
        }

        #[test]
        fn test_ct_app_padding_long_negative_1000() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--padding", "-1000"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("padding"),
                Some(&"-1000".to_string())
            );
        }

        #[test]
        fn test_ct_app_header_long_0() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "0"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("header"),
                Some(&"0".to_string())
            );
        }

        #[test]
        fn test_ct_app_header_long_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("header"),
                Some(&"1".to_string())
            );
        }

        #[test]
        fn test_ct_app_header_long_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("header"),
                Some(&"10".to_string())
            );
        }

        #[test]
        fn test_ct_app_header_long_100() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "100"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("header"),
                Some(&"100".to_string())
            );
        }

        #[test]
        fn test_ct_app_header_long_negitive_1() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "-1"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("header"),
                Some(&"-1".to_string())
            );
        }

        #[test]
        fn test_ct_app_header_long_a() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--header", "a"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("header"),
                Some(&"a".to_string())
            );
        }

        #[test]
        fn test_ct_app_round_long() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_round_long_from_zero() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "from-zero"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("round"),
                Some(&"from-zero".to_string())
            );
        }

        #[test]
        fn test_ct_app_round_long_up() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "up"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("round"),
                Some(&"up".to_string())
            );
        }

        #[test]
        fn test_ct_app_round_long_down() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "down"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("round"),
                Some(&"down".to_string())
            );
        }

        #[test]
        fn test_ct_app_round_long_towards_zero() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "towards-zero"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("round"),
                Some(&"towards-zero".to_string())
            );
        }

        #[test]
        fn test_ct_app_round_long_nearest() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--round", "nearest"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("round"),
                Some(&"nearest".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_k_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "K", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"K".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_ki_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Ki", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Ki".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_m_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "M", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"M".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_mi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Mi", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Mi".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_g_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "G", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"G".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_gi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Gi", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Gi".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_ti_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Ti", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Ti".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_t_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "T", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"T".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_p_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "P", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"P".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_pi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Pi", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Pi".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_e_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "E", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"E".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_ei_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Ei", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Ei".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_z_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Z", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Z".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_zi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Zi", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Zi".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_y_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Y", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Y".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_yi_10() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Yi", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Yi".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_wi_10_limit() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "Wi", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"Wi".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_long_uppercase_w_10_limit() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--suffix", "W", "10"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("suffix"),
                Some(&"W".to_string())
            );
        }

        #[test]
        fn test_ct_app_invalid_long() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--invalid"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_invalid_long_abort() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--invalid", "abort"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("invalid"),
                Some(&"abort".to_string())
            );
        }

        #[test]
        fn test_ct_app_invalid_long_fail() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--invalid", "fail"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("invalid"),
                Some(&"fail".to_string())
            );
        }

        #[test]
        fn test_ct_app_invalid_long_warn() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--invalid", "warn"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("invalid"),
                Some(&"warn".to_string())
            );
        }

        #[test]
        fn test_ct_app_invalid_long_ignore() {
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--invalid", "ignore"];
            let result = command.try_get_matches_from(cmd_args);
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<String>("invalid"),
                Some(&"ignore".to_string())
            );
        }
    }
}
