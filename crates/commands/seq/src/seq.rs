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
use std::io::{Write, stdout};
rust_i18n::i18n!("locales", fallback = "en-US");

use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, Command, crate_version};
use num_traits::{ToPrimitive, Zero};

use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, strip_errno};
use std::borrow::Cow;
use std::error::Error as StdError;
use std::ffi::{CStr, OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use sys_locale::get_locale;
mod error;
use crate::error::SeqError;
use ctcore::ct_format::long_double::{
    ExtendedBigDecimal, GnuFloatFormat, ParseNumberError, PreciseNumber, long_double_linear_value,
    overflows_long_double, quantize_long_double,
};

const SEQ_SEPARATOR: &str = "separator";
const SEQ_TERMINATOR: &str = "terminator";
const SEQ_EQUAL_WIDTH: &str = "equal-width";
const SEQ_FORMAT: &str = "format";

const SEQ_NUMBERS: &str = "numbers";
const SEQ_NEGATIVE_NUMBER_MARKER: &str = "\0CT_NEG_";

const SEQ_GNU_LONG_OPTIONS: &[(&str, bool)] = &[
    ("equal-width", false),
    ("format", true),
    ("separator", true),
    ("help", false),
    ("version", false),
];

// Fast path optimization limit (same as GNU seq)
const SEQ_FAST_STEP_LIMIT: u64 = 200;

#[derive(Clone, Default)]
struct SeqOptions {
    separator: OsString,
    terminator: String,
    is_equal_width: bool,
    format: Option<OsString>,
}

impl SeqOptions {
    fn new(matches: &clap::ArgMatches) -> Self {
        let unmask_terminator = |s: &str| -> String {
            if let Some(stripped) = s.strip_prefix(SEQ_NEGATIVE_NUMBER_MARKER) {
                format!("-{stripped}")
            } else {
                s.to_string()
            }
        };

        Self {
            separator: matches
                .get_one::<OsString>(SEQ_SEPARATOR)
                .map(|value| unmask_negative_number_arg(value.clone()))
                .unwrap_or_else(|| "\n".into()),
            terminator: matches
                .get_one::<String>(SEQ_TERMINATOR)
                .map(|s| unmask_terminator(s.as_str()))
                .unwrap_or_else(|| "\n".to_string()),
            is_equal_width: matches.get_flag(SEQ_EQUAL_WIDTH),
            format: matches
                .get_one::<OsString>(SEQ_FORMAT)
                .map(|value| unmask_negative_number_arg(value.clone())),
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
    separator: &'a [u8],
    terminator: &'a str,
    pad: bool,
    padding: usize,
    format: &'a SeqOutputFormat,
    buffer: Option<&'a mut Vec<u8>>,
}

enum SeqOutputFormat {
    Float(GnuFloatFormat),
    ExactInteger,
}

struct RawIntegerSequence {
    first: Vec<u8>,
    last: Vec<u8>,
    step: u64,
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

#[derive(Debug)]
struct SeqUsageError {
    message: Vec<u8>,
}

impl SeqUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

impl Display for SeqUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl StdError for SeqUsageError {}

impl CTError for SeqUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage(&self) -> bool {
        true
    }
}

enum SeqLongOptionMatch {
    None,
    Recognized(&'static str, bool),
    Ambiguous(Vec<(&'static str, bool)>),
}

fn match_seq_long_option(name: &[u8]) -> SeqLongOptionMatch {
    if let Some((option, takes_value)) = SEQ_GNU_LONG_OPTIONS
        .iter()
        .find(|(option, _)| option.as_bytes() == name)
    {
        return SeqLongOptionMatch::Recognized(option, *takes_value);
    }

    let matches = SEQ_GNU_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|(option, _)| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] if SEQ_TERMINATOR.as_bytes().starts_with(name) => {
            SeqLongOptionMatch::Recognized(SEQ_TERMINATOR, true)
        }
        [] => SeqLongOptionMatch::None,
        [(option, takes_value)] => SeqLongOptionMatch::Recognized(option, *takes_value),
        _ => SeqLongOptionMatch::Ambiguous(matches),
    }
}

fn is_negative_number_argument(bytes: &[u8]) -> bool {
    bytes.len() > 1 && bytes[0] == b'-' && (bytes[1].is_ascii_digit() || bytes[1] == b'.')
}

fn prepare_seq_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut index = 1;

    while index < args.len() {
        let argument = args[index].as_os_str();
        let bytes = argument.as_bytes();
        if bytes == b"--" {
            break;
        }
        if bytes.len() <= 1 || bytes[0] != b'-' || is_negative_number_argument(bytes) {
            break;
        }

        if bytes.starts_with(b"--") {
            let separator = bytes[2..].iter().position(|byte| *byte == b'=');
            let name = &bytes[2..separator.map_or(bytes.len(), |offset| offset + 2)];
            match match_seq_long_option(name) {
                SeqLongOptionMatch::None => {
                    let mut message = b"unrecognized option '".to_vec();
                    message.extend_from_slice(bytes);
                    message.push(b'\'');
                    return Err(SeqUsageError::boxed(message));
                }
                SeqLongOptionMatch::Ambiguous(matches) => {
                    let possibilities = matches
                        .into_iter()
                        .map(|(option, _)| format!("'--{option}'"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let mut message = b"option '".to_vec();
                    message.extend_from_slice(bytes);
                    message.extend_from_slice(b"' is ambiguous; possibilities: ");
                    message.extend_from_slice(possibilities.as_bytes());
                    return Err(SeqUsageError::boxed(message));
                }
                SeqLongOptionMatch::Recognized(option, takes_value) => {
                    if separator.is_some() && !takes_value {
                        return Err(SeqUsageError::boxed(
                            format!("option '--{option}' doesn't allow an argument").into_bytes(),
                        ));
                    }
                    if takes_value && separator.is_none() {
                        if index + 1 == args.len() {
                            return Err(SeqUsageError::boxed(
                                format!("option '--{option}' requires an argument").into_bytes(),
                            ));
                        }
                        index += 1;
                    }
                    if matches!(option, "help" | "version") {
                        return Ok(args);
                    }
                }
            }
        } else {
            let mut short_index = 1;
            while short_index < bytes.len() {
                let option = bytes[short_index];
                match option {
                    b'f' | b's' | b't' => {
                        if short_index + 1 == bytes.len() {
                            if index + 1 == args.len() {
                                return Err(SeqUsageError::boxed(
                                    format!(
                                        "option requires an argument -- '{}'",
                                        char::from(option)
                                    )
                                    .into_bytes(),
                                ));
                            }
                            index += 1;
                        }
                        break;
                    }
                    b'w' | b'h' | b'V' => short_index += 1,
                    _ => {
                        let mut message = b"invalid option -- '".to_vec();
                        message.push(option);
                        message.push(b'\'');
                        return Err(SeqUsageError::boxed(message));
                    }
                }
            }
        }
        index += 1;
    }

    Ok(args)
}

fn mask_negative_number_args(args: Vec<OsString>) -> Vec<OsString> {
    args.into_iter()
        .map(|arg| {
            let bytes = arg.as_os_str().as_bytes();
            if bytes.len() > 1
                && bytes[0] == b'-'
                && (bytes[1].is_ascii_digit() || bytes[1] == b'.')
            {
                let mut masked =
                    Vec::with_capacity(SEQ_NEGATIVE_NUMBER_MARKER.len() + bytes.len() - 1);
                masked.extend_from_slice(SEQ_NEGATIVE_NUMBER_MARKER.as_bytes());
                masked.extend_from_slice(&bytes[1..]);
                return OsString::from_vec(masked);
            }
            arg
        })
        .collect()
}

fn unmask_negative_number_arg(arg: OsString) -> OsString {
    let bytes = arg.as_os_str().as_bytes();
    let Some(unmasked) = bytes.strip_prefix(SEQ_NEGATIVE_NUMBER_MARKER.as_bytes()) else {
        return arg;
    };

    let mut original = Vec::with_capacity(unmasked.len() + 1);
    original.push(b'-');
    original.extend_from_slice(unmasked);
    OsString::from_vec(original)
}

pub fn seq_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    set_process_locale();
    configure_sigpipe();

    let modified_args = mask_negative_number_args(prepare_seq_args(args)?);

    let matches = ct_app().try_get_matches_from(modified_args)?;
    let options = SeqOptions::new(&matches);

    let raw_numbers = collect_number_args(&matches)?;
    let user_format = parse_format_option(options.format.as_deref())?;
    validate_option_compatibility(&options)?;
    if let Some(sequence) = raw_integer_sequence(&raw_numbers, &options) {
        return seq_raw_integer_fast(sequence, options.separator.as_bytes(), &options.terminator)
            .map_err(|error| {
                CtSimpleError::new(1, format!("write error: {}", strip_errno(&error)))
            });
    }

    let numbers = parse_number_args(&raw_numbers)?;
    let (first, increment, last) = get_sequence_range(&numbers)?;

    // Try fast path optimization first
    if let Some((first_u64, last_u64, step_u64)) =
        can_use_fast_path(&first, &increment, &last, &options)
    {
        return match seq_fast(
            first_u64,
            last_u64,
            step_u64,
            options.separator.as_bytes(),
            &options.terminator,
        ) {
            Ok(_) => Ok(()),
            Err(e) => Err(CtSimpleError::new(
                1,
                format!("write error: {}", strip_errno(&e)),
            )),
        };
    }

    let padding = calculate_padding(&first, &last);
    let largest_dec = calculate_largest_decimal(&first, &increment);
    let format = select_output_format(
        &options,
        user_format,
        &first,
        &increment,
        &last,
        padding,
        largest_dec,
    )?;

    let config = PrintConfig {
        largest_dec,
        separator: options.separator.as_bytes(),
        terminator: &options.terminator,
        pad: options.is_equal_width,
        padding,
        format: &format,
        buffer: None,
    };

    match print_seq((first.number, increment.number, last.number), config) {
        Ok(_) => Ok(()),
        Err(e) => Err(CtSimpleError::new(
            1,
            format!("write error: {}", strip_errno(&e)),
        )),
    }
}

pub fn seq_native_semantic(args: impl ctcore::Args) -> CTResult<SeqSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    set_process_locale();

    let modified_args = mask_negative_number_args(prepare_seq_args(args)?);

    let matches = ct_app().try_get_matches_from(modified_args)?;
    let options = SeqOptions::new(&matches);
    let raw_numbers = collect_number_args(&matches)?;
    let user_format = parse_format_option(options.format.as_deref())?;
    validate_option_compatibility(&options)?;
    if let Some(sequence) = raw_integer_sequence(&raw_numbers, &options) {
        return raw_integer_semantic(sequence, options.separator.as_bytes(), &options.terminator)
            .map_err(|error| {
                CtSimpleError::new(1, format!("write error: {}", strip_errno(&error)))
            });
    }

    let numbers = parse_number_args(&raw_numbers)?;
    let (first, increment, last) = get_sequence_range(&numbers)?;

    let padding = calculate_padding(&first, &last);
    let largest_dec = calculate_largest_decimal(&first, &increment);
    let format = select_output_format(
        &options,
        user_format,
        &first,
        &increment,
        &last,
        padding,
        largest_dec,
    )?;

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
            separator: options.separator.as_bytes(),
            terminator: &options.terminator,
            pad: options.is_equal_width,
            padding,
            format: &format,
        },
        &mut rows,
        &mut classic_buffer,
    )
    .map_err(|e| CtSimpleError::new(1, format!("write error: {}", strip_errno(&e))))?;

    Ok(SeqSemantic {
        rows,
        classic_text: String::from_utf8_lossy(&classic_buffer).into_owned(),
    })
}

fn set_process_locale() {
    // SAFETY: setlocale receives a static NUL-terminated string and is called
    // before seq starts formatting values.
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
    }
}

