/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of the License at:
 *          http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY KIND,
 * EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO NON-INFRINGEMENT,
 * MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 */

use std::cmp::Ordering;
use std::error::Error;
use std::fmt::{Display, Formatter};

use num_bigint::BigUint;
use num_traits::{One, Zero};

use crate::extendedbigdecimal::ExtendedBigDecimal;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const LONG_DOUBLE_PRECISION: usize = 64;
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "riscv64",
    target_arch = "s390x"
))]
const LONG_DOUBLE_PRECISION: usize = 113;
#[cfg(not(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "riscv64",
    target_arch = "s390x"
)))]
const LONG_DOUBLE_PRECISION: usize = 53;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const X87_HEX_LAYOUT: bool = true;
#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
const X87_HEX_LAYOUT: bool = false;

#[derive(Debug, Default)]
struct Flags {
    left: bool,
    plus: bool,
    space: bool,
    alternate: bool,
    zero: bool,
}

#[derive(Debug)]
pub struct GnuFloatFormat {
    prefix: String,
    suffix: String,
    flags: Flags,
    width: usize,
    precision: Option<usize>,
    conversion: u8,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GnuFormatError {
    NoDirective(String),
    EndsInPercent(String),
    UnknownDirective(String, u8),
    TooManyDirectives(String),
}

impl Error for GnuFormatError {}

impl ctcore::ct_error::CTError for GnuFormatError {}

impl Display for GnuFormatError {
    fn fmt(&self, output: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDirective(format) => {
                write!(output, "format '{format}' has no % directive")
            }
            Self::EndsInPercent(format) => write!(output, "format '{format}' ends in %"),
            Self::UnknownDirective(format, directive) => write!(
                output,
                "format '{format}' has unknown %{} directive",
                char::from(*directive)
            ),
            Self::TooManyDirectives(format) => {
                write!(output, "format '{format}' has too many % directives")
            }
        }
    }
}

impl GnuFloatFormat {
    pub fn try_parse(format: &str) -> Result<Self, GnuFormatError> {
        let bytes = format.as_bytes();
        let mut percent = 0;
        while percent < bytes.len() {
            if bytes[percent] != b'%' {
                percent += 1;
            } else if bytes.get(percent + 1) == Some(&b'%') {
                percent += 2;
            } else {
                break;
            }
        }
        if percent == bytes.len() {
            return Err(GnuFormatError::NoDirective(format.to_string()));
        }

        let mut index = percent + 1;
        let mut flags = Flags::default();
        while let Some(flag) = bytes.get(index) {
            match flag {
                b'-' => flags.left = true,
                b'+' => flags.plus = true,
                b' ' => flags.space = true,
                b'#' => flags.alternate = true,
                b'0' => flags.zero = true,
                b'\'' => {}
                _ => break,
            }
            index += 1;
        }

        let width_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        let width = parse_usize(&format[width_start..index]);

        let precision = if bytes.get(index) == Some(&b'.') {
            index += 1;
            let precision_start = index;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
            Some(parse_usize(&format[precision_start..index]))
        } else {
            None
        };

        if bytes.get(index) == Some(&b'L') {
            index += 1;
        }
        let Some(&conversion) = bytes.get(index) else {
            return Err(GnuFormatError::EndsInPercent(format.to_string()));
        };
        if !b"aAeEfFgG".contains(&conversion) {
            return Err(GnuFormatError::UnknownDirective(
                format.to_string(),
                conversion,
            ));
        }

        let mut suffix_index = index + 1;
        while suffix_index < bytes.len() {
            if bytes[suffix_index] != b'%' {
                suffix_index += 1;
            } else if bytes.get(suffix_index + 1) == Some(&b'%') {
                suffix_index += 2;
            } else {
                return Err(GnuFormatError::TooManyDirectives(format.to_string()));
            }
        }

        Ok(Self {
            prefix: unescape_percent(&format[..percent]),
            suffix: unescape_percent(&format[index + 1..]),
            flags,
            width,
            precision,
            conversion,
        })
    }

    pub fn format(&self, value: &ExtendedBigDecimal) -> String {
        let value = BinaryLongDouble::from(value);
        let uppercase = self.conversion.is_ascii_uppercase();
        let finite = matches!(value, BinaryLongDouble::Finite(_));
        let (negative, body) = match &value {
            BinaryLongDouble::Infinity { negative } => {
                (*negative, if uppercase { "INF" } else { "inf" }.to_string())
            }
            BinaryLongDouble::Nan { negative } => {
                (*negative, if uppercase { "NAN" } else { "nan" }.to_string())
            }
            BinaryLongDouble::Finite(number) => {
                let body = match self.conversion.to_ascii_lowercase() {
                    b'f' => number.fixed(self.precision.unwrap_or(6), self.flags.alternate),
                    b'e' => number.scientific(
                        self.precision.unwrap_or(6),
                        self.flags.alternate,
                        uppercase,
                    ),
                    b'g' => {
                        number.general(self.precision.unwrap_or(6), self.flags.alternate, uppercase)
                    }
                    b'a' => number.hexadecimal(self.precision, self.flags.alternate, uppercase),
                    _ => unreachable!("ct_format validated the conversion"),
                };
                (number.negative, body)
            }
        };

        let sign = if negative {
            "-"
        } else if self.flags.plus {
            "+"
        } else if self.flags.space {
            " "
        } else {
            ""
        };
        let number = pad_number(
            sign,
            &body,
            self.width,
            self.flags.left,
            self.flags.zero && finite,
        );
        format!("{}{}{}", self.prefix, number, self.suffix)
    }
}

