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

//! sleep 命令用于使当前进程暂停执行一段时间。这个时间可以是以秒为单位的整数或浮点数。

extern crate rust_i18n;
use rust_i18n::t;
use std::thread;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::time::Duration;

use clap::{Arg, ArgAction, Command, builder::OsStringValueParser, crate_version};

use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError};
use ctcore::ct_format::num_parser::{ParseError, ParsedNumber};
use ctcore::ct_posix::GnuGetoptCommandExt;
use std::borrow::Cow;
use std::error::Error;
use std::ffi::{CStr, OsStr, OsString};
use std::fmt::{Display, Formatter};
use sys_locale::get_locale;

mod sleep_flags {
    pub const SLEEP_NUMBER: &str = "NUMBER";
}

#[derive(Debug)]
struct InvalidTimeIntervalError {
    operands: Vec<OsString>,
}

impl Display for InvalidTimeIntervalError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid time interval")
    }
}

impl Error for InvalidTimeIntervalError {}

fn quote_duration_operand(operand: &OsStr) -> Vec<u8> {
    let mut quoted = Vec::with_capacity(operand.len() + 2);
    quoted.push(b'\'');
    for byte in operand.as_encoded_bytes() {
        match byte {
            b'\x07' => quoted.extend_from_slice(b"\\a"),
            b'\x08' => quoted.extend_from_slice(b"\\b"),
            b'\t' => quoted.extend_from_slice(b"\\t"),
            b'\n' => quoted.extend_from_slice(b"\\n"),
            b'\x0b' => quoted.extend_from_slice(b"\\v"),
            b'\x0c' => quoted.extend_from_slice(b"\\f"),
            b'\r' => quoted.extend_from_slice(b"\\r"),
            b'\\' => quoted.extend_from_slice(b"\\\\"),
            b'\'' => quoted.extend_from_slice(b"\\'"),
            b' '..=b'~' => quoted.push(*byte),
            _ => {
                quoted.push(b'\\');
                quoted.push(b'0' + (byte >> 6));
                quoted.push(b'0' + ((byte >> 3) & 0o7));
                quoted.push(b'0' + (byte & 0o7));
            }
        }
    }
    quoted.push(b'\'');
    quoted
}

impl CTError for InvalidTimeIntervalError {
    fn code(&self) -> i32 {
        1
    }

    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        let mut diagnostic = Vec::new();
        for (index, operand) in self.operands.iter().enumerate() {
            if index > 0 {
                diagnostic.extend_from_slice(b"\n");
                diagnostic.extend_from_slice(ctcore::ct_util_name().as_bytes());
                diagnostic.extend_from_slice(b": ");
            }
            diagnostic.extend_from_slice(b"invalid time interval ");
            diagnostic.extend_from_slice(&quote_duration_operand(operand));
        }
        Cow::Owned(diagnostic)
    }

    fn usage(&self) -> bool {
        true
    }
}

#[derive(Debug)]
struct SleepUsageError {
    message: Vec<u8>,
}

impl SleepUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

impl Display for SleepUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for SleepUsageError {}

impl CTError for SleepUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct Sleep;
impl Tool for Sleep {
    fn name(&self) -> &'static str {
        "sleep"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        sleep_main(args.iter().cloned())
    }
}

pub fn sleep_main(args: impl ctcore::Args) -> CTResult<()> {
    initialize_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = prepare_sleep_args(args)?;
    let ends_with_option_delimiter = args.last().is_some_and(|arg| arg == "--");
    let matches = ct_app().try_get_matches_from(args)?;

    let numbers = sleep_parse_numbers(&matches, ends_with_option_delimiter)?;
    let sleep_dur = sleep_handle_second(&numbers)?;

    sleep(sleep_dur)
}

fn sleep_parse_numbers(matches: &clap::ArgMatches, allow_empty: bool) -> CTResult<Vec<&OsStr>> {
    let numbers = matches
        .get_many::<OsString>(sleep_flags::SLEEP_NUMBER)
        .map(|numbers| numbers.map(OsString::as_os_str).collect::<Vec<_>>())
        .or_else(|| allow_empty.then(Vec::new))
        .ok_or_else(|| {
            let err_message = format!(
                "missing operand\nTry '{} --help' for more information.",
                ctcore::ct_help_utility_name()
            );
            CtSimpleError::new(1, err_message)
        })?;

    Ok(numbers)
}