fn configure_sigpipe() {
    if !parent_ignores_sigpipe() {
        let _ = ctcore::ct_signals::enable_pipe_errors();
    }
}

fn parent_ignores_sigpipe() -> bool {
    // Rust ignores SIGPIPE before main. On Linux the parent's signal mask
    // preserves whether an invoking shell explicitly ignored it for children.
    let parent = unsafe { ctcore::libc::getppid() };
    let Ok(status) = std::fs::read_to_string(format!("/proc/{parent}/status")) else {
        return false;
    };
    let Some(mask) = status
        .lines()
        .find_map(|line| line.strip_prefix("SigIgn:\t"))
        .and_then(|mask| u64::from_str_radix(mask, 16).ok())
    else {
        return false;
    };
    mask & (1_u64 << (ctcore::libc::SIGPIPE - 1)) != 0
}

fn collect_number_args(matches: &clap::ArgMatches) -> CTResult<Vec<OsString>> {
    let numbers = matches
        .get_many::<OsString>(SEQ_NUMBERS)
        .ok_or(SeqError::NoArguments)?
        .cloned()
        .map(unmask_negative_number_arg)
        .collect::<Vec<_>>();
    if numbers.len() > 3 {
        return Err(SeqError::ExtraOperand(numbers[3].clone()).into());
    }
    Ok(numbers)
}