fn parse_usize(value: &str) -> usize {
    if value.is_empty() {
        0
    } else {
        value.parse().unwrap_or(usize::MAX)
    }
}

fn unescape_percent(value: &str) -> String {
    value.replace("%%", "%")
}

fn pad_number(sign: &str, body: &str, width: usize, left: bool, zero: bool) -> String {
    let content_len = sign.len().saturating_add(body.len());
    if content_len >= width {
        return format!("{sign}{body}");
    }

    let padding = width - content_len;
    if left {
        format!("{sign}{body}{}", " ".repeat(padding))
    } else if zero {
        if let Some(magnitude) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            let prefix = &body[..2];
            format!("{sign}{prefix}{}{magnitude}", "0".repeat(padding))
        } else {
            format!("{sign}{}{body}", "0".repeat(padding))
        }
    } else {
        format!("{}{sign}{body}", " ".repeat(padding))
    }
}

enum BinaryLongDouble {
    Finite(BinaryFinite),
    Infinity { negative: bool },
    Nan { negative: bool },
}

struct BinaryFinite {
    negative: bool,
    significand: BigUint,
    exponent: i32,
}

impl From<&ExtendedBigDecimal> for BinaryLongDouble {
    fn from(value: &ExtendedBigDecimal) -> Self {
        match value {
            ExtendedBigDecimal::Infinity => Self::Infinity { negative: false },
            ExtendedBigDecimal::MinusInfinity => Self::Infinity { negative: true },
            ExtendedBigDecimal::Nan => Self::Nan { negative: false },
            ExtendedBigDecimal::MinusZero => Self::Finite(BinaryFinite {
                negative: true,
                significand: BigUint::zero(),
                exponent: 0,
            }),
            ExtendedBigDecimal::BigDecimal(value) => {
                let (integer, scale) = value.as_bigint_and_exponent();
                let negative = integer.sign() == num_bigint::Sign::Minus;
                let mut numerator = integer.magnitude().clone();
                let mut denominator = BigUint::one();

                if scale >= 0 {
                    denominator = power_of_ten(scale as usize);
                } else {
                    numerator *= power_of_ten(scale.unsigned_abs() as usize);
                }

                Self::Finite(BinaryFinite::from_ratio(negative, numerator, denominator))
            }
        }
    }
}

impl BinaryFinite {
    fn from_ratio(negative: bool, numerator: BigUint, denominator: BigUint) -> Self {
        if numerator.is_zero() {
            return Self {
                negative,
                significand: BigUint::zero(),
                exponent: 0,
            };
        }

        let mut exponent = floor_log2_ratio(&numerator, &denominator);
        let binary_shift = LONG_DOUBLE_PRECISION as i32 - 1 - exponent;
        let significand = if binary_shift >= 0 {
            round_ratio(numerator << binary_shift as usize, denominator)
        } else {
            round_ratio(
                numerator,
                denominator << binary_shift.unsigned_abs() as usize,
            )
        };
        let significand = if significand.bits() > LONG_DOUBLE_PRECISION as u64 {
            exponent += 1;
            significand >> 1_usize
        } else {
            significand
        };

        Self {
            negative,
            significand,
            exponent,
        }
    }

    fn fixed(&self, precision: usize, alternate: bool) -> String {
        let rounded = self.round_decimal(precision as i64);
        let mut digits = rounded.to_str_radix(10);
        if precision == 0 {
            if alternate {
                digits.push('.');
            }
            return digits;
        }

        if digits.len() <= precision {
            digits.insert_str(0, &"0".repeat(precision + 1 - digits.len()));
        }
        let point = digits.len() - precision;
        digits.insert(point, '.');
        digits
    }

    fn scientific(&self, precision: usize, alternate: bool, uppercase: bool) -> String {
        let (mut digits, exponent) = self.significant_digits(precision + 1);
        if digits.len() < precision + 1 {
            digits.push_str(&"0".repeat(precision + 1 - digits.len()));
        }

        let mut mantissa = digits[..1].to_string();
        if precision > 0 {
            mantissa.push('.');
            mantissa.push_str(&digits[1..]);
        } else if alternate {
            mantissa.push('.');
        }
        let marker = if uppercase { 'E' } else { 'e' };
        format!("{mantissa}{marker}{}", format_exponent(exponent, 2))
    }

