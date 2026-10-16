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
use std::borrow::Cow;
use std::ffi::{CStr, OsStr};

use bigdecimal::{BigDecimal, RoundingMode};
use ctcore::ct_quoting_style::{CtQuotes, CtQuotingStyle, escape_name};

use crate::flags::{NumfmtConfigs, NumfmtRoundMethod, NumfmtTransformOptions};
use crate::units::DisplayableSuffix;
use crate::units::NUMFMT_IEC_BASES;
use crate::units::NUMFMT_SI_BASES;
use crate::units::NumfmtRawSuffix;
use crate::units::NumfmtSuffix;
use crate::units::NumfmtUnit;
use crate::units::Result;

const NUMFMT_MAX_UNSCALED_DIGITS: usize = 18;
const NUMFMT_MAX_ACCEPTABLE_DIGITS: usize = 33;
const NUMFMT_DIAGNOSTIC_QUOTING_STYLE: &CtQuotingStyle = &CtQuotingStyle::C {
    quotes: CtQuotes::Single,
};

fn numfmt_quote(text: &str) -> String {
    escape_name(OsStr::new(text), NUMFMT_DIAGNOSTIC_QUOTING_STYLE)
}

fn numfmt_locale_decimal_point() -> String {
    unsafe {
        let locale = ctcore::libc::localeconv();
        if locale.is_null() || (*locale).decimal_point.is_null() {
            return ".".to_string();
        }

        let decimal_point = CStr::from_ptr((*locale).decimal_point).to_bytes();
        if decimal_point.is_empty() {
            ".".to_string()
        } else {
            String::from_utf8_lossy(decimal_point).into_owned()
        }
    }
}

/// 遍历一行的字段，其中每个字段都是一个连续的非分隔符序列，
/// 可选择以一个或多个前导分隔符作为前缀。字段以 `(prefix, field)`
/// 的元组形式返回。
///
/// # Examples:
///
/// ```
/// let mut fields = ct_numfmt::format::NumfmtWhitespaceSplitter { s: Some("    1234 5") };
///
/// assert_eq!(Some(("    ", "1234")), fields.next());
/// assert_eq!(Some((" ", "5")), fields.next());
/// assert_eq!(None, fields.next());
/// ```
///
/// 结果中包含分隔符；`prefix`仅在行的第一个字段为空（包括输入行为空的情况）：
///
/// ```
/// let mut fields = ct_numfmt::format::NumfmtWhitespaceSplitter { s: Some("first second") };
///
/// assert_eq!(Some(("", "first")), fields.next());
/// assert_eq!(Some((" ", "second")), fields.next());
///
/// let mut fields = ct_numfmt::format::NumfmtWhitespaceSplitter { s: Some("") };
///
/// assert_eq!(Some(("", "")), fields.next());
/// ```
pub struct NumfmtWhitespaceSplitter<'a> {
    pub s: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumfmtAbortRenderError {
    pub partial_output: String,
    pub error: String,
}

fn numfmt_is_blank(ch: char) -> bool {
    matches!(ch, ' ' | '\t')
}

fn numfmt_is_field_separator(ch: char) -> bool {
    numfmt_is_blank(ch) || ch == '\n'
}

impl<'a> Iterator for NumfmtWhitespaceSplitter<'a> {
    type Item = (&'a str, &'a str);

    /// 生成输入字符串中的下一个字段，作为一个元组 `(prefix, field)`。
    fn next(&mut self) -> Option<Self::Item> {
        let haystack = self.s?;

        let (prefix, field_tmp) = haystack.split_at(
            haystack
                .find(|c: char| !numfmt_is_field_separator(c))
                .unwrap_or(haystack.len()),
        );

        let (field, rest) = field_tmp.split_at(
            field_tmp
                .find(numfmt_is_field_separator)
                .unwrap_or(field_tmp.len()),
        );

        self.s = match rest.is_empty() {
            true => None,
            false => Some(rest),
        };

        Some((prefix, field))
    }
}

fn parse_numeric_prefix_with_decimal_point(s: &str, decimal_point: &str) -> Option<(usize, bool)> {
    let mut idx = 0;
    let decimal_point = if decimal_point.is_empty() {
        "."
    } else {
        decimal_point
    };

    if s.starts_with('-') {
        idx = 1;
    }

    let mut digits_before = 0usize;
    while let Some(c) = s[idx..].chars().next() {
        if c.is_ascii_digit() {
            digits_before += 1;
            idx += c.len_utf8();
        } else {
            break;
        }
    }

    let mut has_dot = false;
    let mut digits_after = 0usize;
    if s[idx..].starts_with(decimal_point) {
        has_dot = true;
        idx += decimal_point.len();

        while let Some(c) = s[idx..].chars().next() {
            if c.is_ascii_digit() {
                digits_after += 1;
                idx += c.len_utf8();
            } else {
                break;
            }
        }
    }

    if digits_before == 0 && digits_after == 0 {
        return None;
    }

    Some((idx, has_dot && digits_after == 0))
}

fn parse_numeric_prefix(s: &str) -> Option<(usize, bool)> {
    parse_numeric_prefix_with_decimal_point(s, &numfmt_locale_decimal_point())
}

fn numfmt_normalize_decimal_point_with<'a>(number: &'a str, decimal_point: &str) -> Cow<'a, str> {
    if decimal_point == "." || decimal_point.is_empty() {
        Cow::Borrowed(number)
    } else if let Some(index) = number.find(decimal_point) {
        let mut normalized = String::with_capacity(number.len() - decimal_point.len() + 1);
        normalized.push_str(&number[..index]);
        normalized.push('.');
        normalized.push_str(&number[index + decimal_point.len()..]);
        Cow::Owned(normalized)
    } else {
        Cow::Borrowed(number)
    }
}

fn numfmt_normalize_decimal_point(number: &str) -> Cow<'_, str> {
    numfmt_normalize_decimal_point_with(number, &numfmt_locale_decimal_point())
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NumfmtParsedNumber<'a> {
    number: &'a str,
    suffix: Option<NumfmtSuffix>,
}

fn numfmt_parse_number_parts(s: &str) -> Result<NumfmtParsedNumber<'_>> {
    if s.is_empty() {
        return Err("invalid number: ''".to_string());
    }

    let s = s.trim_matches(numfmt_is_blank);
    if s.is_empty() {
        return Err("invalid number: ''".to_string());
    }

    let (num_end, invalid_fraction) = match parse_numeric_prefix(s) {
        Some(v) => v,
        None => {
            if s.starts_with("..") {
                return Err(format!("invalid suffix in input: {}", numfmt_quote(s)));
            }
            return Err(format!("invalid number: {}", numfmt_quote(s)));
        }
    };

    if invalid_fraction {
        return Err(format!("invalid number: {}", numfmt_quote(s)));
    }

    let number = &s[..num_end];

    let rest = s[num_end..].trim_start_matches(numfmt_is_blank);
    if rest.is_empty() {
        return Ok(NumfmtParsedNumber {
            number,
            suffix: None,
        });
    }

    let raw_suffix = match rest.chars().next() {
        Some('K') => NumfmtRawSuffix::K,
        Some('M') => NumfmtRawSuffix::M,
        Some('G') => NumfmtRawSuffix::G,
        Some('T') => NumfmtRawSuffix::T,
        Some('P') => NumfmtRawSuffix::P,
        Some('E') => NumfmtRawSuffix::E,
        Some('Z') => NumfmtRawSuffix::Z,
        Some('Y') => NumfmtRawSuffix::Y,
        Some('R') => NumfmtRawSuffix::R,
        Some('Q') => NumfmtRawSuffix::Q,
        _ => return Err(format!("invalid suffix in input: {}", numfmt_quote(s))),
    };

    let mut suffix_len = 1usize;
    let mut with_i = false;
    if rest.chars().nth(1) == Some('i') {
        with_i = true;
        suffix_len = 2;
    }

    let trailing = rest[suffix_len..].trim_matches(numfmt_is_blank);
    if !trailing.is_empty() {
        return Err(format!(
            "invalid suffix in input {}: {}",
            numfmt_quote(s),
            numfmt_quote(trailing)
        ));
    }

    Ok(NumfmtParsedNumber {
        number,
        suffix: Some((raw_suffix, with_i)),
    })
}

#[cfg(test)]
fn numfmt_parse_suffix(s: &str) -> Result<(f64, Option<NumfmtSuffix>)> {
    let parsed = numfmt_parse_number_parts(s)?;
    let number = numfmt_normalize_decimal_point(parsed.number)
        .parse::<f64>()
        .map_err(|_| format!("invalid number: {}", numfmt_quote(s)))?;

    Ok((number, parsed.suffix))
}

fn numfmt_suffix_power(raw_suffix: NumfmtRawSuffix) -> usize {
    match raw_suffix {
        NumfmtRawSuffix::K => 1,
        NumfmtRawSuffix::M => 2,
        NumfmtRawSuffix::G => 3,
        NumfmtRawSuffix::T => 4,
        NumfmtRawSuffix::P => 5,
        NumfmtRawSuffix::E => 6,
        NumfmtRawSuffix::Z => 7,
        NumfmtRawSuffix::Y => 8,
        NumfmtRawSuffix::R => 9,
        NumfmtRawSuffix::Q => 10,
    }
}

fn numfmt_dev_general(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }

    let exponent = value.abs().log10().floor() as i32;
    if !(-4..6).contains(&exponent) {
        let scientific = format!("{value:.5E}");
        let (mantissa, exponent) = scientific.split_once('E').unwrap();
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        let exponent = exponent.parse::<i32>().unwrap();
        return format!("{mantissa}E{exponent:+03}");
    }

    let decimal_places = (5 - exponent).max(0) as usize;
    let fixed = format!("{value:.decimal_places$}");
    if fixed.contains('.') {
        fixed
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else {
        fixed
    }
}

// 返回数字的隐式精度，即点后面的位数。例如
// 例如，1.23 的隐含精度为 2。
fn numfmt_parse_implicit_precision(s: &str) -> usize {
    let decimal_point = numfmt_locale_decimal_point();
    if let Some((_, decimal_part_value)) = s.split_once(&decimal_point) {
        decimal_part_value
            .chars()
            .take_while(char::is_ascii_digit)
            .count()
    } else {
        0
    }
}

fn numfmt_parse_input_precision(s: &str) -> Result<usize> {
    let parsed = numfmt_parse_number_parts(s)?;

    // GNU resets the input precision only when it consumes a scale suffix.
    Ok(if parsed.suffix.is_some() {
        0
    } else {
        numfmt_parse_implicit_precision(parsed.number)
    })
}

fn numfmt_remove_suffix(i: f64, s: Option<NumfmtSuffix>, u: &NumfmtUnit) -> Result<f64> {
    match (s, u) {
        (Some((raw_suffix, false)), &NumfmtUnit::Auto)
        | (Some((raw_suffix, false)), &NumfmtUnit::Si) => match raw_suffix {
            NumfmtRawSuffix::K => Ok(i * 1e3),
            NumfmtRawSuffix::M => Ok(i * 1e6),
            NumfmtRawSuffix::G => Ok(i * 1e9),
            NumfmtRawSuffix::T => Ok(i * 1e12),
            NumfmtRawSuffix::P => Ok(i * 1e15),
            NumfmtRawSuffix::E => Ok(i * 1e18),
            NumfmtRawSuffix::Z => Ok(i * 1e21),
            NumfmtRawSuffix::Y => Ok(i * 1e24),
            NumfmtRawSuffix::R => Ok(i * 1e27),
            NumfmtRawSuffix::Q => Ok(i * 1e30),
        },
        (Some((raw_suffix, false)), &NumfmtUnit::Iec(false))
        | (Some((raw_suffix, true)), &NumfmtUnit::Auto)
        | (Some((raw_suffix, true)), &NumfmtUnit::Iec(true)) => match raw_suffix {
            NumfmtRawSuffix::K => Ok(i * NUMFMT_IEC_BASES[1]),
            NumfmtRawSuffix::M => Ok(i * NUMFMT_IEC_BASES[2]),
            NumfmtRawSuffix::G => Ok(i * NUMFMT_IEC_BASES[3]),
            NumfmtRawSuffix::T => Ok(i * NUMFMT_IEC_BASES[4]),
            NumfmtRawSuffix::P => Ok(i * NUMFMT_IEC_BASES[5]),
            NumfmtRawSuffix::E => Ok(i * NUMFMT_IEC_BASES[6]),
            NumfmtRawSuffix::Z => Ok(i * NUMFMT_IEC_BASES[7]),
            NumfmtRawSuffix::Y => Ok(i * NUMFMT_IEC_BASES[8]),
            NumfmtRawSuffix::R => Ok(i * NUMFMT_IEC_BASES[9]),
            NumfmtRawSuffix::Q => Ok(i * NUMFMT_IEC_BASES[10]),
        },
        (None, &NumfmtUnit::Iec(true)) => Ok(i),
        (Some((raw_suffix, false)), &NumfmtUnit::Iec(true)) => Err(format!(
            "missing 'i' suffix in input: '{i}{raw_suffix:?}' (e.g Ki/Mi/Gi)"
        )),
        (Some((raw_suffix, with_i)), &NumfmtUnit::None) => Err(format!(
            "rejecting suffix in input: '{}{:?}{}' (consider using --from)",
            i,
            raw_suffix,
            if with_i { "i" } else { "" }
        )),
        (None, _) => Ok(i),
        (_, _) => Err("This suffix is unsupported for specified unit".to_owned()),
    }
}