fn standard_long_option(name: &[u8]) -> Option<&'static str> {
    if let Some(option) = ["help", "version"]
        .into_iter()
        .find(|option| option.as_bytes() == name)
    {
        return Some(option);
    }
    if name.is_empty() {
        return None;
    }

    let mut matches = ["help", "version"]
        .into_iter()
        .filter(|option| option.as_bytes().starts_with(name));
    let option = matches.next()?;
    matches.next().is_none().then_some(option)
}

fn prepare_sleep_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    prepare_sleep_args_with_mode(args, ctcore::ct_posix::posixly_correct())
}

fn prepare_sleep_args_with_mode(
    args: impl ctcore::Args,
    posixly_correct: bool,
) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();

    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if bytes == b"--" {
            break;
        }
        if bytes.len() <= 1 || bytes[0] != b'-' {
            if posixly_correct {
                break;
            }
            continue;
        }

        if let Some(long) = bytes.strip_prefix(b"--") {
            let separator = long.iter().position(|byte| *byte == b'=');
            let name = &long[..separator.unwrap_or(long.len())];
            let Some(option) = standard_long_option(name) else {
                let mut message = b"unrecognized option '".to_vec();
                message.extend_from_slice(bytes);
                message.push(b'\'');
                return Err(SleepUsageError::boxed(message));
            };
            if separator.is_some() {
                return Err(SleepUsageError::boxed(
                    format!("option '--{option}' doesn't allow an argument").into_bytes(),
                ));
            }

            let mut terminal_args = Vec::with_capacity(2);
            if let Some(program) = args.first() {
                terminal_args.push(program.clone());
            }
            terminal_args.push(argument.clone());
            return Ok(terminal_args);
        }

        let mut message = b"invalid option -- '".to_vec();
        message.push(bytes[1]);
        message.push(b'\'');
        return Err(SleepUsageError::boxed(message));
    }

    Ok(args)
}

fn initialize_locale() {
    // SAFETY: setlocale reads the process environment and receives a static
    // NUL-terminated string. GNU sleep initializes the C locale before parsing.
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
    }
}

fn current_numeric_decimal_point() -> String {
    // SAFETY: localeconv returns process-owned locale storage. The decimal
    // separator is copied before the next locale operation can invalidate it.
    unsafe {
        let locale = ctcore::libc::localeconv();
        if locale.is_null() || (*locale).decimal_point.is_null() {
            return ".".to_string();
        }
        CStr::from_ptr((*locale).decimal_point)
            .to_str()
            .ok()
            .filter(|point| !point.is_empty())
            .unwrap_or(".")
            .to_string()
    }
}

#[derive(Clone, Copy)]
enum FloatParseAttempt {
    Complete(f64),
    Partial(f64, usize),
    Invalid,
}

fn normalize_decimal_point(
    input: &str,
    decimal_point: &str,
    reject_c_decimal_point: bool,
) -> (String, Vec<usize>) {
    let mut normalized = String::with_capacity(input.len());
    let mut offsets = Vec::with_capacity(input.len() + 1);
    let mut index = 0;
    offsets.push(index);

    while index < input.len() {
        if !decimal_point.is_empty() && input[index..].starts_with(decimal_point) {
            normalized.push('.');
            index += decimal_point.len();
            offsets.push(index);
        } else if reject_c_decimal_point && decimal_point != "." && input.as_bytes()[index] == b'.'
        {
            normalized.push('\u{1}');
            index += 1;
            offsets.push(index);
        } else {
            let character = input[index..]
                .chars()
                .next()
                .expect("index is before the end of a UTF-8 string");
            normalized.push(character);
            for byte in 1..=character.len_utf8() {
                offsets.push(index + byte);
            }
            index += character.len_utf8();
        }
    }

    (normalized, offsets)
}

fn parse_float_attempt(
    input: &str,
    decimal_point: &str,
    reject_c_decimal_point: bool,
) -> FloatParseAttempt {
    let (normalized, offsets) =
        normalize_decimal_point(input, decimal_point, reject_c_decimal_point);
    match ParsedNumber::parse_f64(&normalized) {
        Ok(value) => FloatParseAttempt::Complete(value),
        Err(ParseError::CtPartialMatch(value, rest)) => {
            let consumed = normalized.len() - rest.len();
            FloatParseAttempt::Partial(value, offsets[consumed])
        }
        Err(ParseError::CtNotNumeric | ParseError::CtOverflow) => FloatParseAttempt::Invalid,
    }
}