fn parse_number_args(raw_numbers: &[OsString]) -> CTResult<Vec<String>> {
    Ok(raw_numbers
        .iter()
        .map(|value| {
            let Some(value) = value.to_str() else {
                return Err(SeqError::NonUtf8Argument(value.clone()));
            };
            Ok(value.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?)
}

fn validate_option_compatibility(options: &SeqOptions) -> CTResult<()> {
    if options.is_equal_width && options.format.is_some() {
        return Err(SeqError::FormatWithEqualWidth.into());
    }
    Ok(())
}

fn raw_integer_sequence(
    raw_numbers: &[OsString],
    options: &SeqOptions,
) -> Option<RawIntegerSequence> {
    if options.format.is_some() || options.is_equal_width || options.separator.as_bytes().len() != 1
    {
        return None;
    }

    let (first, step, last) = match raw_numbers {
        [last] => (b"1".as_slice(), 1, decimal_digits(last)?),
        [first, last] => (decimal_digits(first)?, 1, decimal_digits(last)?),
        [first, step, last] => (
            decimal_digits(first)?,
            parse_small_decimal(decimal_digits(step)?)?,
            decimal_digits(last)?,
        ),
        _ => return None,
    };

    (step > 0 && step <= SEQ_FAST_STEP_LIMIT).then(|| RawIntegerSequence {
        first: trim_decimal_leading_zeros(first).to_vec(),
        last: trim_decimal_leading_zeros(last).to_vec(),
        step,
    })
}

fn decimal_digits(value: &OsString) -> Option<&[u8]> {
    let bytes = value.as_os_str().as_bytes();
    (!bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)).then_some(bytes)
}

fn parse_small_decimal(value: &[u8]) -> Option<u64> {
    value.iter().try_fold(0_u64, |number, byte| {
        number.checked_mul(10)?.checked_add(u64::from(byte - b'0'))
    })
}

fn trim_decimal_leading_zeros(value: &[u8]) -> &[u8] {
    match value.iter().position(|byte| *byte != b'0') {
        Some(index) => &value[index..],
        None => &value[value.len() - 1..],
    }
}

fn compare_decimal_strings(left: &[u8], right: &[u8]) -> std::cmp::Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn increment_decimal(value: &mut Vec<u8>) {
    for index in (0..value.len()).rev() {
        if value[index] != b'9' {
            value[index] += 1;
            return;
        }
        value[index] = b'0';
    }
    value.insert(0, b'1');
}

fn walk_raw_integer_sequence(
    sequence: RawIntegerSequence,
    mut emit: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<()> {
    if compare_decimal_strings(&sequence.first, &sequence.last).is_gt() {
        return Ok(());
    }

    let mut current = sequence.first;
    loop {
        emit(&current)?;
        for _ in 0..sequence.step {
            increment_decimal(&mut current);
        }
        if compare_decimal_strings(&current, &sequence.last).is_gt() {
            return Ok(());
        }
    }
}

fn seq_raw_integer_fast(
    sequence: RawIntegerSequence,
    separator: &[u8],
    terminator: &str,
) -> std::io::Result<()> {
    use std::io::BufWriter;

    let stdout = stdout();
    let mut writer = BufWriter::with_capacity(8192, stdout.lock());
    let mut is_first = true;
    walk_raw_integer_sequence(sequence, |value| {
        if !is_first {
            writer.write_all(separator)?;
        }
        writer.write_all(value)?;
        is_first = false;
        Ok(())
    })?;
    if !is_first {
        writer.write_all(terminator.as_bytes())?;
    }
    writer.flush()
}

fn raw_integer_semantic(
    sequence: RawIntegerSequence,
    separator: &[u8],
    terminator: &str,
) -> std::io::Result<SeqSemantic> {
    let mut classic_buffer = Vec::new();
    let mut rows = Vec::new();
    let mut is_first = true;

    walk_raw_integer_sequence(sequence, |value| {
        if !is_first {
            classic_buffer.write_all(separator)?;
        }
        classic_buffer.write_all(value)?;
        rows.push(SeqRow {
            index: rows.len(),
            value: String::from_utf8(value.to_vec()).expect("raw integer digits are ASCII"),
        });
        is_first = false;
        Ok(())
    })?;
    if !is_first {
        classic_buffer.write_all(terminator.as_bytes())?;
    }

    Ok(SeqSemantic {
        classic_text: String::from_utf8_lossy(&classic_buffer).into_owned(),
        rows,
    })
}

fn get_sequence_range(
    numbers: &[String],
) -> CTResult<(PreciseNumber, PreciseNumber, PreciseNumber)> {
    let first = if numbers.len() > 1 {
        parse_number_arg(&numbers[0])?
    } else {
        PreciseNumber::one()
    };

    let increment = if numbers.len() > 2 {
        let inc = parse_number_arg(&numbers[1])?;
        if inc.is_zero() {
            return Err(SeqError::ZeroIncrement(numbers[1].clone()).into());
        }
        inc
    } else {
        PreciseNumber::one()
    };

    let last = parse_number_arg(numbers.last().unwrap())?;

    Ok((first, increment, last))
}

fn parse_number_arg(value: &str) -> CTResult<PreciseNumber> {
    parse_number_arg_with_decimal_point(value, &current_numeric_decimal_point())
}

fn parse_number_arg_with_decimal_point(
    value: &str,
    decimal_point: &str,
) -> CTResult<PreciseNumber> {
    let normalized = if decimal_point == "." || !value.contains(decimal_point) {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(value.replace(decimal_point, "."))
    };
    let mut number: PreciseNumber = normalized
        .parse()
        .map_err(|error| SeqError::ParseError(value.to_string(), error))?;
    if value.as_bytes().contains(&b'_') {
        return Err(SeqError::ParseError(value.to_string(), ParseNumberError::Float).into());
    }
    apply_gnu_locale_numeric_layout(&mut number, value, decimal_point);
    if overflows_long_double(&number.number) {
        return Err(SeqError::ParseError(value.to_string(), ParseNumberError::Float).into());
    }
    Ok(number)
}

fn current_numeric_decimal_point() -> String {
    // SAFETY: localeconv returns pointers owned by the process locale. seq sets
    // LC_ALL before parsing operands and only copies the NUL-terminated value.
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

fn apply_gnu_locale_numeric_layout(number: &mut PreciseNumber, value: &str, decimal_point: &str) {
    if decimal_point == "." || !value.contains(decimal_point) || value.contains('.') {
        return;
    }

    let value = value
        .trim_start_matches([' ', '\t', '\n', '\r', '\u{b}', '\u{c}'])
        .strip_prefix('+')
        .unwrap_or(value);
    if value.contains(['x', 'X']) {
        return;
    }

    let Some(exponent_index) = value.find(['e', 'E']) else {
        number.num_integral_digits = value.len();
        number.num_fractional_digits = 0;
        return;
    };
    let exponent = value[exponent_index + 1..]
        .parse::<i64>()
        .expect("parsed seq exponent must be a signed integer");
    let magnitude = usize::try_from(exponent.unsigned_abs()).unwrap_or(usize::MAX);
    number.num_fractional_digits = if exponent.is_negative() { magnitude } else { 0 };
    number.num_integral_digits = if exponent.is_negative() {
        exponent_index.saturating_add(1).saturating_add(magnitude)
    } else {
        exponent_index.saturating_add(magnitude)
    };
}

fn calculate_padding(first: &PreciseNumber, last: &PreciseNumber) -> usize {
    first.num_integral_digits.max(last.num_integral_digits)
}

fn calculate_largest_decimal(first: &PreciseNumber, increment: &PreciseNumber) -> usize {
    first
        .num_fractional_digits
        .max(increment.num_fractional_digits)
}

fn parse_format_option(format_str: Option<&OsStr>) -> CTResult<Option<GnuFloatFormat>> {
    let Some(format_str) = format_str else {
        return Ok(None);
    };

    GnuFloatFormat::try_parse(format_str.as_bytes())
        .map(Some)
        .map_err(|error| Box::new(error) as Box<dyn CTError>)
}

fn select_output_format(
    options: &SeqOptions,
    user_format: Option<GnuFloatFormat>,
    first: &PreciseNumber,
    increment: &PreciseNumber,
    last: &PreciseNumber,
    padding: usize,
    precision: usize,
) -> CTResult<SeqOutputFormat> {
    if let Some(format) = user_format {
        return Ok(SeqOutputFormat::Float(format));
    }
    if uses_exact_integer_output(first, increment, last, options) {
        return Ok(SeqOutputFormat::ExactInteger);
    }

    let format =
        if !is_fixed_decimal(first) || !is_fixed_decimal(increment) || !is_fixed_decimal(last) {
            "%g".to_string()
        } else if options.is_equal_width {
            let width = padding + if precision > 0 { precision + 1 } else { 0 };
            format!("%0{width}.{precision}f")
        } else {
            format!("%.{precision}f")
        };
    Ok(SeqOutputFormat::Float(
        GnuFloatFormat::try_parse(format.as_bytes()).expect("generated seq format must be valid"),
    ))
}

fn is_fixed_decimal(number: &PreciseNumber) -> bool {
    number.is_fixed_precision
        || matches!(
            number.number,
            ExtendedBigDecimal::Infinity | ExtendedBigDecimal::MinusInfinity
        )
}

fn uses_exact_integer_output(
    first: &PreciseNumber,
    increment: &PreciseNumber,
    last: &PreciseNumber,
    options: &SeqOptions,
) -> bool {
    if options.is_equal_width
        || options.separator.as_bytes().len() != 1
        || !first.is_fixed_precision
        || !increment.is_fixed_precision
        || !(last.is_fixed_precision || matches!(last.number, ExtendedBigDecimal::Infinity))
        || first.num_fractional_digits != 0
        || increment.num_fractional_digits != 0
        || last.num_fractional_digits != 0
        || first.number < ExtendedBigDecimal::zero()
        || last.number < ExtendedBigDecimal::zero()
        || !matches!(&first.number, ExtendedBigDecimal::BigDecimal(value) if value.is_integer())
        || !matches!(&increment.number, ExtendedBigDecimal::BigDecimal(value) if value.is_integer())
        || !(matches!(
            &last.number,
            ExtendedBigDecimal::BigDecimal(value) if value.is_integer()
        ) || matches!(&last.number, ExtendedBigDecimal::Infinity))
    {
        return false;
    }

    matches!(
        &increment.number,
        ExtendedBigDecimal::BigDecimal(value)
            if value.to_u64().is_some_and(|step| step > 0 && step <= SEQ_FAST_STEP_LIMIT)
    )
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(SEQ_SEPARATOR)
            .short('s')
            .long("separator")
            .allow_hyphen_values(true)
            .value_parser(OsStringValueParser::new())
            .overrides_with(SEQ_SEPARATOR)
            .help(t!("seq.clap.seq_separator")),
        Arg::new(SEQ_TERMINATOR)
            .short('t')
            .long("terminator")
            .help(t!("seq.clap.seq_terminator")),
        Arg::new(SEQ_EQUAL_WIDTH)
            .short('w')
            .long("equal-width")
            .overrides_with(SEQ_EQUAL_WIDTH)
            .help(t!("seq.clap.seq_equal_width"))
            .action(ArgAction::SetTrue),
        Arg::new(SEQ_FORMAT)
            .short('f')
            .long(SEQ_FORMAT)
            .allow_hyphen_values(true)
            .value_parser(OsStringValueParser::new())
            .overrides_with(SEQ_FORMAT)
            .help(t!("seq.clap.seq_format")),
        Arg::new(SEQ_NUMBERS)
            .value_parser(OsStringValueParser::new())
            .action(ArgAction::Append)
            .num_args(1..),
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
    separator: &[u8],
    terminator: &str,
) -> std::io::Result<()> {
    use std::io::BufWriter;

    let stdout = stdout();
    let mut writer = BufWriter::with_capacity(8192, stdout.lock());
    let mut current = first;
    let mut is_first = true;

    while current <= last {
        if !is_first {
            writer.write_all(separator)?;
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

    if options.format.is_some() || options.is_equal_width || options.separator.as_bytes().len() != 1
    {
        return None;
    }

    // Check if all are integers (precision == 0)
    if !first.is_fixed_precision
        || !increment.is_fixed_precision
        || !last.is_fixed_precision
        || first.num_fractional_digits != 0
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
    separator: &'a [u8],
    terminator: &'a str,
    pad: bool,
    padding: usize,
    format: &'a SeqOutputFormat,
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
        SeqOutputFormat::Float(f) => {
            format_long_double(&mut buffer, f, value)?;
        }
        SeqOutputFormat::ExactInteger => write_value_float(&mut buffer, value, padding, 0)?,
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn walk_sequence(
    range: RangeFloat,
    format: &SeqOutputFormat,
    mut emit: impl FnMut(&ExtendedBigDecimal) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let (first, increment, last) = range;
    if matches!(format, SeqOutputFormat::ExactInteger) {
        let mut value = first;
        while !done_printing(&value, &increment, &last) {
            emit(&value)?;
            value = value + increment.clone();
        }
        return Ok(());
    }

    let SeqOutputFormat::Float(format) = format else {
        unreachable!()
    };
    let first = quantize_long_double(&first);
    let increment = quantize_long_double(&increment);
    let last = quantize_long_double(&last);
    if done_printing(&first, &increment, &last) {
        return Ok(());
    }

    let mut index = 0_u64;
    let mut value = first.clone();
    loop {
        emit(&value)?;
        let Some(next_index) = index.checked_add(1) else {
            break;
        };
        let next = long_double_linear_value(&first, &increment, next_index);
        if done_printing(&next, &increment, &last) {
            if should_print_extra_number(format, &value, &next, &last) {
                emit(&next)?;
            }
            break;
        }
        value = next;
        index = next_index;
    }
    Ok(())
}

fn should_print_extra_number(
    format: &GnuFloatFormat,
    current: &ExtendedBigDecimal,
    next: &ExtendedBigDecimal,
    last: &ExtendedBigDecimal,
) -> bool {
    let next_text = format.format_unlocalized_numeric(next);
    let Ok(parsed) = next_text.parse::<PreciseNumber>() else {
        return false;
    };
    quantize_long_double(&parsed.number) == *last
        && next_text != format.format_unlocalized_numeric(current)
}

fn collect_seq_rows(
    range: RangeFloat,
    config: &SeqRenderConfig<'_>,
    rows: &mut Vec<SeqRow>,
    writer: &mut Vec<u8>,
) -> std::io::Result<()> {
    let mut is_first_iteration = true;
    let mut index = 0usize;

    walk_sequence(range, config.format, |value| {
        let rendered = render_seq_value(value, config)?;
        if !is_first_iteration {
            writer.write_all(config.separator)?;
        }
        write!(writer, "{rendered}")?;
        rows.push(SeqRow {
            index,
            value: rendered,
        });
        is_first_iteration = false;
        index += 1;
        Ok(())
    })?;
    if !is_first_iteration {
        write!(writer, "{}", config.terminator)?;
    }
    Ok(())
}

fn format_long_double(
    writer: &mut impl Write,
    format: &GnuFloatFormat,
    value: &ExtendedBigDecimal,
) -> std::io::Result<()> {
    writer.write_all(&format.format(value))
}

/// Floating point based code path
fn print_seq(range: RangeFloat, config: PrintConfig) -> std::io::Result<()> {
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
    walk_sequence(range, config.format, |value| {
        if !is_first_iteration {
            writer.write_all(config.separator)?;
        }
        match config.format {
            SeqOutputFormat::Float(f) => {
                format_long_double(&mut writer, f, value)?;
            }
            SeqOutputFormat::ExactInteger => write_value_float(&mut writer, value, padding, 0)?,
        }
        is_first_iteration = false;
        Ok(())
    })?;
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
    use std::os::unix::ffi::OsStringExt;

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
    fn test_repeated_separator_uses_last_value() {
        for (args, expected) in [
            (["seq", "-s", ",", "--separator=:", "1", "3"], ":"),
            (["seq", "--separator=:", "-s", ",", "1", "3"], ","),
        ] {
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let options = SeqOptions::new(&matches);

            assert_eq!(options.separator, expected);
        }
    }

    #[test]
    fn test_repeated_format_uses_last_value() {
        for (args, expected) in [
            (["seq", "-f", "%.1f", "--format=%.2f", "1", "2"], "%.2f"),
            (["seq", "--format=%.2f", "-f", "%.1f", "1", "2"], "%.1f"),
        ] {
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let options = SeqOptions::new(&matches);

            assert_eq!(options.format.as_deref(), Some(OsStr::new(expected)));
        }
    }

    #[test]
    fn test_separator_accepts_a_hyphen_prefixed_value() {
        let matches = ct_app()
            .try_get_matches_from(["seq", "-s", "-w", "1", "2"])
            .unwrap();
        let options = SeqOptions::new(&matches);

        assert_eq!(options.separator, "-w");
        assert!(!options.is_equal_width);
    }

    #[test]
    fn test_format_accepts_a_hyphen_prefixed_value() {
        let matches = ct_app()
            .try_get_matches_from(["seq", "-f", "-w", "1", "2"])
            .unwrap();
        let options = SeqOptions::new(&matches);

        assert_eq!(options.format.as_deref(), Some(OsStr::new("-w")));
    }

    #[test]
    fn test_separator_preserves_literal_internal_sentinel_prefix() {
        let args = mask_negative_number_args(
            ["seq", "-s", "CT_NEG_,", "1", "3"]
                .map(OsString::from)
                .to_vec(),
        );
        let matches = ct_app().try_get_matches_from(args).unwrap();

        assert_eq!(SeqOptions::new(&matches).separator, "CT_NEG_,");
    }

    #[test]
    fn test_format_preserves_literal_internal_sentinel_prefix() {
        let args = mask_negative_number_args(
            ["seq", "-f", "CT_NEG_%g", "1", "1"]
                .map(OsString::from)
                .to_vec(),
        );
        let matches = ct_app().try_get_matches_from(args).unwrap();

        assert_eq!(
            SeqOptions::new(&matches).format.as_deref(),
            Some(OsStr::new("CT_NEG_%g"))
        );
    }

    #[test]
    fn test_negative_prefixed_non_utf8_option_value_is_not_reencoded() {
        let args = mask_negative_number_args(vec![
            OsString::from("seq"),
            OsString::from("-s"),
            OsString::from_vec(b"-1\xff".to_vec()),
            OsString::from("1"),
            OsString::from("2"),
        ]);
        let matches = ct_app().try_get_matches_from(args).unwrap();

        assert_eq!(SeqOptions::new(&matches).separator.as_bytes(), b"-1\xff");
    }

    #[test]
    fn test_option_parse_errors_use_gnu_diagnostics() {
        for (args, message) in [
            (&["seq", "-s"][..], "option requires an argument -- 's'"),
            (
                &["seq", "--separator"][..],
                "option '--separator' requires an argument",
            ),
            (&["seq", "-x"][..], "invalid option -- 'x'"),
            (&["seq", "--unknown"][..], "unrecognized option '--unknown'"),
            (
                &["seq", "--equal-width=bad"][..],
                "option '--equal-width' doesn't allow an argument",
            ),
            (&["seq", "-w=bad"][..], "invalid option -- '='"),
            (
                &["seq", "--=bad"][..],
                "option '--=bad' is ambiguous; possibilities: '--equal-width' '--format' '--separator' '--help' '--version'",
            ),
        ] {
            let error = seq_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(error.to_string(), message, "args: {args:?}");
            assert!(error.usage(), "args: {args:?}");
        }
    }

    #[test]
    fn test_infinite_endpoint_preserves_finite_default_precision() {
        for (first_text, increment_text, expected) in [("1", ".1", "1.0"), ("1.00", "1", "1.00")] {
            let first = first_text.parse::<PreciseNumber>().unwrap();
            let increment = increment_text.parse::<PreciseNumber>().unwrap();
            let last = "inf".parse::<PreciseNumber>().unwrap();
            let format = select_output_format(
                &SeqOptions::default(),
                None,
                &first,
                &increment,
                &last,
                calculate_padding(&first, &last),
                calculate_largest_decimal(&first, &increment),
            )
            .unwrap();

            let SeqOutputFormat::Float(format) = format else {
                panic!("an infinite endpoint must use floating-point output");
            };
            assert_eq!(format.format_unlocalized_numeric(&first.number), expected);
        }
    }

    #[test]
    fn test_hex_float_uses_general_default_format() {
        for (input, expected) in [("0x1.8p-1", "0.75"), ("0x1.8", "1.5")] {
            let first = input.parse::<PreciseNumber>().unwrap();
            let increment = PreciseNumber::one();
            let last = input.parse::<PreciseNumber>().unwrap();
            let matches = ct_app()
                .try_get_matches_from(["seq", input, input])
                .unwrap();
            let options = SeqOptions::new(&matches);

            assert!(
                !uses_exact_integer_output(&first, &increment, &last, &options),
                "input: {input}"
            );
            let format = select_output_format(
                &options,
                None,
                &first,
                &increment,
                &last,
                calculate_padding(&first, &last),
                calculate_largest_decimal(&first, &increment),
            )
            .unwrap();

            let SeqOutputFormat::Float(format) = format else {
                panic!("a hexadecimal float must use %g");
            };
            assert_eq!(format.format_unlocalized_numeric(&first.number), expected);
        }
    }

    #[test]
    fn test_tiny_hex_float_underflows_before_sequence_generation() {
        let number = parse_number_arg("0x1p-100001").unwrap();

        assert!(number.is_zero());
        assert!(!number.is_fixed_precision);
    }

    #[test]
    fn test_repeated_equal_width_is_accepted() {
        let matches = ct_app()
            .try_get_matches_from(["seq", "-w", "--equal-width", "1", "3"])
            .unwrap();

        assert!(SeqOptions::new(&matches).is_equal_width);
    }

    #[test]
    fn test_equal_width_conflicts_with_format() {
        for args in [
            ["seq", "-w", "-f", "%f", "1", "2"],
            ["seq", "-f", "%f", "-w", "1", "2"],
        ] {
            let error = seq_main(args.into_iter().map(OsString::from)).unwrap_err();

            assert_eq!(error.code(), 1);
            assert_eq!(
                error.to_string(),
                "format string may not be specified when printing equal width strings"
            );
        }
    }

    #[test]
    fn test_invalid_format_is_reported_before_operand_and_width_errors() {
        for (args, expected) in [
            (
                &["seq", "-f", "bad", "invalid"][..],
                "format 'bad' has no % directive",
            ),
            (
                &["seq", "-wfoo", "1", "2"][..],
                "format 'oo' has no % directive",
            ),
        ] {
            let error = seq_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(error.to_string(), expected, "args: {args:?}");
            assert!(!error.usage(), "args: {args:?}");
        }
    }

    #[test]
    fn test_invalid_non_utf8_format_preserves_gnu_diagnostic_bytes() {
        let no_directive = seq_main(
            vec![
                OsString::from("seq"),
                OsString::from("-f"),
                OsString::from_vec(vec![0xff]),
                OsString::from("1"),
            ]
            .into_iter(),
        )
        .unwrap_err();
        assert_eq!(
            no_directive.diagnostic_bytes().as_ref(),
            b"format '\\377' has no % directive"
        );

        let unknown_directive = seq_main(
            vec![
                OsString::from("seq"),
                OsString::from("-f"),
                OsString::from_vec(vec![b'%', 0xff]),
                OsString::from("1"),
            ]
            .into_iter(),
        )
        .unwrap_err();
        assert_eq!(
            unknown_directive.diagnostic_bytes().as_ref(),
            b"format '%\\377' has unknown %\xff directive"
        );
    }

    #[test]
    fn test_negative_prefixed_non_utf8_operand_preserves_gnu_diagnostic_bytes() {
        let error = seq_main(
            vec![
                OsString::from("seq"),
                OsString::from_vec(b"-1\xff".to_vec()),
            ]
            .into_iter(),
        )
        .unwrap_err();

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"invalid floating point argument: '-1\\377'"
        );
    }

    #[test]
    fn test_extra_operand_reports_the_fourth_value() {
        let error = seq_main(
            ["seq", "1", "2", "3", "four"]
                .map(OsString::from)
                .into_iter(),
        )
        .unwrap_err();

        assert_eq!(error.code(), 1);
        assert_eq!(error.to_string(), "extra operand 'four'");
        assert!(error.usage());
    }

    #[test]
    fn test_rejects_long_double_overflow() {
        for value in ["2e4932", "1e4933", "-2e4932"] {
            let error = seq_main(["seq", value].map(OsString::from).into_iter()).unwrap_err();

            assert_eq!(error.code(), 1, "input: {value}");
            assert_eq!(
                error.to_string(),
                format!("invalid floating point argument: '{value}'")
            );
        }
    }

    #[test]
    fn test_rejects_rust_style_numeric_underscores() {
        for value in ["1_000", "1_0.5", "1_0e1", "0x1_0", "0x1._0"] {
            let error = seq_main(["seq", value].map(OsString::from).into_iter()).unwrap_err();

            assert_eq!(error.code(), 1, "input: {value}");
            assert_eq!(
                error.to_string(),
                format!("invalid floating point argument: '{value}'"),
                "input: {value}"
            );
        }
    }

    #[test]
    fn test_locale_decimal_operands_preserve_gnu_layout() {
        let integer = parse_number_arg_with_decimal_point("1,0", ",").unwrap();
        assert_eq!(integer.num_integral_digits, 3);
        assert_eq!(integer.num_fractional_digits, 0);

        let fractional = parse_number_arg_with_decimal_point("0,1", ",").unwrap();
        assert_eq!(fractional.num_integral_digits, 3);
        assert_eq!(fractional.num_fractional_digits, 0);
        assert!(!uses_exact_integer_output(
            &fractional,
            &PreciseNumber::one(),
            &fractional,
            &SeqOptions::default()
        ));
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
    fn test_equal_width_ignores_increment_sign_width() {
        let first = "0.00916".parse::<PreciseNumber>().unwrap();
        let increment = "-0.00004".parse::<PreciseNumber>().unwrap();
        let last = "0.00912".parse::<PreciseNumber>().unwrap();

        assert_eq!(calculate_padding(&first, &last), 1);
        assert_eq!(increment.num_integral_digits, 2);
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
                separator: b",",
                terminator: "\n",
                pad: false,
                padding: 1,
                format: &SeqOutputFormat::ExactInteger,
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
                separator: b"\n",
                terminator: "\n",
                pad: true,
                padding: 2,
                format: &SeqOutputFormat::ExactInteger,
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
    fn test_native_semantic_prints_integers_beyond_long_double_range() {
        let first = "9".repeat(5000);
        let last = format!("1{}", "0".repeat(5000));
        let expected = format!("{first}\n{last}\n");

        let semantic = seq_native_semantic(
            [
                OsString::from("seq"),
                first.clone().into(),
                last.clone().into(),
            ]
            .into_iter(),
        )
        .unwrap();

        assert_eq!(semantic.classic_text, expected);
        assert_eq!(
            semantic.rows,
            vec![
                SeqRow {
                    index: 0,
                    value: first,
                },
                SeqRow {
                    index: 1,
                    value: last,
                },
            ]
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