    fn general(&self, precision: usize, alternate: bool, uppercase: bool) -> String {
        let precision = precision.max(1);
        let (_, exponent) = self.significant_digits(precision);
        if exponent < -4 || exponent >= precision as i32 {
            let value = self.scientific(precision - 1, alternate, uppercase);
            if alternate {
                value
            } else {
                trim_scientific_fraction(value)
            }
        } else {
            let decimals = (precision as i32 - exponent - 1).max(0) as usize;
            let mut value = self.fixed(decimals, alternate);
            if !alternate {
                trim_fraction(&mut value);
            }
            value
        }
    }

    fn hexadecimal(&self, precision: Option<usize>, alternate: bool, uppercase: bool) -> String {
        if self.significand.is_zero() {
            let fraction = precision.map_or_else(String::new, |p| "0".repeat(p));
            let point = if !fraction.is_empty() || alternate {
                "."
            } else {
                ""
            };
            let mut result = format!("0x0{point}{fraction}p+0");
            if uppercase {
                result.make_ascii_uppercase();
            }
            return result;
        }

        let native_fraction_digits = if X87_HEX_LAYOUT {
            LONG_DOUBLE_PRECISION / 4 - 1
        } else {
            (LONG_DOUBLE_PRECISION - 1).div_ceil(4)
        };
        let wanted_fraction_digits = precision.unwrap_or(native_fraction_digits);
        let mut exponent = self.exponent - if X87_HEX_LAYOUT { 3 } else { 0 };
        let mut digits = if wanted_fraction_digits < native_fraction_digits {
            let drop = 4 * (native_fraction_digits - wanted_fraction_digits);
            round_shift_right(self.significand.clone(), drop)
        } else {
            self.significand.clone()
        };

        let wanted_digits = wanted_fraction_digits + 1;
        if digits.to_str_radix(16).len() > wanted_digits {
            digits >>= 1_usize;
            exponent += 1;
        }

        let mut hex = digits.to_str_radix(16);
        let stored_digits = native_fraction_digits + 1;
        let current_digits = if wanted_fraction_digits > native_fraction_digits {
            stored_digits
        } else {
            wanted_digits
        };
        if hex.len() < current_digits {
            hex.insert_str(0, &"0".repeat(current_digits - hex.len()));
        }
        if wanted_fraction_digits > native_fraction_digits {
            hex.push_str(&"0".repeat(wanted_fraction_digits - native_fraction_digits));
        }

        let first = hex.remove(0);
        let mut fraction = hex;
        if precision.is_none() {
            while fraction.ends_with('0') {
                fraction.pop();
            }
        }
        let point = if !fraction.is_empty() || alternate {
            "."
        } else {
            ""
        };
        let mut result = format!(
            "0x{first}{point}{fraction}p{}",
            format_exponent(exponent, 1)
        );
        if uppercase {
            result.make_ascii_uppercase();
        }
        result
    }

    fn round_decimal(&self, decimal_shift: i64) -> BigUint {
        if self.significand.is_zero() {
            return BigUint::zero();
        }

        let binary_shift = self.exponent - (LONG_DOUBLE_PRECISION as i32 - 1);
        let mut numerator = self.significand.clone();
        let mut denominator = BigUint::one();
        if binary_shift >= 0 {
            numerator <<= binary_shift as usize;
        } else {
            denominator <<= binary_shift.unsigned_abs() as usize;
        }
        if decimal_shift >= 0 {
            numerator *= power_of_ten(decimal_shift as usize);
        } else {
            denominator *= power_of_ten(decimal_shift.unsigned_abs() as usize);
        }
        round_ratio(numerator, denominator)
    }

    fn significant_digits(&self, count: usize) -> (String, i32) {
        if self.significand.is_zero() {
            return ("0".repeat(count), 0);
        }

        let mut exponent = self.decimal_exponent();
        let mut rounded = self.round_decimal(count as i64 - 1 - exponent as i64);
        let upper = power_of_ten(count);
        if rounded >= upper {
            rounded /= 10_u8;
            exponent += 1;
        }
        let mut digits = rounded.to_str_radix(10);
        if digits.len() < count {
            digits.insert_str(0, &"0".repeat(count - digits.len()));
        }
        (digits, exponent)
    }

    fn decimal_exponent(&self) -> i32 {
        if self.significand.is_zero() {
            return 0;
        }

        let mut exponent = (self.exponent as f64 * std::f64::consts::LOG10_2).floor() as i32;
        while self.compare_power_of_ten(exponent) == Ordering::Less {
            exponent -= 1;
        }
        while self.compare_power_of_ten(exponent + 1) != Ordering::Less {
            exponent += 1;
        }
        exponent
    }