fn parse_duration_with_decimal_point(input: &str, decimal_point: &str) -> Option<f64> {
    let locale = parse_float_attempt(input, decimal_point, true);
    let c_locale = parse_float_attempt(input, ".", false);

    let (seconds, consumed) = match (locale, c_locale) {
        (FloatParseAttempt::Complete(seconds), _) => (seconds, input.len()),
        (_, FloatParseAttempt::Complete(seconds)) => (seconds, input.len()),
        (
            FloatParseAttempt::Partial(seconds, consumed),
            FloatParseAttempt::Partial(c_seconds, c_consumed),
        ) => {
            if c_consumed > consumed {
                (c_seconds, c_consumed)
            } else {
                (seconds, consumed)
            }
        }
        (FloatParseAttempt::Partial(seconds, consumed), FloatParseAttempt::Invalid) => {
            (seconds, consumed)
        }
        (FloatParseAttempt::Invalid, FloatParseAttempt::Partial(seconds, consumed)) => {
            (seconds, consumed)
        }
        (FloatParseAttempt::Invalid, FloatParseAttempt::Invalid) => return None,
    };

    let suffix = &input[consumed..];

    match suffix {
        "" | "s" => Some(seconds),
        "m" => Some(seconds * 60.0),
        "h" => Some(seconds * 60.0 * 60.0),
        "d" => Some(seconds * 60.0 * 60.0 * 24.0),
        _ => None,
    }
}

fn parse_duration(input: &str) -> Option<f64> {
    parse_duration_with_decimal_point(input, &current_numeric_decimal_point())
}

fn duration_from_seconds(seconds: f64) -> Duration {
    const NANOS_PER_SECOND: u64 = 1_000_000_000;
    const MAX_TIME_T_SECONDS: u64 = i64::MAX as u64;

    if !seconds.is_finite() || seconds >= MAX_TIME_T_SECONDS as f64 {
        return Duration::new(MAX_TIME_T_SECONDS, NANOS_PER_SECOND as u32 - 1);
    }

    let mut whole_seconds = seconds as u64;
    let fractional_nanos = NANOS_PER_SECOND as f64 * (seconds - whole_seconds as f64);
    let mut nanos = fractional_nanos as u64;
    nanos += u64::from((nanos as f64) < fractional_nanos);
    whole_seconds += nanos / NANOS_PER_SECOND;
    nanos %= NANOS_PER_SECOND;

    Duration::new(whole_seconds, nanos as u32)
}

fn sleep_handle_second<T: AsRef<OsStr>>(args: &[T]) -> CTResult<Duration> {
    let mut invalid_operands = Vec::new();
    let mut seconds = 0.0;

    for input in args {
        match input.as_ref().to_str().and_then(parse_duration) {
            Some(interval) if interval >= 0.0 => seconds += interval,
            _ => invalid_operands.push(input.as_ref().to_os_string()),
        }
    }

    if !invalid_operands.is_empty() {
        return Err(InvalidTimeIntervalError {
            operands: invalid_operands,
        }
        .into());
    }

    Ok(duration_from_seconds(seconds))
}

fn sleep(sleep_dur: Duration) -> CTResult<()> {
    thread::sleep(sleep_dur);

    Ok(())
}

pub fn ct_app() -> Command {
    ct_app_with_getopt_mode(ctcore::ct_posix::posixly_correct())
}