fn numfmt_transform_from(s: &str, numfmt_opts: &NumfmtTransformOptions) -> Result<f64> {
    let decimal_point = numfmt_locale_decimal_point();
    crate::numfmt_dev_trace(format_args!(
        "simple_strtod_human:\n  input string: {}\n  locale decimal-point: {}\n  MAX_UNSCALED_DIGITS: {}\n",
        numfmt_quote(s),
        numfmt_quote(&decimal_point),
        NUMFMT_MAX_UNSCALED_DIGITS,
    ));

    let parsed = numfmt_parse_number_parts(s)?;
    let i = numfmt_normalize_decimal_point(parsed.number)
        .parse::<f64>()
        .map_err(|_| format!("invalid number: {}", numfmt_quote(s)))?;
    let input_precision = numfmt_parse_implicit_precision(parsed.number);
    crate::numfmt_dev_trace(format_args!(
        "  parsed numeric value: {i:.6}\n  input precision = {input_precision}\n"
    ));

    let suffix = parsed.suffix;
    let mut scale_base = match numfmt_opts.from {
        NumfmtUnit::Iec(_) => 1024.0_f64,
        NumfmtUnit::Auto | NumfmtUnit::Si | NumfmtUnit::None => 1000.0_f64,
    };
    let power = suffix.map_or(0, |(raw_suffix, _)| numfmt_suffix_power(raw_suffix));
    if matches!(suffix, Some((_, true))) && numfmt_opts.from == NumfmtUnit::Auto {
        scale_base = 1024.0;
        crate::numfmt_dev_trace(format_args!(
            "  Auto-scaling, found 'i', switching to base {scale_base:.0}\n"
        ));
    }

    let i = i * (numfmt_opts.from_unit as f64);
    numfmt_remove_suffix(i, suffix, &numfmt_opts.from).map(|number| {
        let multiplier = scale_base.powi(power as i32);
        let traced_value = if numfmt_opts.from_unit == 0 {
            0.0
        } else {
            number / (numfmt_opts.from_unit as f64)
        };
        crate::numfmt_dev_trace(format_args!(
            "  suffix power={:.0}^{power} = {multiplier:.6}\n  returning value: {traced_value:.6} ({})\n",
            scale_base,
            numfmt_dev_general(traced_value),
        ));

        if numfmt_opts.from == NumfmtUnit::None && number == -0.0 {
            0.0
        } else {
            number
        }
    })
}

fn numfmt_significant_digits(digits: &str) -> usize {
    digits.trim_start_matches('0').len()
}

fn numfmt_number_may_lose_precision(number: &str) -> bool {
    let normalized = numfmt_normalize_decimal_point(number);
    let unsigned = normalized.strip_prefix('-').unwrap_or(&normalized);
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let integer_digits = numfmt_significant_digits(integer);
    let fraction_digits = numfmt_significant_digits(fraction);

    integer_digits <= NUMFMT_MAX_ACCEPTABLE_DIGITS
        && fraction_digits <= NUMFMT_MAX_ACCEPTABLE_DIGITS
        && (integer_digits > NUMFMT_MAX_UNSCALED_DIGITS
            || fraction_digits > NUMFMT_MAX_UNSCALED_DIGITS)
}

fn numfmt_precision_loss_input(source: &str, options: &NumfmtConfigs) -> Option<String> {
    let source_without_suffix = match &options.suffix {
        Some(suffix) if source.len() > suffix.len() => {
            source.strip_suffix(suffix).unwrap_or(source)
        }
        _ => source,
    };
    let input = source_without_suffix.trim_start();

    if options.delimiter.is_some()
        && numfmt_delimited_trailing_blank_error(input, options).is_some()
    {
        return None;
    }

    let parsed = numfmt_parse_number_parts(input).ok()?;
    let number = numfmt_normalize_decimal_point(parsed.number)
        .parse::<f64>()
        .ok()?
        * (options.transform.from_unit as f64);

    // GNU only reports precision loss after the complete number, including its
    // scale suffix, has been accepted by the input parser.
    numfmt_remove_suffix(number, parsed.suffix, &options.transform.from).ok()?;

    numfmt_number_may_lose_precision(parsed.number).then(|| input.to_string())
}

fn numfmt_nul_delimited_first_field(input: &str) -> &str {
    input.split_once('\0').map_or(input, |(field, _)| field)
}

fn numfmt_push_unselected_whitespace_field(
    out: &mut String,
    field_number: usize,
    prefix: &str,
    field: &str,
) {
    if field_number > 1 {
        out.push(' ');
        out.push_str(&prefix[1..]);
    } else {
        out.push_str(prefix);
    }
    out.push_str(field);
}

fn numfmt_normalize_whitespace_separators(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for (field_number, (prefix, field)) in (1..).zip(NumfmtWhitespaceSplitter { s: Some(input) }) {
        numfmt_push_unselected_whitespace_field(&mut out, field_number, prefix, field);
    }
    out
}

pub(crate) fn numfmt_passthrough_input<'a>(
    input: &'a str,
    options: &NumfmtConfigs,
) -> Cow<'a, str> {
    match &options.delimiter {
        Some(delimiter) if delimiter.is_empty() => {
            Cow::Borrowed(numfmt_nul_delimited_first_field(input))
        }
        Some(_) => Cow::Borrowed(input),
        None => Cow::Owned(numfmt_normalize_whitespace_separators(input)),
    }
}

pub(crate) fn numfmt_precision_loss_inputs(
    input_line: &str,
    options: &NumfmtConfigs,
) -> Vec<String> {
    let mut inputs = Vec::new();
    let mut check_field = |field: &str| {
        if let Some(input) = numfmt_precision_loss_input(field, options) {
            inputs.push(input);
        }
    };

    if let Some(delimiter) = &options.delimiter {
        if delimiter.is_empty() {
            if ctcore::ct_ranges::contain(&options.fields, 1) {
                check_field(numfmt_nul_delimited_first_field(input_line).trim_start());
            }
        } else {
            for (index, field) in (1..).zip(input_line.split(delimiter)) {
                if ctcore::ct_ranges::contain(&options.fields, index) {
                    check_field(field.trim_start());
                }
            }
        }
    } else {
        for (index, (_, field)) in (1..).zip(NumfmtWhitespaceSplitter {
            s: Some(input_line),
        }) {
            if ctcore::ct_ranges::contain(&options.fields, index) {
                check_field(field);
            }
        }
    }

    inputs
}

/// 用分子除以分母，四舍五入。
///
/// 如果除法的结果小于 10.0，则四舍五入到小数点后一位。
///
/// 否则，四舍五入为整数。
///
/// # 例子:
///
/// ```
/// use ct_numfmt::format::numfmt_div_round;
/// use ct_numfmt::flags::NumfmtRoundMethod;
///
/// // 四舍五入方法：
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::FromZero), 1.1);
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::TowardsZero), 1.0);
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::Up), 1.1);
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::Down), 1.0);
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::Nearest), 1.0);
///
/// // Division:
/// assert_eq!(numfmt_div_round(999.1, 1000.0, NumfmtRoundMethod::FromZero), 1.0);
/// assert_eq!(numfmt_div_round(1001., 10., NumfmtRoundMethod::FromZero), 101.);
/// assert_eq!(numfmt_div_round(9991., 10., NumfmtRoundMethod::FromZero), 1000.);
/// assert_eq!(numfmt_div_round(-12.34, 1.0, NumfmtRoundMethod::FromZero), -13.0);
/// assert_eq!(numfmt_div_round(1000.0, -3.14, NumfmtRoundMethod::FromZero), -319.0);
/// assert_eq!(numfmt_div_round(-271828.0, -271.0, NumfmtRoundMethod::FromZero), 1004.0);
/// ```
pub fn numfmt_div_round(n: f64, d: f64, round_method: NumfmtRoundMethod) -> f64 {
    let v = n / d;

    if v.abs() < 10.0 {
        round_method.round(10.0 * v) / 10.0
    } else {
        round_method.round(v)
    }
}

// 四舍五入到指定的小数点位数。
fn numfmt_round_with_precision(n: f64, round_method: NumfmtRoundMethod, precision: usize) -> f64 {
    let p = 10.0_f64.powf(precision as f64);

    round_method.round(p * n) / p
}

fn numfmt_decimal_exponent(mut value: f64) -> usize {
    let mut exponent = 0usize;
    while value.abs() >= 10.0 {
        value /= 10.0;
        exponent += 1;
    }
    exponent
}

fn numfmt_format_diagnostic_value(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }

    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap();
    let exponent = exponent.parse::<i32>().unwrap();

    if (-4..6).contains(&exponent) {
        let decimal_places = (5 - exponent).max(0) as usize;
        let fixed = format!("{value:.decimal_places$}");
        return if fixed.contains('.') {
            fixed
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        } else {
            fixed
        };
    }

    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    format!("{mantissa}e{exponent:+03}")
}

fn numfmt_validate_unscaled_output(value: f64, unit: &NumfmtUnit, precision: usize) -> Result<()> {
    if *unit != NumfmtUnit::None || !value.is_finite() {
        return Ok(());
    }

    if numfmt_decimal_exponent(value).saturating_add(precision) <= NUMFMT_MAX_UNSCALED_DIGITS {
        return Ok(());
    }

    let value = numfmt_format_diagnostic_value(value);
    if precision == 0 {
        Err(format!(
            "value too large to be printed: '{value}' (consider using --to)"
        ))
    } else {
        Err(format!(
            "value/precision too large to be printed: '{value}/{precision}' (consider using --to)"
        ))
    }
}

fn numfmt_consider_suffix(
    n: f64,
    u: &NumfmtUnit,
    round_method: NumfmtRoundMethod,
    precision: usize,
) -> Result<(f64, Option<NumfmtSuffix>)> {
    use crate::units::NumfmtRawSuffix::*;

    if !n.is_finite() {
        return Err("Number is too big and unsupported".to_string());
    }

    let n_abs = n.abs();
    let raw_suffixes = [K, M, G, T, P, E, Z, Y, R, Q];

    let (bases, is_with_i) = match *u {
        NumfmtUnit::Si => (&NUMFMT_SI_BASES, false),
        NumfmtUnit::Iec(with_i) => (&NUMFMT_IEC_BASES, with_i),
        NumfmtUnit::Auto => return Err("Unit 'auto' isn't supported with --to options".to_owned()),
        NumfmtUnit::None => return Ok((n, None)),
    };

    let mut power = 0usize;
    while power + 1 < bases.len() && n_abs >= bases[power + 1] {
        power += 1;
    }

    if power > raw_suffixes.len() {
        return Err("Number is too big and unsupported".to_string());
    }

    crate::numfmt_dev_trace(format_args!(
        "  scaled value to {:.6} * {:.0} ^ {power}\n",
        n / bases[power],
        bases[1],
    ));

    let mut v = if precision > 0 {
        numfmt_round_with_precision(n / bases[power], round_method, precision)
    } else {
        numfmt_div_round(n, bases[power], round_method)
    };

    // GNU chooses the initial power before rounding, then promotes only if
    // rounding crosses the next unit boundary.
    if v.abs() >= bases[1] {
        if power >= raw_suffixes.len() {
            return Err("Number is too big and unsupported".to_string());
        }
        v /= bases[1];
        power += 1;
    }

    crate::numfmt_dev_trace(format_args!(
        "  after rounding, value={v:.6} * {:.0} ^ {power}\n",
        bases[1],
    ));

    let suffix = power
        .checked_sub(1)
        .map(|index| (raw_suffixes[index], is_with_i));
    Ok((v, suffix))
}