    fn compare_power_of_ten(&self, decimal_exponent: i32) -> Ordering {
        let binary_shift = self.exponent - (LONG_DOUBLE_PRECISION as i32 - 1);
        let mut left = self.significand.clone();
        let mut right = BigUint::one();
        if binary_shift >= 0 {
            left <<= binary_shift as usize;
        } else {
            right <<= binary_shift.unsigned_abs() as usize;
        }

        if decimal_exponent >= 0 {
            right *= power_of_ten(decimal_exponent as usize);
        } else {
            left *= power_of_ten(decimal_exponent.unsigned_abs() as usize);
        }
        left.cmp(&right)
    }
}

fn floor_log2_ratio(numerator: &BigUint, denominator: &BigUint) -> i32 {
    let candidate = numerator.bits() as i32 - denominator.bits() as i32;
    let below = if candidate >= 0 {
        numerator < &(denominator << candidate as usize)
    } else {
        &(numerator << candidate.unsigned_abs() as usize) < denominator
    };
    if below { candidate - 1 } else { candidate }
}

fn round_ratio(numerator: BigUint, denominator: BigUint) -> BigUint {
    let quotient = &numerator / &denominator;
    let remainder = numerator % &denominator;
    let twice_remainder = remainder << 1_usize;
    if twice_remainder > denominator
        || (twice_remainder == denominator && (&quotient & BigUint::one()) == BigUint::one())
    {
        quotient + 1_u8
    } else {
        quotient
    }
}

fn round_shift_right(value: BigUint, shift: usize) -> BigUint {
    if shift == 0 {
        return value;
    }
    round_ratio(value, BigUint::one() << shift)
}

fn power_of_ten(exponent: usize) -> BigUint {
    BigUint::from(10_u8).pow(exponent.try_into().unwrap_or(u32::MAX))
}

fn format_exponent(exponent: i32, minimum_digits: usize) -> String {
    let magnitude = exponent.unsigned_abs().to_string();
    let zeroes = "0".repeat(minimum_digits.saturating_sub(magnitude.len()));
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{sign}{zeroes}{magnitude}")
}

fn trim_fraction(value: &mut String) {
    if let Some(point) = value.find('.') {
        while value.ends_with('0') {
            value.pop();
        }
        if value.len() == point + 1 {
            value.pop();
        }
    }
}

fn trim_scientific_fraction(value: String) -> String {
    let marker = value
        .find(['e', 'E'])
        .expect("scientific value has exponent");
    let mut mantissa = value[..marker].to_string();
    trim_fraction(&mut mantissa);
    mantissa + &value[marker..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::number::PreciseNumber;

    fn render(format: &str, value: &str) -> String {
        let value = value.parse::<PreciseNumber>().unwrap().number;
        GnuFloatFormat::try_parse(format).unwrap().format(&value)
    }

    #[test]
    fn uses_printf_default_precision() {
        assert_eq!(render("%e", "1"), "1.000000e+00");
        assert_eq!(render("%E", "2"), "2.000000E+00");
        assert_eq!(render("%f", "1"), "1.000000");
        assert_eq!(render("%F", "2"), "2.000000");
    }

    #[test]
    fn applies_zero_padding_after_the_sign() {
        assert_eq!(render("%05.1f", "1"), "001.0");
        assert_eq!(render("%+05.1f", "1"), "+01.0");
        assert_eq!(render("%05.1f", "-1"), "-01.0");
    }

    #[test]
    fn preserves_literal_prefix_and_suffix() {
        assert_eq!(render("%% value=%f %%", "1"), "% value=1.000000 %");
    }

    #[test]
    fn general_format_uses_the_rounded_exponent() {
        assert_eq!(render("%.1g", "9.9995"), "1e+01");
    }

    #[test]
    fn uses_target_long_double_hex_layout() {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        assert_eq!(render("%a", "1"), "0x8p-3");
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        assert_eq!(render("%a", "1"), "0x1p+0");
    }

    #[test]
    fn rejects_non_gnu_directives_without_panicking() {
        for (format, expected) in [
            ("%%g", GnuFormatError::NoDirective("%%g".to_string())),
            ("%", GnuFormatError::EndsInPercent("%".to_string())),
            (
                "%lf",
                GnuFormatError::UnknownDirective("%lf".to_string(), b'l'),
            ),
            (
                "%1$f",
                GnuFormatError::UnknownDirective("%1$f".to_string(), b'$'),
            ),
            (
                "%g%g",
                GnuFormatError::TooManyDirectives("%g%g".to_string()),
            ),
        ] {
            assert_eq!(GnuFloatFormat::try_parse(format).unwrap_err(), expected);
        }
    }
}