fn ct_app_with_getopt_mode(posixly_correct: bool) -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("sleep.about");
    let usage_description = t!("sleep.usage");
    let args = vec![
        Arg::new(sleep_flags::SLEEP_NUMBER)
            .help(t!("sleep.clap.sleep_number"))
            .value_name(sleep_flags::SLEEP_NUMBER)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new()),
        Arg::new("help").long("help").action(ArgAction::Help),
        Arg::new("version")
            .long("version")
            .action(ArgAction::Version),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(t!("sleep.after_help"))
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(args)
        .gnu_getopt_with_mode(posixly_correct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn test_tool_implementation() {
        let tool = Sleep;

        // Test name method
        assert_eq!(tool.name(), "sleep");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("sleep"));

        // Test execute method with help flag (should work)
        let args: Vec<OsString> = vec![OsString::from("sleep"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    #[cfg(test)]
    mod handle_second_tests {
        use super::*;
        use clap::Command;

        #[test]
        fn test_sleep_parse_numbers() {
            let cmd = Command::new("test").arg(
                Arg::new(sleep_flags::SLEEP_NUMBER)
                    .action(ArgAction::Append)
                    .value_parser(OsStringValueParser::new()),
            );

            let matches = cmd.try_get_matches_from(vec!["test", "5", "10"]).unwrap();
            let numbers = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(numbers, vec!["5", "10"]);
        }

        #[test]
        fn test_sleep_handle_second() {
            let args = vec!["5s", "1m", "2h"];
            let duration = sleep_handle_second(&args).unwrap();

            assert_eq!(duration, Duration::from_secs(5 + 60 + 2 * 3600));
        }

        #[test]
        fn test_sleep_handle_second_invalid_input() {
            let args = vec!["5x", "1m"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_single_second() {
            let args = vec!["5s"];
            let duration = sleep_handle_second(&args).unwrap();

            assert_eq!(duration, Duration::from_secs(5));
        }

        #[test]
        fn test_sleep_handle_second_multiple_units() {
            let args = vec!["1d", "2h", "30m", "45s"];
            let duration = sleep_handle_second(&args).unwrap();

            let expected_duration = Duration::from_secs(86400 + 2 * 3600 + 30 * 60 + 45);
            assert_eq!(duration, expected_duration);
        }

        #[test]
        fn test_sleep_handle_second_empty_input() {
            let args = vec![""];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_whitespace_input() {
            let args = vec!["  "];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_invalid_format() {
            let args = vec!["5x", "2y"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_mixed_valid_invalid() {
            let args = vec!["5s", "invalid", "10m"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_large_duration() {
            let args = vec!["1000d", "24h"];
            let duration = sleep_handle_second(&args).unwrap();

            let expected_duration = Duration::from_secs(1000 * 86400 + 24 * 3600);
            assert_eq!(duration, expected_duration);
        }

        #[test]
        fn test_sleep_handle_second_negative_duration() {
            let args = vec!["-5s"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_accepts_hex_duration_with_suffix() {
            let duration = sleep_handle_second(&["0x1p0s"]).unwrap();

            assert_eq!(duration, Duration::from_secs(1));
        }

        #[test]
        fn test_sleep_handle_second_rejects_hex_duration_without_mantissa() {
            for input in ["0x.", "0x.p0"] {
                assert!(sleep_handle_second(&[input]).is_err(), "{input}");
            }
        }

        #[test]
        fn test_sleep_handle_second_rejects_trailing_whitespace() {
            assert!(sleep_handle_second(&["0 "]).is_err());
        }

        #[test]
        fn test_sleep_handle_second_accepts_leading_whitespace() {
            assert_eq!(sleep_handle_second(&[" 0"]).unwrap(), Duration::ZERO);
        }

        #[test]
        fn test_sleep_handle_second_accumulates_and_rounds_subnanosecond_intervals() {
            assert_eq!(
                sleep_handle_second(&["0.0000000001"]).unwrap(),
                Duration::from_nanos(1)
            );
            assert_eq!(
                sleep_handle_second(&["0.0000000006", "0.0000000006"]).unwrap(),
                Duration::from_nanos(2)
            );
        }

        #[test]
        fn test_parse_duration_prefers_locale_decimal_point_and_falls_back_to_c_locale() {
            assert_eq!(parse_duration_with_decimal_point("0,0", ","), Some(0.0));
            assert_eq!(parse_duration_with_decimal_point("0.0", ","), Some(0.0));
            assert_eq!(parse_duration_with_decimal_point("0,0.0", ","), None);
        }

        #[test]
        fn test_prepare_sleep_args_uses_gnu_standard_option_diagnostics() {
            let cases = [
                (
                    vec![OsString::from("sleep"), OsString::from("-h")],
                    b"invalid option -- 'h'".as_slice(),
                ),
                (
                    vec![OsString::from("sleep"), OsString::from("--unknown")],
                    b"unrecognized option '--unknown'".as_slice(),
                ),
                (
                    vec![OsString::from("sleep"), OsString::from("--hel=value")],
                    b"option '--help' doesn't allow an argument".as_slice(),
                ),
            ];

            for (args, diagnostic) in cases {
                let error = prepare_sleep_args(args.into_iter()).unwrap_err();
                assert_eq!(error.diagnostic_bytes().as_ref(), diagnostic);
            }
        }

        #[test]
        fn test_posix_mode_stops_option_parsing_after_the_first_duration() {
            let args = vec![
                OsString::from("sleep"),
                OsString::from("0"),
                OsString::from("--help"),
            ];
            assert_eq!(
                prepare_sleep_args_with_mode(args.clone().into_iter(), true).unwrap(),
                args
            );

            let matches = ct_app_with_getopt_mode(true)
                .try_get_matches_from(args)
                .unwrap();
            let durations = matches
                .get_many::<OsString>(sleep_flags::SLEEP_NUMBER)
                .unwrap()
                .collect::<Vec<_>>();
            assert_eq!(durations, [&OsString::from("0"), &OsString::from("--help")]);
        }

        #[cfg(unix)]
        #[test]
        fn test_sleep_handle_second_preserves_non_utf8_diagnostic_bytes() {
            let result = sleep_handle_second(&[OsString::from_vec(vec![0xff])]).unwrap_err();

            assert_eq!(
                result.diagnostic_bytes().as_ref(),
                b"invalid time interval '\\377'"
            );
        }
    }
    #[cfg(test)]
    mod sleep_parse_numbers_tests {
        use super::*;
        #[test]
        fn test_sleep_parse_numbers_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false);

            assert!(result.is_err());
            let error = result.unwrap_err().to_string();
            assert!(error.contains("missing operand"));
            assert!(error.contains(&format!(
                "Try '{} --help' for more information.",
                ctcore::ct_help_utility_name()
            )));
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_5() {
            let args = vec![ctcore::ct_util_name(), "5"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["5"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_0() {
            let args = vec![ctcore::ct_util_name(), "0"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["0"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_seconds_2() {
            let args = vec![ctcore::ct_util_name(), "2s"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2s"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_minutes_2() {
            let args = vec![ctcore::ct_util_name(), "2m"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2m"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_hours_2() {
            let args = vec![ctcore::ct_util_name(), "2h"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2h"]);
        }
        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_days_2() {
            let args = vec![ctcore::ct_util_name(), "2d"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2d"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_err_2() {
            let args = vec![ctcore::ct_util_name(), "2q"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches, false).unwrap();

            assert_eq!(result, ["2q"]);
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_sleep_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = sleep_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_rejects_short_version_option() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = sleep_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_rejects_short_help_option() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_accepts_end_of_options_without_operands() {
            let args = [ctcore::ct_util_name(), "--"];
            let result = sleep_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_1() {
            let args = [ctcore::ct_util_name(), "1"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_0_3_0_2() {
            let args = [ctcore::ct_util_name(), "0.3", "0.2"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_0_3_0_2_0_1() {
            let args = [ctcore::ct_util_name(), "0.3", "0.2", "0.1"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_1_qq() {
            let args = [ctcore::ct_util_name(), "1", "qq"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_sleep_0_1_0_3_s() {
            let args = [ctcore::ct_util_name(), "0.1s", "0.3s"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_0() {
            let args = [ctcore::ct_util_name(), "0"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_suffix_seconds_1() {
            let args = [ctcore::ct_util_name(), "1s"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_suffix_err_1() {
            let args = [ctcore::ct_util_name(), "1q"];
            let result = sleep_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;
        #[cfg(unix)]
        use std::os::unix::ffi::OsStringExt;

        // sleep 接口: sleep NUMBER[SUFFIX]...
        //             sleep OPTION
        //
        // Arguments:
        //   [NUMBER]...  pause for NUMBER seconds
        //
        // Options:
        //   --help         Print help
        //   --version      Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_rejects_short_version_option() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
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
        fn test_ct_app_rejects_short_help_option() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
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

            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[cfg(unix)]
        #[test]
        fn test_ct_app_accepts_non_utf8_duration_operand() {
            let command = ct_app();
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from_vec(vec![0xff]),
            ];

            assert!(command.try_get_matches_from(args).is_ok());
        }

        #[test]
        fn test_ct_app_sleep_5() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "5"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_0() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "0"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_seconds_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2s"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_minutes_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2m"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_hours_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2h"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_sleep_suffix_days_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2d"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_err_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2q"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