fn numfmt_transform_to(
    s: f64,
    opts: &NumfmtTransformOptions,
    round_method: NumfmtRoundMethod,
    precision: usize,
) -> Result<String> {
    let value = s / (opts.to_unit as f64);
    numfmt_validate_unscaled_output(value, &opts.to, precision)?;
    crate::numfmt_dev_trace(format_args!("double_to_human:\n"));
    let (i2, suffix) = numfmt_consider_suffix(value, &opts.to, round_method, precision)?;
    let output = match suffix {
        None if opts.to != NumfmtUnit::None => {
            format!("{i2:.precision$}")
        }
        None => {
            format!(
                "{:.precision$}",
                numfmt_round_with_precision(i2, round_method, precision),
                precision = precision
            )
        }
        Some(numfmt_suffix) if precision > 0 => {
            format!(
                "{:.precision$}{}",
                i2,
                DisplayableSuffix(numfmt_suffix),
                precision = precision
            )
        }
        Some(numfmt_suffix) if i2.abs() < 10.0 => {
            format!("{:.1}{}", i2, DisplayableSuffix(numfmt_suffix))
        }
        Some(numfmt_suffix) => format!("{:.0}{}", i2, DisplayableSuffix(numfmt_suffix)),
    };

    if opts.to == NumfmtUnit::None {
        crate::numfmt_dev_trace(format_args!("  no scaling, returning value: {output}\n"));
    } else {
        crate::numfmt_dev_trace(format_args!(
            "  returning value: {}\n",
            numfmt_quote(&output),
        ));
    }

    Ok(output)
}

fn numfmt_normalize_exact_integer(number: &str) -> Option<String> {
    let normalized = numfmt_normalize_decimal_point(number);
    let number = normalized.as_ref();

    if number.contains('.') {
        return None;
    }

    let (is_negative, digits) = if let Some(rest) = number.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = number.strip_prefix('+') {
        (false, rest)
    } else {
        (false, number)
    };

    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some("0".to_string());
    }

    if digits.len() > 19 {
        return None;
    }

    let mut normalized = String::with_capacity(digits.len() + if is_negative { 1 } else { 0 });
    if is_negative {
        normalized.push('-');
    }
    normalized.push_str(digits);
    Some(normalized)
}

fn numfmt_format_unscaled_integer_exact(
    source: &str,
    options: &NumfmtConfigs,
    precision: usize,
) -> Result<Option<String>> {
    if precision != 0
        || options.transform.from != NumfmtUnit::None
        || options.transform.to != NumfmtUnit::None
        || options.transform.from_unit != 1
        || options.transform.to_unit != 1
    {
        return Ok(None);
    }

    let parsed = numfmt_parse_number_parts(source)?;
    if parsed.suffix.is_some() {
        return Ok(None);
    }

    Ok(numfmt_normalize_exact_integer(parsed.number))
}

fn numfmt_decimal_rounding_mode(method: NumfmtRoundMethod) -> RoundingMode {
    match method {
        NumfmtRoundMethod::Up => RoundingMode::Ceiling,
        NumfmtRoundMethod::Down => RoundingMode::Floor,
        NumfmtRoundMethod::FromZero => RoundingMode::Up,
        NumfmtRoundMethod::TowardsZero => RoundingMode::Down,
        NumfmtRoundMethod::Nearest => RoundingMode::HalfUp,
    }
}

fn numfmt_format_unscaled_decimal_exact(
    source: &str,
    options: &NumfmtConfigs,
    precision: usize,
) -> Result<Option<String>> {
    if options.transform.to != NumfmtUnit::None
        || options.transform.from_unit != 1
        || options.transform.to_unit != 1
    {
        return Ok(None);
    }

    let parsed = numfmt_parse_number_parts(source)?;
    let normalized = numfmt_normalize_decimal_point(parsed.number);
    if parsed.suffix.is_some() || !normalized.contains('.') {
        return Ok(None);
    }

    let precision = i64::try_from(precision)
        .map_err(|_| "value/precision too large to be printed".to_string())?;
    let number = normalized
        .parse::<BigDecimal>()
        .map_err(|_| format!("invalid number: {}", numfmt_quote(source)))?;
    let approximate = normalized
        .parse::<f64>()
        .map_err(|_| format!("invalid number: {}", numfmt_quote(source)))?;
    numfmt_validate_unscaled_output(approximate, &options.transform.to, precision as usize)?;

    Ok(Some(
        number
            .with_scale_round(precision, numfmt_decimal_rounding_mode(options.round))
            .to_plain_string(),
    ))
}

fn numfmt_finish_format_string(
    number: String,
    numfmt_configs: &NumfmtConfigs,
    implicit_padding: Option<isize>,
    trace_value: Option<f64>,
) -> String {
    let number = if numfmt_configs.format.is_grouping {
        numfmt_group_number_thousands(&number)
    } else {
        numfmt_localize_number(&number)
    };

    let number_with_suffix = match &numfmt_configs.suffix {
        Some(suffix) => format!("{number}{suffix}"),
        None => number,
    };

    if let Some(value) = trace_value {
        crate::numfmt_dev_trace(format_args!(
            "formatting output:\n  value: {value:.6}\n  humanized: {}\n",
            numfmt_quote(&number_with_suffix),
        ));
    }

    let padded_number =
        get_padded_number(numfmt_configs, implicit_padding, number_with_suffix.clone());
    if trace_value.is_some() && padded_number != number_with_suffix {
        crate::numfmt_dev_trace(format_args!(
            "  After padding: {}\n",
            numfmt_quote(&padded_number),
        ));
    }

    format!(
        "{}{}{}",
        numfmt_configs.format.prefix, padded_number, numfmt_configs.format.suffix
    )
}

fn numfmt_localize_number_with(number: &str, decimal_point: &str) -> String {
    if decimal_point == "." || decimal_point.is_empty() {
        return number.to_string();
    }

    let mantissa_end = number.find(['e', 'E', 'p', 'P']).unwrap_or(number.len());
    let Some(decimal_index) = number[..mantissa_end].find('.') else {
        return number.to_string();
    };

    let mut localized = String::with_capacity(number.len() - 1 + decimal_point.len());
    localized.push_str(&number[..decimal_index]);
    localized.push_str(decimal_point);
    localized.push_str(&number[decimal_index + 1..]);
    localized
}

fn numfmt_localize_number(number: &str) -> String {
    numfmt_localize_number_with(number, &numfmt_locale_decimal_point())
}

fn numfmt_group_number_thousands(number: &str) -> String {
    let decimal_point = numfmt_locale_decimal_point();
    let Some((sep, grouping)) = numfmt_locale_thousands_grouping() else {
        return numfmt_localize_number_with(number, &decimal_point);
    };

    numfmt_group_number_thousands_with_decimal_point(number, &sep, &grouping, &decimal_point)
}

#[cfg(test)]
fn numfmt_group_number_thousands_with(number: &str, sep: &str, grouping: &[u8]) -> String {
    numfmt_group_number_thousands_with_decimal_point(number, sep, grouping, ".")
}

fn numfmt_group_number_thousands_with_decimal_point(
    number: &str,
    sep: &str,
    grouping: &[u8],
    decimal_point: &str,
) -> String {
    if sep.is_empty() || grouping.is_empty() {
        return numfmt_localize_number_with(number, decimal_point);
    }

    let (num_with_no_exp, exp) = match number.find(['e', 'E', 'p', 'P']) {
        Some(idx) => (&number[..idx], &number[idx..]),
        None => (number, ""),
    };

    let (sign, rest) = match num_with_no_exp.as_bytes().first().copied() {
        Some(b'+') | Some(b'-') | Some(b' ') => (&num_with_no_exp[..1], &num_with_no_exp[1..]),
        _ => ("", num_with_no_exp),
    };

    if rest.starts_with("0x") || rest.starts_with("0X") || rest.is_empty() {
        return numfmt_localize_number_with(number, decimal_point);
    }

    let (int_part, frac_part) = match rest.find('.') {
        Some(dot_idx) => (&rest[..dot_idx], Some(&rest[dot_idx + 1..])),
        None => (rest, None),
    };

    if int_part.len() <= 3 || !int_part.bytes().all(|c| c.is_ascii_digit()) {
        return numfmt_localize_number_with(number, decimal_point);
    }

    let grouped = numfmt_apply_locale_grouping(int_part, sep, grouping);
    if grouped == int_part {
        return numfmt_localize_number_with(number, decimal_point);
    }

    let fraction_capacity = frac_part.map_or(0, |fraction| decimal_point.len() + fraction.len());
    let mut output =
        String::with_capacity(sign.len() + grouped.len() + fraction_capacity + exp.len());
    output.push_str(sign);
    output.push_str(&grouped);
    if let Some(fraction) = frac_part {
        output.push_str(decimal_point);
        output.push_str(fraction);
    }
    output.push_str(exp);
    output
}

fn numfmt_locale_thousands_grouping() -> Option<(String, Vec<u8>)> {
    unsafe {
        let lc = ctcore::libc::localeconv();
        if lc.is_null() {
            return None;
        }

        let sep_ptr = (*lc).thousands_sep;
        let grouping_ptr = (*lc).grouping;
        if sep_ptr.is_null() || grouping_ptr.is_null() {
            return None;
        }

        let sep = CStr::from_ptr(sep_ptr).to_bytes();
        if sep.is_empty() {
            return None;
        }

        let mut grouping = Vec::new();
        for idx in 0..16 {
            let g = *grouping_ptr.add(idx) as u8;
            if g == 0 {
                if grouping.is_empty() {
                    return None;
                }
                grouping.push(0);
                break;
            }

            if g == u8::MAX || g == 127 {
                break;
            }

            grouping.push(g);
        }

        if grouping.is_empty() {
            return None;
        }

        Some((String::from_utf8_lossy(sep).into_owned(), grouping))
    }
}

fn numfmt_apply_locale_grouping(int_part: &str, sep: &str, grouping: &[u8]) -> String {
    let mut groups = Vec::new();
    let mut end = int_part.len();
    let mut grouping_idx = 0usize;
    let mut last_size = 0usize;
    let mut repeat_last = false;

    while end > 0 {
        let size = if repeat_last {
            last_size
        } else if let Some(&g) = grouping.get(grouping_idx) {
            if g == 0 {
                repeat_last = true;
                last_size
            } else {
                let new_size = g as usize;
                last_size = new_size;
                if grouping_idx + 1 < grouping.len() {
                    grouping_idx += 1;
                }
                new_size
            }
        } else {
            last_size
        };

        if size == 0 {
            break;
        }

        let start = end.saturating_sub(size);
        groups.push(&int_part[start..end]);
        end = start;
    }

    if groups.is_empty() || end > 0 {
        groups.push(&int_part[..end]);
    }

    groups.reverse();
    groups.join(sep)
}

fn numfmt_trace_exact_value(
    source: &str,
    options: &NumfmtConfigs,
    precision: usize,
) -> Result<Option<f64>> {
    if !crate::numfmt_dev_debug_enabled() {
        return Ok(None);
    }

    let transformed = numfmt_transform_from(source, &options.transform)?;
    let trace_value = transformed / (options.transform.to_unit as f64);
    let _ = numfmt_transform_to(transformed, &options.transform, options.round, precision)?;
    Ok(Some(trace_value))
}

fn numfmt_format_string(
    source: &str,
    numfmt_configs: &NumfmtConfigs,
    implicit_padding: Option<isize>,
) -> Result<String> {
    // 在应用任何转换之前，先去除（optional）后缀
    let source_without_suffix = match &numfmt_configs.suffix {
        Some(suffix) => source.strip_suffix(suffix).unwrap_or(source),
        None => source,
    };

    let precision = if let Some(p) = numfmt_configs.format.precision {
        p
    } else if numfmt_configs.transform.to == NumfmtUnit::None {
        numfmt_parse_input_precision(source_without_suffix)?
    } else {
        0
    };

    if let Some(number) =
        numfmt_format_unscaled_integer_exact(source_without_suffix, numfmt_configs, precision)?
    {
        let trace_value =
            numfmt_trace_exact_value(source_without_suffix, numfmt_configs, precision)?;
        return Ok(numfmt_finish_format_string(
            number,
            numfmt_configs,
            implicit_padding,
            trace_value,
        ));
    }

    if let Some(number) =
        numfmt_format_unscaled_decimal_exact(source_without_suffix, numfmt_configs, precision)?
    {
        let trace_value =
            numfmt_trace_exact_value(source_without_suffix, numfmt_configs, precision)?;
        return Ok(numfmt_finish_format_string(
            number,
            numfmt_configs,
            implicit_padding,
            trace_value,
        ));
    }

    let transformed = numfmt_transform_from(source_without_suffix, &numfmt_configs.transform)?;
    let trace_value = transformed / (numfmt_configs.transform.to_unit as f64);
    let number = numfmt_transform_to(
        transformed,
        &numfmt_configs.transform,
        numfmt_configs.round,
        precision,
    )?;

    Ok(numfmt_finish_format_string(
        number,
        numfmt_configs,
        implicit_padding,
        Some(trace_value),
    ))
}

fn numfmt_delimited_trailing_blank_error(
    source: &str,
    numfmt_configs: &NumfmtConfigs,
) -> Option<String> {
    let source_without_suffix = match &numfmt_configs.suffix {
        Some(suffix) => source.strip_suffix(suffix).unwrap_or(source),
        None => source,
    };
    let without_trailing_blanks = source_without_suffix.trim_end_matches([' ', '\t']);

    if without_trailing_blanks.len() == source_without_suffix.len()
        || numfmt_parse_number_parts(source_without_suffix).is_err()
    {
        return None;
    }

    if numfmt_configs.transform.from == NumfmtUnit::None {
        Some(format!(
            "rejecting suffix in input: {} (consider using --from)",
            numfmt_quote(source_without_suffix)
        ))
    } else {
        Some(format!(
            "invalid suffix in input {}: {}",
            numfmt_quote(source_without_suffix),
            numfmt_quote(&source_without_suffix[without_trailing_blanks.len()..])
        ))
    }
}

fn numfmt_trace_custom_suffix(source: &str, options: &NumfmtConfigs) {
    let Some(suffix) = &options.suffix else {
        return;
    };
    if source.len() <= suffix.len() {
        return;
    }

    if source.ends_with(suffix) {
        crate::numfmt_dev_trace(format_args!("trimming suffix {}\n", numfmt_quote(suffix)));
    } else {
        crate::numfmt_dev_trace(format_args!("no valid suffix found\n"));
    }
}

fn numfmt_auto_padding_enabled(options: &NumfmtConfigs) -> bool {
    if options.delimiter.is_some() || options.padding != 0 {
        return false;
    }

    !matches!(
        options.format.padding,
        Some(value)
            if value != 0 && !(options.format.is_zero_padding && value.is_positive())
    )
}

fn numfmt_trace_auto_padding(options: &NumfmtConfigs, implicit_padding: Option<isize>) {
    if numfmt_auto_padding_enabled(options) {
        crate::numfmt_dev_trace(format_args!(
            "setting Auto-Padding to {} characters\n",
            implicit_padding.unwrap_or(0).unsigned_abs(),
        ));
    }
}

fn numfmt_format_delimited_field(field: &str, numfmt_configs: &NumfmtConfigs) -> Result<String> {
    let source = field.trim_start();

    numfmt_trace_custom_suffix(source, numfmt_configs);

    // With an explicit delimiter, GNU keeps trailing blanks inside the field
    // and lets end-pointer validation reject them.
    if let Some(error) = numfmt_delimited_trailing_blank_error(source, numfmt_configs) {
        return Err(error);
    }

    numfmt_format_string(source, numfmt_configs, None)
}

fn pad_string(value: String, width: isize, fill: char) -> String {
    if width == 0 {
        return value;
    }

    let target_width = width.unsigned_abs();
    let value_width = value.len();
    if target_width <= value_width {
        return value;
    }

    let pad_count = target_width - value_width;
    let padding: String = std::iter::repeat_n(fill, pad_count).collect();

    if width > 0 {
        let mut out = String::with_capacity(target_width);
        out.push_str(&padding);
        out.push_str(&value);
        out
    } else {
        let mut out = String::with_capacity(target_width);
        out.push_str(&value);
        out.push_str(&padding);
        out
    }
}

fn get_padded_number(
    numfmt_configs: &NumfmtConfigs,
    implicit_padding: Option<isize>,
    number_with_suffix: String,
) -> String {
    let padding = numfmt_configs
        .format
        .padding
        .unwrap_or_else(|| implicit_padding.unwrap_or(numfmt_configs.padding));

    match padding {
        0 => number_with_suffix,
        p_isize if p_isize > 0 && numfmt_configs.format.is_zero_padding => {
            let zero_padded = pad_string(number_with_suffix, p_isize, '0');

            match implicit_padding.unwrap_or(numfmt_configs.padding) {
                0 => zero_padded,
                p => pad_string(zero_padded, p, ' '),
            }
        }
        p_isize => pad_string(number_with_suffix, p_isize, ' '),
    }
}

fn numfmt_format_line_delimited(s: &str, options: &NumfmtConfigs) -> Result<String> {
    let delimiter = options.delimiter.as_ref().unwrap();

    let mut out = String::new();

    if delimiter.is_empty() {
        let field = numfmt_nul_delimited_first_field(s);
        if ctcore::ct_ranges::contain(&options.fields, 1) {
            out.push_str(&numfmt_format_delimited_field(field, options)?);
        } else {
            out.push_str(field);
        }
    } else {
        for (n, field) in (1..).zip(s.split(delimiter)) {
            let is_field_selected = ctcore::ct_ranges::contain(&options.fields, n);

            // 在第二个及其后的字段前打印分隔符
            if n > 1 {
                out.push_str(delimiter);
            }

            if is_field_selected {
                out.push_str(&numfmt_format_delimited_field(field, options)?);
            } else {
                // 打印未选择的字段，不进行转换
                out.push_str(field);
            }
        }
    }

    if options.zero_terminated {
        out.push('\0');
    } else {
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
fn numfmt_format_and_print_delimited(s: &str, options: &NumfmtConfigs) -> Result<()> {
    let _ = numfmt_format_line_delimited(s, options)?;
    Ok(())
}

fn numfmt_format_line_whitespace(s: &str, numfmt_configs: &NumfmtConfigs) -> Result<String> {
    let mut out = String::new();

    for (n, (prefix, field)) in (1..).zip(NumfmtWhitespaceSplitter { s: Some(s) }) {
        let is_field_selected = ctcore::ct_ranges::contain(&numfmt_configs.fields, n);

        if is_field_selected {
            let is_empty_prefix = prefix.is_empty();

            // 在第二个及其后的字段前打印分隔符
            let prefix_str = if n > 1 {
                out.push(' ');
                &prefix[1..]
            } else {
                prefix
            };

            let implicit_padding = if !is_empty_prefix && numfmt_configs.padding == 0 {
                Some((prefix_str.len() + field.len()) as isize)
            } else {
                None
            };

            numfmt_trace_custom_suffix(field, numfmt_configs);
            numfmt_trace_auto_padding(numfmt_configs, implicit_padding);

            out.push_str(&numfmt_format_string(
                field,
                numfmt_configs,
                implicit_padding,
            )?);
        } else {
            // 打印未选择的字段，不进行转换
            numfmt_push_unselected_whitespace_field(&mut out, n, prefix, field);
        }
    }

    if numfmt_configs.zero_terminated {
        out.push('\0');
    } else {
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
fn numfmt_format_and_print_whitespace(s: &str, numfmt_configs: &NumfmtConfigs) -> Result<()> {
    let _ = numfmt_format_line_whitespace(s, numfmt_configs)?;
    Ok(())
}

fn numfmt_format_line_delimited_abort(
    s: &str,
    options: &NumfmtConfigs,
) -> std::result::Result<String, NumfmtAbortRenderError> {
    let delimiter = options.delimiter.as_ref().unwrap();
    let mut out = String::new();

    if delimiter.is_empty() {
        let field = numfmt_nul_delimited_first_field(s);
        if ctcore::ct_ranges::contain(&options.fields, 1) {
            match numfmt_format_delimited_field(field, options) {
                Ok(rendered) => out.push_str(&rendered),
                Err(error) => {
                    return Err(NumfmtAbortRenderError {
                        partial_output: out,
                        error,
                    });
                }
            }
        } else {
            out.push_str(field);
        }
    } else {
        for (n, field) in (1..).zip(s.split(delimiter)) {
            let is_field_selected = ctcore::ct_ranges::contain(&options.fields, n);

            if n > 1 {
                out.push_str(delimiter);
            }

            if is_field_selected {
                match numfmt_format_delimited_field(field, options) {
                    Ok(rendered) => out.push_str(&rendered),
                    Err(error) => {
                        return Err(NumfmtAbortRenderError {
                            partial_output: out,
                            error,
                        });
                    }
                }
            } else {
                out.push_str(field);
            }
        }
    }

    if options.zero_terminated {
        out.push('\0');
    } else {
        out.push('\n');
    }

    Ok(out)
}

fn numfmt_format_line_whitespace_abort(
    s: &str,
    numfmt_configs: &NumfmtConfigs,
) -> std::result::Result<String, NumfmtAbortRenderError> {
    let mut out = String::new();

    for (n, (prefix, field)) in (1..).zip(NumfmtWhitespaceSplitter { s: Some(s) }) {
        let is_field_selected = ctcore::ct_ranges::contain(&numfmt_configs.fields, n);

        if is_field_selected {
            let is_empty_prefix = prefix.is_empty();

            let prefix_str = if n > 1 {
                out.push(' ');
                &prefix[1..]
            } else {
                prefix
            };

            let implicit_padding = if !is_empty_prefix && numfmt_configs.padding == 0 {
                Some((prefix_str.len() + field.len()) as isize)
            } else {
                None
            };

            numfmt_trace_custom_suffix(field, numfmt_configs);
            numfmt_trace_auto_padding(numfmt_configs, implicit_padding);

            match numfmt_format_string(field, numfmt_configs, implicit_padding) {
                Ok(rendered) => out.push_str(&rendered),
                Err(error) => {
                    return Err(NumfmtAbortRenderError {
                        partial_output: out,
                        error,
                    });
                }
            }
        } else {
            numfmt_push_unselected_whitespace_field(&mut out, n, prefix, field);
        }
    }

    if numfmt_configs.zero_terminated {
        out.push('\0');
    } else {
        out.push('\n');
    }

    Ok(out)
}

pub fn numfmt_format_line(s: &str, numfmt_configs: &NumfmtConfigs) -> Result<String> {
    if numfmt_configs.delimiter.is_some() {
        numfmt_format_line_delimited(s, numfmt_configs)
    } else {
        numfmt_format_line_whitespace(s, numfmt_configs)
    }
}

fn numfmt_preserve_line_ending(
    mut rendered: String,
    zero_terminated: bool,
    has_line_ending: bool,
) -> String {
    if !has_line_ending {
        let line_ending = if zero_terminated { '\0' } else { '\n' };
        if rendered.ends_with(line_ending) {
            rendered.pop();
        }
    }
    rendered
}

pub(crate) fn numfmt_format_line_with_line_ending(
    s: &str,
    numfmt_configs: &NumfmtConfigs,
    has_line_ending: bool,
) -> Result<String> {
    numfmt_format_line(s, numfmt_configs).map(|rendered| {
        numfmt_preserve_line_ending(rendered, numfmt_configs.zero_terminated, has_line_ending)
    })
}

pub fn numfmt_format_line_abort(
    s: &str,
    numfmt_configs: &NumfmtConfigs,
) -> std::result::Result<String, NumfmtAbortRenderError> {
    if numfmt_configs.delimiter.is_some() {
        numfmt_format_line_delimited_abort(s, numfmt_configs)
    } else {
        numfmt_format_line_whitespace_abort(s, numfmt_configs)
    }
}

pub(crate) fn numfmt_format_line_abort_with_line_ending(
    s: &str,
    numfmt_configs: &NumfmtConfigs,
    has_line_ending: bool,
) -> std::result::Result<String, NumfmtAbortRenderError> {
    numfmt_format_line_abort(s, numfmt_configs).map(|rendered| {
        numfmt_preserve_line_ending(rendered, numfmt_configs.zero_terminated, has_line_ending)
    })
}

/// 根据所选选项格式化一行文本。
///
/// 给定一行文本 "s"，将该行文本拆分成若干字段，对选定的数字字段进行转换和format，
/// 并将结果打印到stdout。未被选中转换的字段将原封不动地通过。
pub fn numfmt_format_and_print(s: &str, numfmt_configs: &NumfmtConfigs) -> Result<()> {
    numfmt_format_and_print_with_line_ending(s, numfmt_configs, true)?;
    Ok(())
}

pub(crate) fn numfmt_format_and_print_with_line_ending(
    s: &str,
    numfmt_configs: &NumfmtConfigs,
    has_line_ending: bool,
) -> Result<()> {
    print!(
        "{}",
        numfmt_format_line_with_line_ending(s, numfmt_configs, has_line_ending)?
    );
    Ok(())
}

pub fn numfmt_format_and_print_abort(s: &str, numfmt_configs: &NumfmtConfigs) -> Result<()> {
    numfmt_format_and_print_abort_with_line_ending(s, numfmt_configs, true)
}

pub(crate) fn numfmt_format_and_print_abort_with_line_ending(
    s: &str,
    numfmt_configs: &NumfmtConfigs,
    has_line_ending: bool,
) -> Result<()> {
    match numfmt_format_line_abort_with_line_ending(s, numfmt_configs, has_line_ending) {
        Ok(rendered) => {
            print!("{rendered}");
            Ok(())
        }
        Err(error) => {
            print!("{}", error.partial_output);
            Err(error.error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::NumfmtInvalidModes;

    fn setup_default_config() -> NumfmtConfigs {
        NumfmtConfigs {
            transform: NumfmtTransformOptions {
                from: NumfmtUnit::Auto,
                from_unit: 1,
                to: NumfmtUnit::Si,
                to_unit: 1,
            },
            padding: 0,
            header: 0,
            fields: vec![],
            delimiter: None,
            round: NumfmtRoundMethod::Up,
            suffix: None,
            format: Default::default(),
            invalid: NumfmtInvalidModes::Abort,
            zero_terminated: false,
        }
    }

    #[test]
    fn test_numfmt_group_number_thousands_with_standard_grouping() {
        assert_eq!(
            numfmt_group_number_thousands_with("123456789", ",", &[3, 0]),
            "123,456,789"
        );
        assert_eq!(
            numfmt_group_number_thousands_with("-1234567.89", ",", &[3, 0]),
            "-1,234,567.89"
        );
        assert_eq!(
            numfmt_group_number_thousands_with("1234567e+08", ",", &[3, 0]),
            "1,234,567e+08"
        );
    }

    #[test]
    fn test_numfmt_group_number_thousands_with_non_repeating_grouping() {
        assert_eq!(
            numfmt_group_number_thousands_with("123456789", ",", &[3, 2]),
            "12,34,56,789"
        );
    }

    #[test]
    fn test_locale_decimal_point_helpers() {
        assert_eq!(
            parse_numeric_prefix_with_decimal_point("1234,5K", ","),
            Some((6, false))
        );
        assert_eq!(
            parse_numeric_prefix_with_decimal_point("1234.5", ","),
            Some((4, false))
        );
        assert_eq!(
            parse_numeric_prefix_with_decimal_point("1234,", ","),
            Some((5, true))
        );
        assert_eq!(parse_numeric_prefix_with_decimal_point("+1", ","), None);

        let multibyte = "12٫5";
        assert_eq!(
            parse_numeric_prefix_with_decimal_point(multibyte, "٫"),
            Some((multibyte.len(), false))
        );
        assert_eq!(
            numfmt_normalize_decimal_point_with("1234,50", ","),
            "1234.50"
        );
        assert_eq!(numfmt_localize_number_with("1.25K", ","), "1,25K");
        assert_eq!(numfmt_localize_number_with("1.25e+03", ","), "1,25e+03");
        assert_eq!(
            numfmt_group_number_thousands_with_decimal_point("1234.50", ".", &[3, 0], ","),
            "1.234,50"
        );
    }

    #[test]
    fn test_format_line_preserves_input_line_ending() {
        let mut config = setup_default_config();

        assert_eq!(
            numfmt_format_line_with_line_ending("1", &config, true).unwrap(),
            "1\n"
        );
        assert_eq!(
            numfmt_format_line_with_line_ending("1", &config, false).unwrap(),
            "1"
        );
        assert_eq!(
            numfmt_format_line_abort_with_line_ending("1", &config, false).unwrap(),
            "1"
        );

        config.zero_terminated = true;
        assert_eq!(
            numfmt_format_line_with_line_ending("1", &config, true).unwrap(),
            "1\0"
        );
        assert_eq!(
            numfmt_format_line_with_line_ending("1", &config, false).unwrap(),
            "1"
        );
        assert_eq!(numfmt_format_line("1", &config).unwrap(), "1\0");
    }

    #[test]
    fn test_empty_delimiter_stops_at_nul() {
        let mut config = setup_default_config();
        config.delimiter = Some(String::new());
        config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }];

        assert_eq!(numfmt_format_line("12\u{0}34", &config).unwrap(), "12\n");
        assert_eq!(
            numfmt_format_line_abort("12\u{0}34", &config).unwrap(),
            "12\n"
        );
        assert_eq!(numfmt_passthrough_input("bad\u{0}34", &config), "bad");

        config.fields = vec![ctcore::ct_ranges::CtRange { low: 2, high: 2 }];
        assert_eq!(numfmt_format_line("12\u{0}34", &config).unwrap(), "12\n");
    }

    #[test]
    fn test_delimited_field_rejects_trailing_blank() {
        let mut config = setup_default_config();
        config.transform.from = NumfmtUnit::None;
        config.transform.to = NumfmtUnit::None;
        config.delimiter = Some(",".to_string());
        config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }];

        let expected = "rejecting suffix in input: '123 ' (consider using --from)";
        assert_eq!(numfmt_format_line("123 ,x", &config).unwrap_err(), expected);

        let error = numfmt_format_line_abort("123 ,x", &config).unwrap_err();
        assert_eq!(error.partial_output, "");
        assert_eq!(error.error, expected);
        assert!(numfmt_precision_loss_inputs("1234567890123456789 ,x", &config).is_empty());
    }

    #[test]
    fn test_default_delimiter_normalizes_first_separator_byte() {
        let mut config = setup_default_config();
        config.transform.from = NumfmtUnit::None;
        config.transform.to = NumfmtUnit::None;
        config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }];

        assert_eq!(numfmt_format_line("1\t2", &config).unwrap(), "1 2\n");
        assert_eq!(numfmt_format_line_abort("1\t2", &config).unwrap(), "1 2\n");
        assert_eq!(numfmt_format_line("1\t\t2", &config).unwrap(), "1 \t2\n");
        assert_eq!(numfmt_passthrough_input("x\t2", &config), "x 2");

        config.zero_terminated = true;
        assert_eq!(numfmt_format_line("1\t2", &config).unwrap(), "1 2\0");
    }

    #[test]
    fn test_padding_counts_multibyte_suffix_bytes() {
        let mut config = setup_default_config();
        config.transform.from = NumfmtUnit::None;
        config.transform.to = NumfmtUnit::None;
        config.padding = 5;
        config.suffix = Some("é".to_string());

        assert_eq!(numfmt_format_string("1", &config, None).unwrap(), "  1é");

        config.padding = -5;
        assert_eq!(numfmt_format_string("1", &config, None).unwrap(), "1é  ");

        config.padding = 5;
        config.suffix = Some("界".to_string());
        assert_eq!(numfmt_format_string("1", &config, None).unwrap(), " 1界");
    }

    #[cfg(test)]
    mod numfmt_parse_suffix_tests {
        use crate::format::numfmt_parse_suffix;
        use crate::units::NumfmtRawSuffix;

        #[test]
        fn test_parse_with_suffix() {
            let examples = [
                ("100K", 100.0, Some((NumfmtRawSuffix::K, false))),
                ("200Mi", 200.0, Some((NumfmtRawSuffix::M, true))),
                ("300G", 300.0, Some((NumfmtRawSuffix::G, false))),
                ("400Ti", 400.0, Some((NumfmtRawSuffix::T, true))),
                ("500P", 500.0, Some((NumfmtRawSuffix::P, false))),
                ("600Ei", 600.0, Some((NumfmtRawSuffix::E, true))),
                ("700Z", 700.0, Some((NumfmtRawSuffix::Z, false))),
                ("800Yi", 800.0, Some((NumfmtRawSuffix::Y, true))),
            ];
            for (input, expected_value, expected_suffix) in examples {
                let result = numfmt_parse_suffix(input).unwrap();
                assert_eq!(result, (expected_value, expected_suffix));
            }
        }

        #[test]
        fn test_parse_without_suffix() {
            let result = numfmt_parse_suffix("1234").unwrap();
            assert_eq!(result, (1234.0, None));
        }

        #[test]
        fn test_parse_rejects_leading_plus() {
            for input in ["+1", "+.5", "+1.2", "+1K"] {
                assert_eq!(
                    numfmt_parse_suffix(input).unwrap_err(),
                    format!("invalid number: '{input}'")
                );
            }

            assert_eq!(numfmt_parse_suffix("-1").unwrap(), (-1.0, None));
            assert_eq!(numfmt_parse_suffix("-.5").unwrap(), (-0.5, None));
        }

        // #[test]
        // fn test_parse_invalid_suffix() {
        //     let inputs = ["100Kx", "200 Mi", "300Gi", "400.25T"];
        //     for input in inputs {
        //         assert!(numfmt_parse_suffix(input).is_err(), "Expected error for input: {}", input);
        //     }
        // }

        #[test]
        fn test_parse_empty_string() {
            assert!(
                numfmt_parse_suffix("").is_err(),
                "Expected error for empty input"
            );
        }

        #[test]
        fn test_parse_extreme_values() {
            let large_number = format!("{}K", f64::MAX);
            let small_number = format!("{}M", f64::MIN);
            assert!(
                numfmt_parse_suffix(&large_number).is_ok(),
                "Should handle large values"
            );
            assert!(
                numfmt_parse_suffix(&small_number).is_ok(),
                "Should handle small values"
            );
        }

        #[test]
        fn test_parse_with_non_standard_characters_in_suffix() {
            assert!(
                numfmt_parse_suffix("100K3").is_err(),
                "Expected error for non-standard characters in suffix"
            );
        }

        #[test]
        fn test_parse_with_multiple_suffixes() {
            assert!(
                numfmt_parse_suffix("100KM").is_err(),
                "Expected error for multiple suffixes"
            );
        }

        #[test]
        fn test_parse_with_spaces() {
            let result = numfmt_parse_suffix("100 K").unwrap();
            assert_eq!(result, (100.0, Some((NumfmtRawSuffix::K, false))));

            let result2 = numfmt_parse_suffix("100 M ").unwrap();
            assert_eq!(result2, (100.0, Some((NumfmtRawSuffix::M, false))));
        }

        #[test]
        fn test_parse_with_additional_characters_between_number_and_suffix() {
            assert!(
                numfmt_parse_suffix("100xK").is_err(),
                "Expected error for additional characters between number and suffix"
            );
        }

        #[test]
        fn test_parse_with_high_precision_decimals() {
            let result = numfmt_parse_suffix("123.456789K").unwrap();
            assert_eq!(
                result,
                (123.456789, Some((NumfmtRawSuffix::K, false))),
                "Should parse high precision decimals correctly"
            );
        }

        #[test]
        fn test_parse_with_non_ascii_characters() {
            assert!(
                numfmt_parse_suffix("100€K").is_err(),
                "Expected error for non-ASCII characters"
            );
            assert!(
                numfmt_parse_suffix("100K€").is_err(),
                "Expected error for non-ASCII characters after suffix"
            );
        }
    }
    #[cfg(test)]
    mod parse_implicit_precision_tests {
        use super::*;
        #[test]
        fn test_no_decimal_point() {
            assert_eq!(numfmt_parse_implicit_precision("100"), 0);
        }

        #[test]
        fn test_decimal_point_no_digits() {
            assert_eq!(numfmt_parse_implicit_precision("100."), 0);
        }

        #[test]
        fn test_normal_decimal_point_with_digits() {
            assert_eq!(numfmt_parse_implicit_precision("100.1234"), 4);
        }

        #[test]
        fn test_decimal_point_with_non_digit_characters() {
            assert_eq!(numfmt_parse_implicit_precision("100.1234abc"), 4);
        }

        #[test]
        fn test_multiple_decimal_points() {
            // 实际上这不是一个合法的浮点数表示，但测试可以用来看看函数怎样处理这种输入
            assert_eq!(numfmt_parse_implicit_precision("100.12.34"), 2);
        }
    }
    #[cfg(test)]
    mod precision_loss_tests {
        use super::numfmt_number_may_lose_precision;

        #[test]
        fn detects_gnu_long_double_precision_boundary() {
            assert!(!numfmt_number_may_lose_precision("999999999999999999"));
            assert!(numfmt_number_may_lose_precision("1000000000000000000"));
            assert!(numfmt_number_may_lose_precision("0001234567890123456789"));
            assert!(!numfmt_number_may_lose_precision("0000000000000000000000"));
            assert!(numfmt_number_may_lose_precision("0.1234567890123456789"));
            assert!(!numfmt_number_may_lose_precision(
                "123456789012345678.123456789012345678"
            ));
            assert!(!numfmt_number_may_lose_precision(
                "1234567890123456789012345678901234"
            ));
        }
    }
    #[cfg(test)]
    mod remove_suffix_tests {
        use super::*;
        // 假设NUMFMT_IEC_BASES数组如下
        const NUMFMT_IEC_BASES: [f64; 9] = [
            1.0,
            1024.0,
            1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        ];

        #[test]
        fn test_si_suffix_handling() {
            let i = 1.0;
            let suffixes = vec![
                (NumfmtRawSuffix::K, 1e3),
                (NumfmtRawSuffix::M, 1e6),
                (NumfmtRawSuffix::G, 1e9),
                (NumfmtRawSuffix::T, 1e12),
            ];
            for (suffix, multiplier) in suffixes {
                let result =
                    numfmt_remove_suffix(i, Some((suffix, false)), &NumfmtUnit::Si).unwrap();
                assert_eq!(result, i * multiplier);
            }
        }

        #[test]
        fn test_iec_suffix_handling() {
            let i = 1.0;
            let unit = NumfmtUnit::Iec(true);
            for (index, suffix) in [
                NumfmtRawSuffix::K,
                NumfmtRawSuffix::M,
                NumfmtRawSuffix::G,
                NumfmtRawSuffix::T,
            ]
            .iter()
            .enumerate()
            {
                let result = numfmt_remove_suffix(i, Some((*suffix, true)), &unit).unwrap();
                assert_eq!(result, i * NUMFMT_IEC_BASES[index + 1]);
            }
        }

        #[test]
        fn test_error_handling_mismatch_suffix_unit() {
            let result =
                numfmt_remove_suffix(1.0, Some((NumfmtRawSuffix::K, true)), &NumfmtUnit::Si);
            assert!(
                result.is_err(),
                "Expected an error due to mismatch suffix and unit"
            );
        }

        #[test]
        fn test_no_suffix_handling() {
            let i = 123.0;
            let result = numfmt_remove_suffix(i, None, &NumfmtUnit::Si).unwrap();
            assert_eq!(
                result, i,
                "Expected the original number when no suffix is provided"
            );
        }

        #[test]
        fn test_unsupported_suffix_error() {
            let result =
                numfmt_remove_suffix(1.0, Some((NumfmtRawSuffix::Z, false)), &NumfmtUnit::None);
            assert!(
                result.is_err(),
                "Expected an error for unsupported suffix with NumfmtUnit::None"
            );
        }
        #[test]
        fn test_large_value_suffix_handling() {
            let i = 1e12; // 极大数值
            let result =
                numfmt_remove_suffix(i, Some((NumfmtRawSuffix::T, false)), &NumfmtUnit::Si)
                    .unwrap();
            assert_eq!(result, i * 1e12);
        }

        #[test]
        fn test_small_value_suffix_handling() {
            let i = 1e-12; // 极小数值
            let result =
                numfmt_remove_suffix(i, Some((NumfmtRawSuffix::K, false)), &NumfmtUnit::Si)
                    .unwrap();
            assert_eq!(result, i * 1e3);
        }

        #[test]
        fn test_special_float_values_handling() {
            // let nan_result = numfmt_remove_suffix(f64::NAN, Some((NumfmtRawSuffix::K, false)), &NumfmtUnit::Si);
            // assert_eq!(nan_result.unwrap(), f64::NAN);

            let inf_result = numfmt_remove_suffix(
                f64::INFINITY,
                Some((NumfmtRawSuffix::M, false)),
                &NumfmtUnit::Si,
            );
            println!("{inf_result:?}");
            assert_eq!(inf_result.unwrap(), f64::INFINITY);
        }

        #[test]
        fn test_complete_mismatch_error_handling() {
            let result = numfmt_remove_suffix(
                1.0,
                Some((NumfmtRawSuffix::P, false)),
                &NumfmtUnit::Iec(true),
            );
            assert!(
                result.is_err(),
                "Expected an error due to complete mismatch of suffix and unit settings"
            );
        }

        #[test]
        fn test_detailed_iec_units() {
            let i = 1.0;
            let unit = NumfmtUnit::Iec(true);
            let suffixes = [
                (NumfmtRawSuffix::K, NUMFMT_IEC_BASES[1]),
                (NumfmtRawSuffix::M, NUMFMT_IEC_BASES[2]),
                (NumfmtRawSuffix::G, NUMFMT_IEC_BASES[3]),
            ];
            for (suffix, base) in suffixes.iter() {
                let result = numfmt_remove_suffix(i, Some((*suffix, true)), &unit).unwrap();
                assert_eq!(result, i * base);
            }
        }

        #[test]
        fn test_iec_i_without_suffix_is_accepted() {
            let result = numfmt_remove_suffix(1.0, None, &NumfmtUnit::Iec(true)).unwrap();
            assert_eq!(result, 1.0);
        }
    }
    #[cfg(test)]
    mod transform_from_tests {
        use super::*;

        #[test]
        fn test_basic_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::None,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("400", &opts).unwrap(), 400.0);
        }

        #[test]
        fn test_with_unit_and_rounding() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("100K", &opts).unwrap(), 100000.0);
        }

        #[test]
        fn test_from_scaling_does_not_round() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };

            assert_eq!(numfmt_transform_from("1.1", &opts).unwrap(), 1.1);
            assert_eq!(numfmt_transform_from("0.0001K", &opts).unwrap(), 0.1);
        }

        #[test]
        fn test_negative_value_handling() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("-300K", &opts).unwrap(), -300000.0);
        }

        #[test]
        fn test_error_handling() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert!(numfmt_transform_from("unsupported", &opts).is_err());
        }

        #[test]
        fn test_large_unit_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("1M", &opts).unwrap(), 1_000_000.0);
        }

        #[test]
        fn test_iec_unit_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Iec(true),
                from_unit: 1024,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(
                numfmt_transform_from("1K", &opts).unwrap_err(),
                "missing 'i' suffix in input: '1024K' (e.g Ki/Mi/Gi)".to_string()
            );
        }

        #[test]
        fn test_very_small_values() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("0.001K", &opts).unwrap(), 1.0);
        }

        #[test]
        fn test_special_float_values() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert!(numfmt_transform_from("NaN", &opts).is_err());
            assert!(numfmt_transform_from("inf", &opts).is_err());
        }

        #[test]
        fn test_invalid_multi_unit_input() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert!(
                numfmt_transform_from("100KM", &opts).is_err(),
                "Expected an error for invalid multi-unit input"
            );
        }
    }

    #[cfg(test)]
    mod numfmt_div_round_tests {
        use super::*;
        #[test]
        fn test_basic_division_and_rounding() {
            assert_eq!(numfmt_div_round(10.0, 3.0, NumfmtRoundMethod::Up), 3.4);
            assert_eq!(numfmt_div_round(10.0, 3.0, NumfmtRoundMethod::Down), 3.3);
            assert_eq!(numfmt_div_round(10.0, 3.0, NumfmtRoundMethod::Nearest), 3.3);
        }

        #[test]
        fn test_decimal_precision() {
            assert_eq!(numfmt_div_round(1.0, 3.0, NumfmtRoundMethod::Up), 0.4);
            assert_eq!(numfmt_div_round(1.0, 3.0, NumfmtRoundMethod::Down), 0.3);
            assert_eq!(numfmt_div_round(1.0, 3.0, NumfmtRoundMethod::Nearest), 0.3);
        }

        #[test]
        fn test_small_values_rounding() {
            assert_eq!(numfmt_div_round(0.05, 2.0, NumfmtRoundMethod::Up), 0.1);
            assert_eq!(numfmt_div_round(0.05, 2.0, NumfmtRoundMethod::Down), 0.0);
        }

        #[test]
        fn test_zero_and_infinity_handling() {
            assert_eq!(numfmt_div_round(0.0, 5.0, NumfmtRoundMethod::Nearest), 0.0);
            assert_eq!(
                numfmt_div_round(1.0, 0.0, NumfmtRoundMethod::Nearest),
                f64::INFINITY
            );
        }

        #[test]
        fn test_negative_values_rounding() {
            assert_eq!(numfmt_div_round(-10.0, 3.0, NumfmtRoundMethod::Up), -3.3);
            assert_eq!(numfmt_div_round(-10.0, 3.0, NumfmtRoundMethod::Down), -3.4);
            assert_eq!(
                numfmt_div_round(-10.0, 3.0, NumfmtRoundMethod::Nearest),
                -3.3
            );
        }
    }

    #[cfg(test)]
    mod consider_suffix_tests {
        use super::*;
        use crate::units::{NumfmtRawSuffix::*, NumfmtWithI};

        fn setup_si_options() -> NumfmtUnit {
            NumfmtUnit::Si
        }

        fn setup_iec_options(with_i: NumfmtWithI) -> NumfmtUnit {
            NumfmtUnit::Iec(with_i)
        }

        #[test]
        fn test_no_conversion_needed() {
            let unit = NumfmtUnit::None;
            let result =
                numfmt_consider_suffix(1234.0, &unit, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(result, (1234.0, None));
        }

        #[test]
        fn test_si_conversion() {
            let unit = setup_si_options();
            let result = numfmt_consider_suffix(1e6, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (1.0, Some((M, false))));
        }

        #[test]
        fn test_iec_conversion() {
            let unit = setup_iec_options(true);
            let result =
                numfmt_consider_suffix(1024.0 * 1024.0, &unit, NumfmtRoundMethod::Nearest, 0)
                    .unwrap();
            assert_eq!(result, (1.0, Some((M, true))));
        }

        #[test]
        fn test_auto_unit_error() {
            let unit = NumfmtUnit::Auto;
            let result = numfmt_consider_suffix(1234.0, &unit, NumfmtRoundMethod::Nearest, 2);
            assert!(result.is_err(), "Expected an error for unit 'auto'");
        }

        #[test]
        fn test_number_too_big() {
            let unit = setup_si_options();
            let result = numfmt_consider_suffix(1e33, &unit, NumfmtRoundMethod::Nearest, 0);
            assert!(result.is_err(), "Expected an error for too large number");
        }

        #[test]
        fn test_different_rounding_methods() {
            let unit = setup_si_options();
            let round_up =
                numfmt_consider_suffix(999_500.0, &unit, NumfmtRoundMethod::Up, 0).unwrap();
            assert_eq!(round_up, (1.0, Some((M, false))));

            let round_down =
                numfmt_consider_suffix(999_499.0, &unit, NumfmtRoundMethod::Down, 0).unwrap();
            assert_eq!(round_down, (999.0, Some((K, false))));
        }

        #[test]
        fn test_precision_control() {
            let unit = setup_si_options();
            let high_precision =
                numfmt_consider_suffix(1234567.0, &unit, NumfmtRoundMethod::Nearest, 3).unwrap();
            assert_eq!(high_precision, (1.235, Some((M, false))));
        }

        #[test]
        fn test_edge_cases_at_unit_boundaries() {
            let unit = setup_si_options();
            let edge_case =
                numfmt_consider_suffix(999_999.999, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(edge_case, (1.0, Some((M, false))));
        }

        #[test]
        fn test_first_suffix_selected_after_rounding() {
            let si = setup_si_options();
            assert_eq!(
                numfmt_consider_suffix(999.1, &si, NumfmtRoundMethod::Down, 0).unwrap(),
                (999.0, None)
            );
            assert_eq!(
                numfmt_consider_suffix(999.1, &si, NumfmtRoundMethod::Nearest, 0).unwrap(),
                (999.0, None)
            );
            assert_eq!(
                numfmt_consider_suffix(999.1, &si, NumfmtRoundMethod::Up, 0).unwrap(),
                (1.0, Some((K, false)))
            );
            assert_eq!(
                numfmt_consider_suffix(-999.1, &si, NumfmtRoundMethod::Up, 0).unwrap(),
                (-999.0, None)
            );

            let iec = setup_iec_options(false);
            assert_eq!(
                numfmt_consider_suffix(1023.1, &iec, NumfmtRoundMethod::Down, 0).unwrap(),
                (1023.0, None)
            );
            assert_eq!(
                numfmt_consider_suffix(1023.9, &iec, NumfmtRoundMethod::Nearest, 0).unwrap(),
                (1.0, Some((K, false)))
            );
        }

        #[test]
        fn test_multi_level_unit_thresholds() {
            let unit = setup_si_options();
            let result = numfmt_consider_suffix(1e9, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (1.0, Some((G, false))));
        }

        #[test]
        fn test_iec_precision_interaction() {
            let unit = setup_iec_options(false);
            let result =
                numfmt_consider_suffix(1023.0 * 1024.0, &unit, NumfmtRoundMethod::Nearest, 3)
                    .unwrap();
            assert_eq!(result, (1023.0, Some((K, false))));
        }

        #[test]
        fn test_negative_values() {
            let unit = setup_si_options();
            let result =
                numfmt_consider_suffix(-1e6, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (-1.0, Some((M, false))));
        }

        #[test]
        fn test_very_small_values_unit_conversion() {
            let unit = setup_si_options();
            let result =
                numfmt_consider_suffix(1e-6, &unit, NumfmtRoundMethod::Nearest, 9).unwrap();
            assert_eq!(result, (1e-6, None)); // Assuming Micro represents μ
        }

        #[test]
        fn test_unit_carry_over() {
            let unit = setup_si_options();
            let result =
                numfmt_consider_suffix(999_950.0, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (1.0, Some((M, false))));
        }

        #[test]
        fn test_special_float_values() {
            let unit = setup_si_options();
            let result_nan = numfmt_consider_suffix(f64::NAN, &unit, NumfmtRoundMethod::Nearest, 2);
            assert!(result_nan.is_err(), "Expected an error for NaN input");

            let result_inf =
                numfmt_consider_suffix(f64::INFINITY, &unit, NumfmtRoundMethod::Nearest, 2);
            assert!(result_inf.is_err(), "Expected an error for Infinity input");
        }

        #[test]
        fn test_values_close_to_unit_thresholds() {
            let unit = setup_si_options();
            let result =
                numfmt_consider_suffix(999_999.5, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (1.0, Some((M, false))));
        }
    }

    #[cfg(test)]
    mod transform_to_tests {
        use super::*;
        fn setup_transform_options() -> NumfmtTransformOptions {
            NumfmtTransformOptions {
                from: NumfmtUnit::None,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            }
        }

        #[test]
        fn test_basic_no_conversion() {
            let opts = setup_transform_options();
            let result =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(result, "1234.57");
        }

        #[test]
        fn test_unit_conversion() {
            let mut opts = setup_transform_options();
            opts.from = NumfmtUnit::Si;
            opts.to = NumfmtUnit::Si;
            opts.to_unit = 1000; // Converting to kilounits
            let result =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(result, "1.23");
        }

        #[test]
        fn test_to_unit_applied_before_suffix_selection() {
            let mut opts = setup_transform_options();
            opts.to = NumfmtUnit::Si;
            opts.to_unit = 3;

            assert_eq!(
                numfmt_transform_to(1000.0, &opts, NumfmtRoundMethod::FromZero, 0).unwrap(),
                "334"
            );
            assert_eq!(
                numfmt_transform_to(3000.0, &opts, NumfmtRoundMethod::FromZero, 0).unwrap(),
                "1.0K"
            );

            opts.to_unit = 1000;
            assert_eq!(
                numfmt_transform_to(999.0, &opts, NumfmtRoundMethod::Down, 0).unwrap(),
                "1"
            );
            assert_eq!(
                numfmt_transform_to(1024.0, &opts, NumfmtRoundMethod::Up, 0).unwrap(),
                "1"
            );
        }

        #[test]
        fn test_rounding_methods() {
            let opts = setup_transform_options();
            let round_up = numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Up, 2).unwrap();
            assert_eq!(round_up, "1234.57");

            let round_down =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Down, 2).unwrap();
            assert_eq!(round_down, "1234.56");

            let round_nearest =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(round_nearest, "1235");
        }

        #[test]
        fn test_precision_control() {
            let opts = setup_transform_options();
            let high_precision =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 4).unwrap();
            assert_eq!(high_precision, "1234.5678");

            let no_precision =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(no_precision, "1235");
        }

        #[test]
        fn test_edge_cases() {
            let opts = setup_transform_options();
            let zero_value =
                numfmt_transform_to(0.0, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(zero_value, "0.00");

            let very_small_value =
                numfmt_transform_to(0.0001234, &opts, NumfmtRoundMethod::Nearest, 6).unwrap();
            assert_eq!(very_small_value, "0.000123");

            let very_large_value =
                numfmt_transform_to(123456789.0, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(very_large_value, "123456789.00");
        }

        #[test]
        fn test_iec_true_to_si_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Iec(true),
                from_unit: 1024,
                to: NumfmtUnit::Si,
                to_unit: 1000,
            };
            let result = numfmt_transform_to(2048.0, &opts, NumfmtRoundMethod::Nearest, 1).unwrap();
            assert_eq!(result, "2.0");
        }

        #[test]
        fn test_iec_false_to_si_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Iec(false),
                from_unit: 1024,
                to: NumfmtUnit::Si,
                to_unit: 1000,
            };
            let result = numfmt_transform_to(2048.0, &opts, NumfmtRoundMethod::Nearest, 1).unwrap();
            assert_eq!(result, "2.0");
        }

        #[test]
        fn test_decimal_and_negative_values() {
            let opts = setup_transform_options();
            let negative_value =
                numfmt_transform_to(-1234.5678, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(negative_value, "-1234.57");

            let decimal_value =
                numfmt_transform_to(0.98765, &opts, NumfmtRoundMethod::Nearest, 4).unwrap();
            assert_eq!(decimal_value, "0.9877");
        }

        #[test]
        fn test_very_large_values() {
            let opts = setup_transform_options();
            let large_value =
                numfmt_transform_to(1e12, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(large_value, "1000000000000");
        }

        #[test]
        fn test_unscaled_output_digit_limit() {
            let mut opts = setup_transform_options();

            assert_eq!(
                numfmt_transform_to(1e18, &opts, NumfmtRoundMethod::Nearest, 0).unwrap(),
                "1000000000000000000"
            );
            assert_eq!(
                numfmt_transform_to(1e19, &opts, NumfmtRoundMethod::Nearest, 0).unwrap_err(),
                "value too large to be printed: '1e+19' (consider using --to)"
            );
            assert_eq!(
                numfmt_transform_to(1e18, &opts, NumfmtRoundMethod::Nearest, 1).unwrap_err(),
                "value/precision too large to be printed: '1e+18/1' (consider using --to)"
            );
            assert_eq!(
                numfmt_transform_to(1e17, &opts, NumfmtRoundMethod::Nearest, 2).unwrap_err(),
                "value/precision too large to be printed: '1e+17/2' (consider using --to)"
            );

            opts.to_unit = 10;
            assert_eq!(
                numfmt_transform_to(1e19, &opts, NumfmtRoundMethod::Nearest, 0).unwrap(),
                "1000000000000000000"
            );
        }

        #[test]
        fn test_non_standard_unit_conversion() {
            let mut opts = setup_transform_options();
            opts.to_unit = 500; // 非标准单位
            let result = numfmt_transform_to(1500.0, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, "3");
        }

        #[test]
        fn test_output_with_suffix() {
            let mut opts = setup_transform_options();
            opts.to = NumfmtUnit::Si;
            let result = numfmt_transform_to(1000.0, &opts, NumfmtRoundMethod::Nearest, 1).unwrap();
            assert_eq!(result, "1.0K"); // 假设这里是以k为后缀
        }

        #[test]
        fn test_non_numeric_input() {
            let opts = setup_transform_options();
            let result = numfmt_transform_to(
                "abc".parse().unwrap_or_default(),
                &opts,
                NumfmtRoundMethod::Nearest,
                2,
            );
            println!("{result:?}");
            assert_eq!(result.unwrap(), "0.00");
        }

        #[test]
        fn test_extreme_decimal_precision() {
            let opts = setup_transform_options();
            let result =
                numfmt_transform_to(123.456, &opts, NumfmtRoundMethod::Nearest, 10).unwrap();
            assert_eq!(result, "123.4560000000");
        }

        #[test]
        fn test_extreme_unit_ratio_conversion() {
            let mut opts = setup_transform_options();
            opts.to_unit = 1_000_000; // 使用极端大的单位比例
            let result =
                numfmt_transform_to(1_000_000_000.0, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, "1000");
        }

        #[test]
        fn test_zero_input() {
            let opts = setup_transform_options();
            let result = numfmt_transform_to(0.0, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(result, "0.00");
        }

        #[test]
        fn test_very_small_input_values() {
            let opts = setup_transform_options();
            let result =
                numfmt_transform_to(0.000001234, &opts, NumfmtRoundMethod::Nearest, 10).unwrap();
            assert_eq!(result, "0.0000012340");
        }

        #[test]
        fn test_invalid_rounding_method() {
            let opts = setup_transform_options();
            let result = numfmt_transform_to(123.456, &opts, NumfmtRoundMethod::Up, 2);
            println!("{result:?}");
            assert_eq!(result.unwrap(), "123.46");
        }

        #[test]
        fn test_input_with_special_characters() {
            let opts = setup_transform_options();
            let result = numfmt_transform_to(
                "12.34$".parse().unwrap_or_default(),
                &opts,
                NumfmtRoundMethod::Nearest,
                2,
            );
            println!("{result:?}");
            assert_eq!(result.unwrap(), "0.00");
        }

        #[test]
        fn test_rounding_boundary_conditions() {
            let opts = setup_transform_options();
            let round_up_at_boundary =
                numfmt_transform_to(2.995, &opts, NumfmtRoundMethod::Up, 2).unwrap();
            assert_eq!(round_up_at_boundary, "3.00");

            let round_down_at_boundary =
                numfmt_transform_to(2.994, &opts, NumfmtRoundMethod::Down, 2).unwrap();
            assert_eq!(round_down_at_boundary, "2.99");
        }

        #[test]
        fn test_performance() {
            let opts = setup_transform_options();
            let start_time = std::time::Instant::now();
            for _ in 0..10000 {
                let _ = numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 2);
            }
            let duration = start_time.elapsed();
            assert!(
                duration.as_secs_f64() < 1.0,
                "Performance test failed, took too long"
            );
        }
    }
    #[cfg(test)]
    mod format_string_tests {
        use super::*;
        use crate::NumfmtFormatOptions;

        #[test]
        fn test_format_string_unscaled_decimal_rounding_methods() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::None;
            config.format.precision = Some(1);

            let cases = [
                (NumfmtRoundMethod::Up, "1.11", "1.2"),
                (NumfmtRoundMethod::Up, "-1.11", "-1.1"),
                (NumfmtRoundMethod::Down, "1.19", "1.1"),
                (NumfmtRoundMethod::Down, "-1.11", "-1.2"),
                (NumfmtRoundMethod::FromZero, "1.11", "1.2"),
                (NumfmtRoundMethod::FromZero, "-1.11", "-1.2"),
                (NumfmtRoundMethod::TowardsZero, "1.19", "1.1"),
                (NumfmtRoundMethod::TowardsZero, "-1.19", "-1.1"),
                (NumfmtRoundMethod::Nearest, "1.15", "1.2"),
                (NumfmtRoundMethod::Nearest, "-1.15", "-1.2"),
            ];

            for (round, input, expected) in cases {
                config.round = round;
                assert_eq!(
                    numfmt_format_string(input, &config, None).unwrap(),
                    expected
                );
            }
        }

        #[test]
        fn test_format_string_basic() {
            let config = setup_default_config();
            let s = "123456789";
            let shell_expected_output = "124M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_field_selection() {
            let mut config = setup_default_config();
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "123456789";
            let shell_expected_output = "124M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_basic_err() {
            let mut config = setup_default_config();
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "123456789\n234567890";
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
        }

        #[test]
        fn test_format_string_padding_and_alignment() {
            let mut config = setup_default_config();
            config.padding = 10;
            let s = "123456789";
            let shell_expected_output = "124M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_padding_and_alignment_9() {
            let mut config = setup_default_config();
            config.padding = 9;
            let s = "1";
            let shell_expected_output = "1".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_auto_to_auto_err() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output = "Unit 'auto' isn't supported with --to options".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_none_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output = "Unit 'auto' isn't supported with --to options".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_false_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output = "Unit 'auto' isn't supported with --to options".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_iec_true_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output = "Unit 'auto' isn't supported with --to options".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_si_true_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output = "Unit 'auto' isn't supported with --to options".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_auto_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output = "102420484096".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_none_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output = "102420484096".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_false_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output = "102420484096".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_iec_true_to_none_102420484096() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output = "102420484096".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_true_to_none_102420484096i() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096i";
            let shell_expected_output = "invalid suffix in input: '102420484096i'".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_true_to_none_102420484096ki() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096Ki";
            let shell_expected_output = "104878575714304".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_si_to_none_102420484096ki() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096Ki";
            let shell_expected_output = "This suffix is unsupported for specified unit".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_si_to_none_102420484096k() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096K";
            let shell_expected_output = "102420484096000".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_si_to_none_102420484096() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output = "102420484096".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_si_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_iec_false_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_true_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_none_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_auto_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_suffix_some_f() {
            let mut config = setup_default_config();
            config.suffix = Some("f".to_string());
            let s = "102420484096";
            let shell_expected_output = "103Gf".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_suffix_none() {
            let mut config = setup_default_config();
            config.suffix = None;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_default() {
            let mut config = setup_default_config();
            config.format = Default::default();

            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: None,
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config_grouping_true() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: true,
                padding: None,
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_format_config_zero_padding_true() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: None,
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: true,
            };

            let s = "102420484096";
            let shell_expected_output = "+103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config_padding_1() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: Some(1),
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config_padding_10() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: Some(10),
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+      103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_format_config_padding_10_zero_padding_true() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: Some(10),
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: true,
            };

            let s = "102420484096";
            let shell_expected_output = "+000000103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config_padding_10_precision_10() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: Some(10),
                precision: Some(10),
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+102.4204840960G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_header_0() {
            let mut config = setup_default_config();
            config.header = 0;

            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_header() {
            let mut config = setup_default_config();
            config.header = 2;

            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_rounding_down() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Down;
            let s = "102420484.096";
            let shell_expected_output = "102M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_rounding_from_zero() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::FromZero;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_rounding_towards_zero() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::TowardsZero;
            let s = "102420484.096";
            let shell_expected_output = "102M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_rounding_nearest() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Nearest;
            let s = "102420484.096";
            let shell_expected_output = "102M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_rounding_up() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Up;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_invalid_about() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Abort;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_invalid_fail() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Fail;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_invalid_warn() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Warn;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_invalid_ignore() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Ignore;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_null_handling() {
            let config = setup_default_config();
            let s = ""; // 非法输入
            let shell_expected_output = "invalid number: ''".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{output:?}");
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }
    }

    #[cfg(test)]
    mod format_and_print_delimited_tests {
        use super::*;
        #[test]
        fn test_basic_delimited() {
            let mut config = setup_default_config();
            config.delimiter = Some(" ".to_string());
            let s = "123 456 789";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }

        #[test]
        fn test_custom_delimiter() {
            let mut config = setup_default_config();
            config.delimiter = Some(",".to_string());
            let s = "100,200,300";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }

        #[test]
        fn test_field_selection() {
            let mut config = setup_default_config();
            config.delimiter = Some(" ".to_string());
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "100 200 300";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }

        #[test]
        fn test_transformations() {
            let mut config = setup_default_config();
            config.delimiter = Some(" ".to_string());
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }

        #[test]
        fn test_edge_cases() {
            let mut config = setup_default_config();
            config.delimiter = Some(",".to_string());
            let s = ",100,,200,";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }
    }
    #[cfg(test)]
    mod format_and_print_whitespace_tests {
        use super::*;
        use crate::flags::NumfmtFormatOptions;
        #[test]
        fn test_basic() {
            let config = setup_default_config();
            let s = "123 456 789";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_field_selection() {
            let mut config = setup_default_config();
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "123 456 789";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_basic2() {
            let mut config = setup_default_config();
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "123 456 789\n234 567 890";
            let _shell_expected_output = "";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_padding_and_alignment() {
            let mut config = setup_default_config();
            config.padding = 10;
            let s = "123 456 789";
            let _shell_expected_output = "       123       456       789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_padding_and_alignment_9() {
            let mut config = setup_default_config();
            config.padding = 9;
            let s = "1";
            let _shell_expected_output = "         1\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_auto_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_none_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_iec_false_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_iec_true_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_si_true_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_auto_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_none_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_iec_false_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_iec_true_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_si_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_si_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_iec_false_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_iec_true_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_none_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_auto_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_suffix_some_f() {
            let mut config = setup_default_config();
            config.suffix = Some("f".to_string());

            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_suffix_none() {
            let mut config = setup_default_config();
            config.suffix = None;

            let s = "1024 2048 4096";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_format_default() {
            let mut config = setup_default_config();
            config.format = Default::default();

            let s = "1024 2048 4096";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_format_config() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: None,
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_header_0() {
            let mut config = setup_default_config();
            config.header = 0;

            let s = "1024 2048 4096\n11111\n222222\n333333\n444444\n555555\n666666\n777777\n888888\n999999\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_header() {
            let mut config = setup_default_config();
            config.header = 2;

            let s = "1024 2048 4096\n11111\n222222\n333333\n444444\n555555\n666666\n777777\n888888\n999999\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_rounding_down() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Down;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_rounding_from_zero() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::FromZero;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_rounding_towards_zero() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::TowardsZero;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_rounding_nearest() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Nearest;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_rounding_up() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Up;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_invalid_about() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Abort;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_invalid_fail() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Fail;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_invalid_warn() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Warn;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_invalid_ignore() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Ignore;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_null_handling() {
            let config = setup_default_config();
            let s = ""; // 非法输入
            assert!(numfmt_format_and_print_whitespace(s, &config).is_ok());
        }
    }
    #[cfg(test)]
    mod format_and_print_tests {
        use super::*;
        #[test]
        fn test_numfmt_format_and_print_with_delimiter() {
            let mut config = setup_default_config();
            config.delimiter = Some(','.to_string());

            assert!(numfmt_format_and_print("123456", &config).is_ok());
        }
        #[test]
        fn test_numfmt_format_and_print_without_delimiter() {
            let mut config = setup_default_config();
            config.delimiter = None;

            assert!(numfmt_format_and_print("123456", &config).is_ok());
        }
        // 测试错误情况，例如无效的配置
        #[test]
        fn test_numfmt_format_and_print_with_invalid_config() {
            let mut config = setup_default_config();
            config.delimiter = Some('\0'.to_string());

            assert!(numfmt_format_and_print("123456", &config).is_ok());
        }
        // 测试空字符串
        #[test]
        fn test_numfmt_format_and_print_with_empty_string() {
            let mut config = setup_default_config();
            config.delimiter = Some('\t'.to_string());

            assert!(numfmt_format_and_print("", &config).is_ok());
        }
        // 测试大数字
        #[test]
        fn test_numfmt_format_and_print_large_number() {
            let mut config = setup_default_config();
            config.delimiter = Some('.'.to_string());
            assert!(numfmt_format_and_print("1234567890", &config).is_ok());
        }
    }

    #[cfg(test)]
    mod round_with_precision_tests {
        use super::*;
        #[test]
        fn test_numfmt_round_with_precision() {
            // Test case 1: Round down with precision 0
            let result1 =
                numfmt_round_with_precision(std::f64::consts::PI, NumfmtRoundMethod::Down, 0);
            assert_eq!(result1, 3.0);

            // Test case 2: Round up with precision 2
            let result2 =
                numfmt_round_with_precision(std::f64::consts::PI, NumfmtRoundMethod::Up, 2);
            assert_eq!(result2, 3.15);

            // Test case 3: Round to nearest with precision 1
            let result3 =
                numfmt_round_with_precision(std::f64::consts::PI, NumfmtRoundMethod::Nearest, 1);
            assert_eq!(result3, 3.1);
        }
        #[test]
        #[allow(clippy::cognitive_complexity)]
        fn test_base_round_with_precision() {
            let rm = NumfmtRoundMethod::FromZero;
            assert_eq!(1.0, numfmt_round_with_precision(0.12345, rm, 0));
            assert_eq!(0.2, numfmt_round_with_precision(0.12345, rm, 1));
            assert_eq!(0.13, numfmt_round_with_precision(0.12345, rm, 2));
            assert_eq!(0.124, numfmt_round_with_precision(0.12345, rm, 3));
            assert_eq!(0.1235, numfmt_round_with_precision(0.12345, rm, 4));
            assert_eq!(0.12345, numfmt_round_with_precision(0.12345, rm, 5));

            let rm = NumfmtRoundMethod::TowardsZero;
            assert_eq!(0.0, numfmt_round_with_precision(0.12345, rm, 0));
            assert_eq!(0.1, numfmt_round_with_precision(0.12345, rm, 1));
            assert_eq!(0.12, numfmt_round_with_precision(0.12345, rm, 2));
            assert_eq!(0.123, numfmt_round_with_precision(0.12345, rm, 3));
            assert_eq!(0.1234, numfmt_round_with_precision(0.12345, rm, 4));
            assert_eq!(0.12345, numfmt_round_with_precision(0.12345, rm, 5));
        }

        #[test]
        fn test_base_parse_implicit_precision() {
            assert_eq!(0, numfmt_parse_implicit_precision(""));
            assert_eq!(0, numfmt_parse_implicit_precision("1"));
            assert_eq!(1, numfmt_parse_implicit_precision("1.2"));
            assert_eq!(2, numfmt_parse_implicit_precision("1.23"));
            assert_eq!(3, numfmt_parse_implicit_precision("1.234"));
            assert_eq!(0, numfmt_parse_implicit_precision("1K"));
            assert_eq!(1, numfmt_parse_implicit_precision("1.2K"));
            assert_eq!(2, numfmt_parse_implicit_precision("1.23K"));
            assert_eq!(3, numfmt_parse_implicit_precision("1.234K"));
        }
    }
}
