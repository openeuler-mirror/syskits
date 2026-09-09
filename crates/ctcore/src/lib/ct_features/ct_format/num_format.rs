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

//! 用于以各种格式格式化数字的实用程序

use std::io::Write;

use super::{
    FormatError,
    spec::{CanAsterisk, Spec},
};

pub trait Formatter {
    type Input;
    fn fmt(&self, writer: impl Write, x: Self::Input) -> std::io::Result<()>;
    fn try_from_spec(s: Spec) -> Result<Self, FormatError>
    where
        Self: Sized;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsignedIntVariant {
    Decimal,
    Octal(Prefix),
    Hexadecimal(Case, Prefix),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FloatVariant {
    Decimal,
    Scientific,
    Shortest,
    Hexadecimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    Lowercase,
    Uppercase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefix {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForceDecimal {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PositiveSign {
    None,
    Plus,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberAlignment {
    Left,
    RightSpace,
    RightZero,
}

pub struct SignedInt {
    pub width: usize,
    pub precision: Option<usize>,
    pub positive_sign: PositiveSign,
    pub alignment: NumberAlignment,
}

impl Formatter for SignedInt {
    type Input = i64;

    // 该函数负责根据结构体中提供的规范格式化无符号整数值。
    fn fmt(&self, mut writer: impl Write, x: Self::Input) -> std::io::Result<()> {
        let prefix = if x < 0 {
            "-"
        } else {
            match self.positive_sign {
                PositiveSign::None => "",
                PositiveSign::Plus => "+",
                PositiveSign::Space => " ",
            }
        };
        let raw_digits = if x == 0 && self.precision == Some(0) {
            String::new()
        } else {
            x.unsigned_abs().to_string()
        };
        let precision_zeros = self.precision.unwrap_or(0).saturating_sub(raw_digits.len());
        let digits = format!("{}{raw_digits}", "0".repeat(precision_zeros));

        write_integer_aligned(&mut writer, prefix, &digits, self.width, self.alignment)
    }

    // 它定义了一个名为 try_from_spec 的方法，用于尝试从 Spec 实例创建一个新的 SignedInt 实例。
    // Spec 结构是各种格式化规范的通用容器。
    fn try_from_spec(s: Spec) -> Result<Self, FormatError> {
        if let Spec::SignedInt {
            width,
            precision,
            positive_sign,
            alignment,
            thousand_separate: _,
        } = s
        {
            let width = if let Some(CanAsterisk::Fixed(x)) = width {
                x
            } else if width.is_none() {
                0
            } else {
                return Err(FormatError::WrongSpecType);
            };

            let precision = match precision {
                Some(CanAsterisk::Fixed(x)) => Some(x),
                None => None,
                Some(CanAsterisk::Asterisk) => return Err(FormatError::WrongSpecType),
            };

            Ok(Self {
                width,
                precision,
                positive_sign,
                alignment,
            })
        } else {
            Err(FormatError::WrongSpecType)
        }
    }
}

pub struct UnsignedInt {
    pub width: usize,
    pub precision: Option<usize>,
    pub alignment: NumberAlignment,
    pub variant: UnsignedIntVariant,
}

impl Formatter for UnsignedInt {
    type Input = u64;

    // 该函数负责根据提供的规范格式化无符号整数值。
    fn fmt(&self, mut writer: impl Write, x: Self::Input) -> std::io::Result<()> {
        let raw_digits = if x == 0 && self.precision == Some(0) {
            String::new()
        } else {
            match self.variant {
                UnsignedIntVariant::Decimal => format!("{x}"),
                UnsignedIntVariant::Octal(_) => format!("{x:o}"),
                UnsignedIntVariant::Hexadecimal(Case::Lowercase, _) => format!("{x:x}"),
                UnsignedIntVariant::Hexadecimal(Case::Uppercase, _) => format!("{x:X}"),
            }
        };
        let precision_zeros = self.precision.unwrap_or(0).saturating_sub(raw_digits.len());
        let digits = format!("{}{raw_digits}", "0".repeat(precision_zeros));
        let prefix = match (x, self.variant) {
            (1.., UnsignedIntVariant::Hexadecimal(Case::Lowercase, Prefix::Yes)) => "0x",
            (1.., UnsignedIntVariant::Hexadecimal(Case::Uppercase, Prefix::Yes)) => "0X",
            (_, UnsignedIntVariant::Octal(Prefix::Yes)) if !digits.starts_with('0') => "0",
            _ => "",
        };

        write_integer_aligned(&mut writer, prefix, &digits, self.width, self.alignment)
    }

    // 该函数负责从提供的 Spec 结构创建 SignedInt 结构的新实例。
    // Spec 结构是各种格式化规范的通用容器。
    fn try_from_spec(s: Spec) -> Result<Self, FormatError> {
        // A signed int spec might be mapped to an unsigned int spec if no sign is specified
        let s = if let Spec::SignedInt {
            width,
            precision,
            positive_sign: PositiveSign::None,
            alignment,
            thousand_separate: _,
        } = s
        {
            Spec::UnsignedInt {
                variant: UnsignedIntVariant::Decimal,
                width,
                precision,
                alignment,
                thousand_separate: false,
            }
        } else {
            s
        };

        if let Spec::UnsignedInt {
            variant,
            width,
            precision,
            alignment,
            thousand_separate: _,
        } = s
        {
            let width = if let Some(CanAsterisk::Fixed(x)) = width {
                x
            } else if width.is_none() {
                0
            } else {
                return Err(FormatError::WrongSpecType);
            };

            let precision = match precision {
                Some(CanAsterisk::Fixed(x)) => Some(x),
                None => None,
                Some(CanAsterisk::Asterisk) => return Err(FormatError::WrongSpecType),
            };

            Ok(Self {
                width,
                precision,
                variant,
                alignment,
            })
        } else {
            Err(FormatError::WrongSpecType)
        }
    }
}

fn write_integer_aligned(
    mut writer: impl Write,
    prefix: &str,
    digits: &str,
    width: usize,
    alignment: NumberAlignment,
) -> std::io::Result<()> {
    let padding = width.saturating_sub(prefix.len() + digits.len());
    match alignment {
        NumberAlignment::Left => write!(writer, "{prefix}{digits}{}", " ".repeat(padding)),
        NumberAlignment::RightSpace => {
            write!(writer, "{}{prefix}{digits}", " ".repeat(padding))
        }
        NumberAlignment::RightZero => {
            write!(writer, "{prefix}{}{digits}", "0".repeat(padding))
        }
    }
}

pub struct Float {
    pub variant: FloatVariant,
    pub case: Case,
    pub force_decimal: ForceDecimal,
    pub width: usize,
    pub positive_sign: PositiveSign,
    pub alignment: NumberAlignment,
    pub precision: Option<usize>,
}

impl Default for Float {
    fn default() -> Self {
        Self {
            width: 0,
            precision: None,
            case: Case::Lowercase,
            variant: FloatVariant::Decimal,
            force_decimal: ForceDecimal::No,
            alignment: NumberAlignment::Left,
            positive_sign: PositiveSign::None,
        }
    }
}

impl Formatter for Float {
    type Input = f64;

    // 该方法负责根据 Float 结构的规范格式化输入的 Float 值 x。
    fn fmt(&self, mut writer: impl Write, x: Self::Input) -> std::io::Result<()> {
        if x.is_sign_positive() {
            match self.positive_sign {
                PositiveSign::None => Ok(()),
                PositiveSign::Plus => write!(writer, "+"),
                PositiveSign::Space => write!(writer, " "),
            }?;
        }

        let s = match x.is_finite() {
            true => {
                if self.variant == FloatVariant::Decimal {
                    format_float_decimal(x, self.precision.unwrap_or(6), self.force_decimal)
                } else if self.variant == FloatVariant::Scientific {
                    format_float_scientific(
                        x,
                        self.precision.unwrap_or(6),
                        self.case,
                        self.force_decimal,
                    )
                } else if self.variant == FloatVariant::Shortest {
                    format_float_shortest(
                        x,
                        self.precision.unwrap_or(6),
                        self.case,
                        self.force_decimal,
                    )
                } else {
                    // self.variant == FloatVariant::Hexadecimal
                    format_float_hexadecimal(x, self.precision, self.case, self.force_decimal)
                }
            }
            false => format_float_non_finite(x, self.case),
        };

        if self.alignment == NumberAlignment::Left {
            write!(writer, "{s:<width$}", s = s, width = self.width)
        } else if self.alignment == NumberAlignment::RightSpace {
            write!(writer, "{s:>width$}", s = s, width = self.width)
        } else {
            // self.alignment == NumberAlignment::RightZero
            write!(writer, "{s:0>width$}", s = s, width = self.width)
        }
    }

    // 函数是 Float 结构的一个方法。
    // 该函数负责将 Spec 结构解析为 Float 结构。
    // Spec 结构是一个通用容器，用于容纳各种类型的规范。
    // 在本例中，它用于指定浮点数的格式。
    fn try_from_spec(s: Spec) -> Result<Self, FormatError>
    where
        Self: Sized,
    {
        if let Spec::Float {
            variant,
            case,
            force_decimal,
            width,
            positive_sign,
            alignment,
            precision,
            thousand_separate: _,
        } = s
        {
            let width = if let Some(CanAsterisk::Fixed(x)) = width {
                x
            } else if width.is_none() {
                0
            } else {
                return Err(FormatError::WrongSpecType);
            };

            let precision = if let Some(CanAsterisk::Fixed(x)) = precision {
                Some(x)
            } else if precision.is_none() {
                None
            } else {
                return Err(FormatError::WrongSpecType);
            };

            Ok(Self {
                case,
                width,
                variant,
                alignment,
                precision,
                positive_sign,
                force_decimal,
            })
        } else {
            Err(FormatError::WrongSpecType)
        }
    }
}

// 负责格式化非无限浮点数。该函数将浮点数 f 和 Case 枚举值作为输入参数。
// Case 枚举有两种变体： Case::Lowercase（小写）和 Case::Uppercase（大写），
// 用于指定输出字符串是小写还是大写。
fn format_float_non_finite(f: f64, case: Case) -> String {
    debug_assert!(!f.is_finite());

    let s = format!("{f}");

    match case {
        Case::Uppercase => s.to_ascii_uppercase(),
        _ => s,
    }
}

// 该函数负责根据指定的精度和强制小数设置格式化浮点数。
// 函数首先检查精度是否为零，强制小数设置是否为 "是"。
// 如果是，它将格式化带有小数点和零精度的浮点数。
fn format_float_decimal(f: f64, precision: usize, force_decimal: ForceDecimal) -> String {
    match (precision, force_decimal) {
        (0, ForceDecimal::Yes) => format!("{f:.0}."),
        _ => format!("{f:.precision$}"),
    }
}

// 该函数用科学计数法格式化浮点数。函数首先检查输入数字是否为零。
// 如果是，则在强制十进制为 "是 "且精度为零的情况下，将数字格式化为 "0.e+00"。
// 否则，它将把数字格式化为 "0.000...00e+00"。
fn format_float_scientific(
    f: f64,
    precision: usize,
    case: Case,
    force_decimal: ForceDecimal,
) -> String {
    if f.abs() < f64::EPSILON {
        let new_result = match (force_decimal, precision) {
            (ForceDecimal::Yes, 0) => "0.e+00".into(),
            _ => format!("{:.*}e+00", precision, 0.0),
        };

        return new_result;
    }

    let mut exponent: i32 = f.abs().log10().floor() as i32;
    let mut normalized = f / 10.0_f64.powi(exponent);

    // 如果规范化后的值将被舍入为大于 10 的值，我们需要进行修正。
    let tmp_value = normalized * 10_f64.powi(precision as i32);
    let value = tmp_value.round() / 10_f64.powi(precision as i32);
    if value.abs() >= 10.0 {
        normalized /= 10.0;
        exponent += 1;
    }

    let additional_dot = match (precision, force_decimal) {
        (0, ForceDecimal::Yes) => ".",
        _ => "",
    };

    let exp_char = if Case::Lowercase == case { 'e' } else { 'E' };

    format!("{normalized:.precision$}{additional_dot}{exp_char}{exponent:+03}")
}

// 该函数负责将浮点数格式化为最短格式。这种格式是十进制和科学记数法的混合体，
// 但也有一些区别。
fn format_float_shortest(
    f: f64,
    precision: usize,
    case: Case,
    force_decimal: ForceDecimal,
) -> String {
    // 此处的精度是指应显示多少位数，而不是小数部分的位数，这意味着如果我们将其传递给Rust的格式字符串，它总是少一位。
    let precision = precision.saturating_sub(1);

    if f.abs() < f64::EPSILON {
        let new_value = if force_decimal == ForceDecimal::Yes {
            if precision == 0 {
                "0.".into()
            } else {
                format!("{:.*}", precision, 0.0)
            }
        } else {
            "0".into()
        };
        return new_value;
    }

    let mut exponent = f.abs().log10().floor() as i32;
    let mut normalized = f / 10.0_f64.powi(exponent);

    // %g selects its notation from the exponent after rounding to the requested
    // number of significant digits.
    let scale = 10_f64.powi(precision as i32);
    let rounded_normalized = (normalized * scale).round() / scale;
    if rounded_normalized.abs() >= 10.0 {
        normalized /= 10.0;
        exponent += 1;
    }

    if exponent < -4 || exponent > precision as i32 {
        // 类似科学记数法（有几个不同之处）
        let additional_dot = match (precision, force_decimal) {
            (0, ForceDecimal::Yes) => ".",
            _ => "",
        };

        let mut normalized = format!("{normalized:.precision$}");

        if ForceDecimal::No == force_decimal {
            strip_fractional_zeroes_and_dot(&mut normalized);
        }

        let exp_char = if case == Case::Lowercase { 'e' } else { 'E' };

        format!("{normalized}{additional_dot}{exp_char}{exponent:+03}")
    } else {
        // 类似十进制的表示法，有以下几点不同：
        // - 精度的工作方式不同，指定的是总位数，而不是小数部分的位数。
        // - 如果不强制使用小数点，小数部分的.和尾部的0会被修剪掉。
        let decimal_places = (precision as i32 - exponent) as usize;
        let mut formatted = match (decimal_places, force_decimal) {
            (0, ForceDecimal::Yes) => format!("{f:.0}."),
            _ => format!("{f:.decimal_places$}"),
        };

        if ForceDecimal::No == force_decimal {
            strip_fractional_zeroes_and_dot(&mut formatted);
        }

        formatted
    }
}

// 函数将浮点数格式化为十六进制字符串。
fn format_float_hexadecimal(
    f: f64,
    precision: Option<usize>,
    case: Case,
    force_decimal: ForceDecimal,
) -> String {
    let (first_digit, mantissa, exponent) = if f.abs() < f64::EPSILON {
        (0, 0, 0)
    } else {
        let bits = f.to_bits();
        let exponent_bits = ((bits >> 52) & 0x7fff) as i64;
        let exponent = exponent_bits - 1023;
        let mantissa = bits & 0xf_ffff_ffff_ffff;
        (1, mantissa, exponent)
    };

    const NATIVE_FRACTION_DIGITS: usize = 13;
    let (first_digit, mut fraction) = match precision {
        None => {
            let mut fraction = format!("{mantissa:0>NATIVE_FRACTION_DIGITS$x}");
            while fraction.ends_with('0') {
                fraction.pop();
            }
            (first_digit as u64, fraction)
        }
        Some(wanted) if wanted < NATIVE_FRACTION_DIGITS => {
            let significand = ((first_digit as u64) << 52) | mantissa;
            let shift = 4 * (NATIVE_FRACTION_DIGITS - wanted);
            let rounded = round_shift_right_ties_even(significand, shift);
            let digits = format!("{rounded:0>width$x}", width = wanted + 1);
            let (first, fraction) = digits.split_at(1);
            (
                u64::from_str_radix(first, 16).unwrap(),
                fraction.to_string(),
            )
        }
        Some(wanted) => {
            let mut fraction = format!("{mantissa:0>NATIVE_FRACTION_DIGITS$x}");
            fraction.push_str(&"0".repeat(wanted - NATIVE_FRACTION_DIGITS));
            (first_digit as u64, fraction)
        }
    };

    if let Some(wanted) = precision {
        fraction.truncate(wanted);
    }
    let point = if !fraction.is_empty() || force_decimal == ForceDecimal::Yes {
        "."
    } else {
        ""
    };
    let mut s = format!("0x{first_digit:x}{point}{fraction}p{exponent:+x}");

    if Case::Uppercase == case {
        s.make_ascii_uppercase();
    }
    s
}

fn round_shift_right_ties_even(value: u64, shift: usize) -> u64 {
    debug_assert!((1..64).contains(&shift));
    let truncated = value >> shift;
    let remainder = value & ((1_u64 << shift) - 1);
    let halfway = 1_u64 << (shift - 1);

    if remainder > halfway || (remainder == halfway && truncated & 1 == 1) {
        truncated + 1
    } else {
        truncated
    }
}

// 它将字符串的可变引用作为输入，并删除字符串尾部的零和小数点。
fn strip_fractional_zeroes_and_dot(s: &mut String) {
    let mut trim_to = s.len();

    for (pos, char_info) in s.char_indices().rev() {
        let pos_c = pos + char_info.len_utf8();
        if trim_to == pos_c && (char_info == '0' || char_info == '.') {
            trim_to = pos;
        }

        if char_info == '.' {
            s.truncate(trim_to);
            break;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ct_format::num_format::format_float_hexadecimal;
    use crate::ct_format::num_format::strip_fractional_zeroes_and_dot;
    use crate::ct_format::num_format::{Case, ForceDecimal};

    use std::io::Cursor;

    #[test]
    fn test_signed_int_try_from_spec_success() {
        let spec = Spec::SignedInt {
            width: Some(CanAsterisk::Fixed(10)),
            precision: Some(CanAsterisk::Fixed(4)),
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::Left,
            thousand_separate: false,
        };

        let signed_int = SignedInt::try_from_spec(spec).unwrap();

        assert_eq!(signed_int.width, 10);
        assert_eq!(signed_int.precision, Some(4));
        assert_eq!(signed_int.positive_sign, PositiveSign::Plus);
        assert_eq!(signed_int.alignment, NumberAlignment::Left);
    }

    #[test]
    fn test_signed_int_try_from_spec_zero_width() {
        // Spec with zero width
        let spec = Spec::SignedInt {
            width: Some(CanAsterisk::Fixed(0)),
            precision: Some(CanAsterisk::Fixed(4)),
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::RightSpace,
            thousand_separate: false,
        };

        let result = SignedInt::try_from_spec(spec);

        assert!(result.is_ok());
        let signed_int = result.unwrap();
        assert_eq!(signed_int.width, 0);
        assert_eq!(signed_int.precision, Some(4));
        assert_eq!(signed_int.positive_sign, PositiveSign::Plus);
        assert_eq!(signed_int.alignment, NumberAlignment::RightSpace);
    }

    #[test]
    fn test_signed_int_try_from_spec_max_width() {
        // Spec with maximum possible width
        let spec = Spec::SignedInt {
            width: Some(CanAsterisk::Fixed(i64::MAX as usize)),
            precision: Some(CanAsterisk::Fixed(4)),
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::Left,
            thousand_separate: false,
        };

        let result = SignedInt::try_from_spec(spec);

        assert!(result.is_ok());
        let signed_int = result.unwrap();
        assert_eq!(signed_int.width, i64::MAX as usize);
        assert_eq!(signed_int.precision, Some(4));
        assert_eq!(signed_int.positive_sign, PositiveSign::Plus);
        assert_eq!(signed_int.alignment, NumberAlignment::Left);
    }

    #[test]
    fn test_signed_int_try_from_spec_max_precision() {
        // Spec with maximum possible precision
        let spec = Spec::SignedInt {
            width: Some(CanAsterisk::Fixed(10)),
            precision: Some(CanAsterisk::Fixed(i64::MAX as usize)),
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::RightZero,
            thousand_separate: false,
        };

        let result = SignedInt::try_from_spec(spec);

        assert!(result.is_ok());
        let signed_int = result.unwrap();
        assert_eq!(signed_int.width, 10);
        assert_eq!(signed_int.precision, Some(i64::MAX as usize));
        assert_eq!(signed_int.positive_sign, PositiveSign::Plus);
        assert_eq!(signed_int.alignment, NumberAlignment::RightZero);
    }

    #[test]
    fn test_signed_int_fmt_positive_plus() {
        let formatter = SignedInt {
            width: 8,
            precision: None,
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::RightSpace,
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"   +1234");
    }

    #[test]
    fn test_signed_int_fmt_positive_space() {
        let formatter = SignedInt {
            width: 10,
            precision: None,
            positive_sign: PositiveSign::Space,
            alignment: NumberAlignment::Left,
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 5678).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b" 5678     ");
    }

    #[test]
    fn test_signed_int_fmt_negative() {
        let formatter = SignedInt {
            width: 12,
            precision: None,
            positive_sign: PositiveSign::None,
            alignment: NumberAlignment::RightZero,
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, -9876).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"-00000009876");
    }

    #[test]
    fn test_unsigned_int_fmt_decimal() {
        let formatter = UnsignedInt {
            variant: UnsignedIntVariant::Decimal,
            width: 10,
            precision: Some(2),
            alignment: NumberAlignment::Left,
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"1234      ");
    }

    #[test]
    fn test_unsigned_int_fmt_octal() {
        let formatter = UnsignedInt {
            variant: UnsignedIntVariant::Octal(Prefix::Yes),
            width: 8,
            precision: None,
            alignment: NumberAlignment::RightSpace,
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"   02322");
    }

    #[test]
    fn test_unsigned_int_fmt_hex_lowercase() {
        let formatter = UnsignedInt {
            variant: UnsignedIntVariant::Hexadecimal(Case::Lowercase, Prefix::Yes),
            width: 16,
            precision: None,
            alignment: NumberAlignment::Left,
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"0x4d2           ");
    }

    #[test]
    fn test_unsigned_int_fmt_hex_uppercase() {
        let formatter = UnsignedInt {
            variant: UnsignedIntVariant::Hexadecimal(Case::Uppercase, Prefix::Yes),
            width: 16,
            precision: None,
            alignment: NumberAlignment::RightZero,
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"0X000000000004D2");
    }

    #[test]
    fn test_unsigned_int_try_from_spec_decimal() {
        let spec = Spec::UnsignedInt {
            variant: UnsignedIntVariant::Decimal,
            width: Some(CanAsterisk::Fixed(10)),
            precision: Some(CanAsterisk::Fixed(2)),
            alignment: NumberAlignment::Left,
            thousand_separate: false,
        };

        let unsigned_int = UnsignedInt::try_from_spec(spec).unwrap();

        assert_eq!(unsigned_int.variant, UnsignedIntVariant::Decimal);
        assert_eq!(unsigned_int.width, 10);
        assert_eq!(unsigned_int.precision, Some(2));
        assert_eq!(unsigned_int.alignment, NumberAlignment::Left);
    }

    #[test]
    fn test_unsigned_int_try_from_spec_octal() {
        let spec = Spec::UnsignedInt {
            variant: UnsignedIntVariant::Octal(Prefix::Yes),
            width: Some(CanAsterisk::Fixed(8)),
            precision: None,
            alignment: NumberAlignment::RightSpace,
            thousand_separate: false,
        };

        let unsigned_int = UnsignedInt::try_from_spec(spec).unwrap();

        assert_eq!(unsigned_int.variant, UnsignedIntVariant::Octal(Prefix::Yes));
        assert_eq!(unsigned_int.width, 8);
        assert_eq!(unsigned_int.precision, None);
        assert_eq!(unsigned_int.alignment, NumberAlignment::RightSpace);
    }

    #[test]
    fn test_unsigned_int_try_from_spec_hexadecimal() {
        let spec = Spec::UnsignedInt {
            variant: UnsignedIntVariant::Hexadecimal(Case::Uppercase, Prefix::No),
            width: Some(CanAsterisk::Fixed(16)),
            precision: Some(CanAsterisk::Asterisk), // Precision as asterisk should fail
            alignment: NumberAlignment::Left,
            thousand_separate: false,
        };

        let result = UnsignedInt::try_from_spec(spec);

        assert!(result.is_err());
        // assert_eq!(result.err().unwrap(), FormatError::WrongSpecType);
    }

    #[test]
    fn test_float_fmt_decimal() {
        let formatter = Float {
            variant: FloatVariant::Decimal,
            case: Case::Lowercase,
            force_decimal: ForceDecimal::No,
            width: 10,
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::Left,
            precision: Some(2),
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234.567).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"+1234.57   ");
    }
    #[test]
    fn test_float_fmt_decimal2() {
        let formatter = Float {
            variant: FloatVariant::Decimal,
            case: Case::Lowercase,
            force_decimal: ForceDecimal::No,
            width: 5,
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::Left,
            precision: Some(1),
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234.567).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"+1234.6");
    }

    #[test]
    fn test_float_fmt_decimal3() {
        let formatter = Float {
            variant: FloatVariant::Decimal,
            case: Case::Lowercase,
            force_decimal: ForceDecimal::No,
            width: 5,
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::RightSpace,
            precision: Some(1),
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234.567).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"+1234.6");
    }
    #[test]
    fn test_float_fmt_decimal4() {
        let formatter = Float {
            variant: FloatVariant::Decimal,
            case: Case::Lowercase,
            force_decimal: ForceDecimal::Yes,
            width: 10,
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::RightSpace,
            precision: Some(1),
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234.567).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"+    1234.6");
    }
    #[test]
    fn test_float_fmt_scientific() {
        let formatter = Float {
            variant: FloatVariant::Scientific,
            case: Case::Lowercase,
            force_decimal: ForceDecimal::No,
            width: 10,
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::RightSpace,
            precision: Some(3),
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234.567).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"+ 1.235e+03");
    }

    #[test]
    fn test_float_fmt_shortest() {
        let formatter = Float {
            variant: FloatVariant::Shortest,
            case: Case::Uppercase,
            force_decimal: ForceDecimal::Yes,
            width: 15,
            positive_sign: PositiveSign::Space,
            alignment: NumberAlignment::RightZero,
            precision: Some(4),
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234.567).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b" 00000000001235.");
    }
    // use std::io::Cursor;
    #[test]
    fn test_float_fmt_hexadecimal() {
        let formatter = Float {
            variant: FloatVariant::Hexadecimal,
            case: Case::Uppercase,
            force_decimal: ForceDecimal::No,
            width: 20,
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::Left,
            precision: Some(0),
        };

        let mut buffer = Cursor::new(Vec::new());
        formatter.fmt(&mut buffer, 1234.567).unwrap();
        buffer.set_position(0);
        let result = buffer.get_ref();
        assert_eq!(result, b"+0X1P+A              ");
    }

    #[test]
    fn test_float_try_from_spec_success() {
        let spec = Spec::Float {
            variant: FloatVariant::Decimal,
            case: Case::Lowercase,
            force_decimal: ForceDecimal::No,
            width: Some(CanAsterisk::Fixed(10)),
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::Left,
            precision: Some(CanAsterisk::Fixed(6)),
            thousand_separate: false,
        };

        let float = Float::try_from_spec(spec).unwrap();

        assert_eq!(float.variant, FloatVariant::Decimal);
        assert_eq!(float.case, Case::Lowercase);
        assert_eq!(float.force_decimal, ForceDecimal::No);
        assert_eq!(float.width, 10);
        assert_eq!(float.positive_sign, PositiveSign::Plus);
        assert_eq!(float.alignment, NumberAlignment::Left);
        assert_eq!(float.precision, Some(6));
    }

    #[test]
    fn test_float_try_from_spec_default_precision_for_shortest() {
        let spec = Spec::Float {
            variant: FloatVariant::Shortest,
            case: Case::Lowercase,
            force_decimal: ForceDecimal::No,
            width: Some(CanAsterisk::Fixed(10)),
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::Left,
            precision: None, // Precision not provided
            thousand_separate: false,
        };

        let float = Float::try_from_spec(spec).unwrap();

        assert_eq!(float.variant, FloatVariant::Shortest);
        assert_eq!(float.case, Case::Lowercase);
        assert_eq!(float.force_decimal, ForceDecimal::No);
        assert_eq!(float.width, 10);
        assert_eq!(float.positive_sign, PositiveSign::Plus);
        assert_eq!(float.alignment, NumberAlignment::Left);
        assert_eq!(float.precision, None); // Formatting supplies the default precision.
    }

    #[test]
    fn test_float_try_from_spec_zero_width_and_precision() {
        let spec = Spec::Float {
            variant: FloatVariant::Decimal,
            case: Case::Lowercase,
            force_decimal: ForceDecimal::No,
            width: Some(CanAsterisk::Fixed(0)), // Zero width
            positive_sign: PositiveSign::Plus,
            alignment: NumberAlignment::Left,
            precision: Some(CanAsterisk::Fixed(0)), // Zero precision
            thousand_separate: false,
        };

        let float = Float::try_from_spec(spec).unwrap();

        assert_eq!(float.variant, FloatVariant::Decimal);
        assert_eq!(float.case, Case::Lowercase);
        assert_eq!(float.force_decimal, ForceDecimal::No);
        assert_eq!(float.width, 0);
        assert_eq!(float.positive_sign, PositiveSign::Plus);
        assert_eq!(float.alignment, NumberAlignment::Left);
        assert_eq!(float.precision, Some(0));
    }

    #[test]
    fn test_format_float_non_finite_inf_input() {
        let f = f64::INFINITY;
        let case = Case::Lowercase;

        let expected = "inf";
        let actual = format_float_non_finite(f, case);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_non_finite_nan_input() {
        let f = f64::NAN;
        let case = Case::Lowercase;

        let expected = "NaN";
        let actual = format_float_non_finite(f, case);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_non_finite_case_uppercase_input() {
        let f = f64::INFINITY;
        let case = Case::Uppercase;

        let expected = "INF";
        let actual = format_float_non_finite(f, case);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_non_finite_case_lowercase_input() {
        let f = f64::INFINITY;
        let case = Case::Lowercase;

        let expected = "inf";
        let actual = format_float_non_finite(f, case);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_decimal_normal_functionality() {
        let f = 123456.789;
        let precision = 6;
        let force_decimal = ForceDecimal::No;

        let expected = "123456.789000";
        let actual = format_float_decimal(f, precision, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_decimal_normal_functionality2() {
        let f = 123456.789;
        let precision = 5;
        let force_decimal = ForceDecimal::No;

        let expected = "123456.78900";
        let actual = format_float_decimal(f, precision, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_decimal_precision_zero() {
        let f = 123456.789;
        let precision = 0;
        let force_decimal = ForceDecimal::No;

        let expected = "123457";
        let actual = format_float_decimal(f, precision, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_decimal_precision_nonzero() {
        let f = 123456.789;
        let precision = 6;
        let force_decimal = ForceDecimal::No;

        let expected = "123456.789000";
        let actual = format_float_decimal(f, precision, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_decimal_force_decimal_zero_precision() {
        let f = 123456.789;
        let precision = 0;
        let force_decimal = ForceDecimal::Yes;

        let expected = "123457.";
        let actual = format_float_decimal(f, precision, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_decimal_inf_input() {
        let f = f64::INFINITY;
        let precision = 6;
        let force_decimal = ForceDecimal::No;

        let expected = "inf";
        let actual = format_float_decimal(f, precision, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_decimal_nan_input() {
        let f = f64::NAN;
        let precision = 6;
        let force_decimal = ForceDecimal::No;

        let expected = "NaN";
        let actual = format_float_decimal(f, precision, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_scientific_normal_functionality() {
        let f = 123456.789;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "1.234568e+05";
        let actual = format_float_scientific(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_scientific_normal_functionality2() {
        let f = 123456.789;
        let precision = 5;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "1.23457e+05";
        let actual = format_float_scientific(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_scientific_precision_zero() {
        let f = 123456.789;
        let precision = 0;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "1e+05";
        let actual = format_float_scientific(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_scientific_precision_nonzero() {
        let f = 123456.789;
        let precision = 6;
        let case = Case::Uppercase;
        let force_decimal = ForceDecimal::No;

        let expected = "1.234568E+05";
        let actual = format_float_scientific(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_scientific_force_decimal_zero_precision() {
        let f = 123456.789;
        let precision = 0;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::Yes;

        let expected = "1.e+05";
        let actual = format_float_scientific(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_scientific_inf_input() {
        let f = f64::INFINITY;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "NaNe+2147483647";
        let actual = format_float_scientific(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_scientific_inf_input2() {
        let f = f64::INFINITY;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::Yes;

        let expected = "NaNe+2147483647";
        let actual = format_float_scientific(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_scientific_nan_input() {
        let f = f64::NAN;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "NaNe+00";
        let actual = format_float_scientific(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_shortest_normal_functionality() {
        let f = 123456.789;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::Yes;

        let expected = "123457.";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_shortest_normal_functionality2() {
        let f = 123456.789;
        let precision = 5;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::Yes;

        let expected = "1.2346e+05";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_shortest_precision_zero() {
        let f = 123456.789;
        let precision = 0;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "1e+05";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_shortest_precision_nonzero() {
        let f = 123456.789;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "123457";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_shortest_precision_nonzero2() {
        let f = 123456.789;
        let precision = 7;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "123456.8";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_shortest_force_decimal_zero_precision() {
        let f = 123456.789;
        let precision = 0;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::Yes;

        let expected = "1.e+05";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_shortest_force_decimal_nonzero_precision() {
        let f = 123456.789;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::Yes;

        let expected = "123457.";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_shortest_invalid_input() {
        let f = f64::INFINITY;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "NaNe+2147483647";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_shortest_inf_input() {
        let f = f64::INFINITY;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "NaNe+2147483647";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_shortest_nan_input() {
        let f = f64::NAN;
        let precision = 6;
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "NaN";
        let actual = format_float_shortest(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_hexadecimal() {
        let f = 123456.789;
        let precision = Some(6);
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "0x1.e240cap+10";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_hexadecimal_zero() {
        let f = 0.0;
        let precision = Some(6);
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "0x0.000000p+0";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_hexadecimal_positive_number() {
        let f = 123456.789;
        let precision = Some(6);
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "0x1.e240cap+10";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_hexadecimal_positive_number2() {
        let f = 123456.789;
        let precision = Some(6);
        let case = Case::Uppercase;
        let force_decimal = ForceDecimal::No;

        let expected = "0X1.E240CAP+10";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_hexadecimal_negative_number() {
        let f = -123456.789;
        let precision = Some(6);
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "0x1.e240cap+810";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_hexadecimal_zero_precision() {
        let f = 123456.789;
        let precision = Some(0);
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "0x2p+10";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_hexadecimal_zero_precision_forced() {
        let f = 123456.789;
        let precision = Some(0);
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::Yes;

        let expected = "0x2.p+10";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }
    #[test]
    fn test_format_float_hexadecimal_force_decimal_zero_precision() {
        let f = 123456.789;
        let precision = Some(0);
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::Yes;

        let expected = "0x2.p+10";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_hexadecimal_invalid_input() {
        let f: f64 = 0.0;
        let precision = Some(6);
        let case = Case::Uppercase;
        let force_decimal = ForceDecimal::No;

        let expected = "0X0.000000P+0";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_hexadecimal_zero_input() {
        let f: f64 = 0.0;
        let precision = Some(6);
        let case = Case::Uppercase;
        let force_decimal = ForceDecimal::Yes;

        let expected = "0X0.000000P+0";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_float_hexadecimal_inf_input() {
        let f = f64::INFINITY;
        let precision = Some(6);
        let case = Case::Lowercase;
        let force_decimal = ForceDecimal::No;

        let expected = "0x1.000000p+400";
        let actual = format_float_hexadecimal(f, precision, case, force_decimal);

        assert_eq!(actual, expected);
    }

    #[test]
    fn hexadecimal_float_trims_default_precision_and_rounds_explicit_precision() {
        assert_eq!(
            format_float_hexadecimal(1.5, None, Case::Lowercase, ForceDecimal::No),
            "0x1.8p+0"
        );
        assert_eq!(
            format_float_hexadecimal(1.5, Some(0), Case::Lowercase, ForceDecimal::No),
            "0x2p+0"
        );
        assert_eq!(
            format_float_hexadecimal(1.96875, Some(1), Case::Lowercase, ForceDecimal::No),
            "0x2.0p+0"
        );
        assert_eq!(
            format_float_hexadecimal(1.5, Some(0), Case::Lowercase, ForceDecimal::Yes),
            "0x2.p+0"
        );
    }

    #[test]
    fn test_strip_fractional_zeroes_and_dot_normal_functionality() {
        let mut s = String::from("1000.00000");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "1000");

        let mut s = String::from("1000.");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "1000");

        let mut s = String::from("1000.02030");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "1000.0203");

        let mut s = String::from("1000.00000");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "1000");
    }

    #[test]
    fn test_strip_fractional_zeroes_and_dot_empty_string() {
        let mut s = String::new();
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "");
    }

    #[test]
    fn test_strip_fractional_zeroes_and_dot_no_decimal_point() {
        let mut s = String::from("1000");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "1000");
    }

    #[test]
    fn test_strip_fractional_zeroes_and_dot_leading_zeros() {
        let mut s = String::from("0000.00000");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "0000");

        let mut s = String::from("0000.");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "0000");
    }

    #[test]
    fn test_strip_fractional_zeroes_and_dot_trailing_zeros_and_decimal_point() {
        let mut s = String::from("1000.000000");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "1000");

        let mut s = String::from("1000.");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "1000");
    }

    #[test]
    fn test_strip_fractional_zeroes_and_dot_no_trailing_zeros_and_decimal_point() {
        let mut s = String::from("1000");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "1000");

        let mut s = String::from("1000.");
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "1000");
    }

    #[test]
    fn test_strip_fractional_zeroes_and_dot_no_input() {
        let mut s = String::new();
        strip_fractional_zeroes_and_dot(&mut s);
        assert_eq!(s, "");
        // }
    }
    //
    #[test]
    fn unsigned_octal() {
        use super::{Formatter, NumberAlignment, Prefix, UnsignedInt, UnsignedIntVariant};
        let f = |x| {
            let mut s = Vec::new();
            UnsignedInt {
                variant: UnsignedIntVariant::Octal(Prefix::Yes),
                width: 0,
                precision: None,
                alignment: NumberAlignment::Left,
            }
            .fmt(&mut s, x)
            .unwrap();
            String::from_utf8(s).unwrap()
        };

        assert_eq!(f(0), "0");
        assert_eq!(f(5), "05");
        assert_eq!(f(8), "010");
    }

    #[test]
    fn decimal_float() {
        use super::format_float_decimal;
        let f = |x| format_float_decimal(x, 6, ForceDecimal::No);
        assert_eq!(f(0.0), "0.000000");
        assert_eq!(f(1.0), "1.000000");
        assert_eq!(f(100.0), "100.000000");
        assert_eq!(f(123456.789), "123456.789000");
        assert_eq!(f(12.3456789), "12.345679");
        assert_eq!(f(1000000.0), "1000000.000000");
        assert_eq!(f(99999999.0), "99999999.000000");
        assert_eq!(f(1.9999995), "1.999999");
        assert_eq!(f(1.9999996), "2.000000");
    }

    #[test]
    fn scientific_float() {
        use super::format_float_scientific;
        let f = |x| format_float_scientific(x, 6, Case::Lowercase, ForceDecimal::No);
        assert_eq!(f(0.0), "0.000000e+00");
        assert_eq!(f(1.0), "1.000000e+00");
        assert_eq!(f(100.0), "1.000000e+02");
        assert_eq!(f(123456.789), "1.234568e+05");
        assert_eq!(f(12.3456789), "1.234568e+01");
        assert_eq!(f(1000000.0), "1.000000e+06");
        assert_eq!(f(99999999.0), "1.000000e+08");
    }

    #[test]
    fn scientific_float_normalizes_negative_values() {
        use super::format_float_scientific;

        assert_eq!(
            format_float_scientific(-123.0, 6, Case::Lowercase, ForceDecimal::No),
            "-1.230000e+02"
        );
        assert_eq!(
            format_float_scientific(-9.999_999_6, 6, Case::Lowercase, ForceDecimal::No),
            "-1.000000e+01"
        );
    }

    #[test]
    fn scientific_float_zero_precision() {
        use super::format_float_scientific;

        let f = |x| format_float_scientific(x, 0, Case::Lowercase, ForceDecimal::No);
        assert_eq!(f(0.0), "0e+00");
        assert_eq!(f(1.0), "1e+00");
        assert_eq!(f(100.0), "1e+02");
        assert_eq!(f(123456.789), "1e+05");
        assert_eq!(f(12.3456789), "1e+01");
        assert_eq!(f(1000000.0), "1e+06");
        assert_eq!(f(99999999.0), "1e+08");

        let f = |x| format_float_scientific(x, 0, Case::Lowercase, ForceDecimal::Yes);
        assert_eq!(f(0.0), "0.e+00");
        assert_eq!(f(1.0), "1.e+00");
        assert_eq!(f(100.0), "1.e+02");
        assert_eq!(f(123456.789), "1.e+05");
        assert_eq!(f(12.3456789), "1.e+01");
        assert_eq!(f(1000000.0), "1.e+06");
        assert_eq!(f(99999999.0), "1.e+08");
    }

    #[test]
    fn shortest_float() {
        use super::format_float_shortest;
        let f = |x| format_float_shortest(x, 6, Case::Lowercase, ForceDecimal::No);
        assert_eq!(f(0.0), "0");
        assert_eq!(f(1.0), "1");
        assert_eq!(f(100.0), "100");
        assert_eq!(f(123456.789), "123457");
        assert_eq!(f(12.3456789), "12.3457");
        assert_eq!(f(1000000.0), "1e+06");
        assert_eq!(f(99999999.0), "1e+08");
    }

    #[test]
    fn shortest_float_uses_rounded_exponent_at_notation_boundaries() {
        use super::format_float_shortest;
        let f = |x| format_float_shortest(x, 6, Case::Lowercase, ForceDecimal::No);

        assert_eq!(f(0.0001), "0.0001");
        assert_eq!(f(0.00001), "1e-05");
        assert_eq!(f(999999.0), "999999");
        assert_eq!(f(999999.5), "1e+06");
        assert_eq!(f(1000000.0), "1e+06");
    }

    #[test]
    fn shortest_float_force_decimal() {
        use super::format_float_shortest;
        let f = |x| format_float_shortest(x, 6, Case::Lowercase, ForceDecimal::Yes);
        assert_eq!(f(0.0), "0.00000");
        assert_eq!(f(1.0), "1.00000");
        assert_eq!(f(100.0), "100.000");
        assert_eq!(f(123456.789), "123457.");
        assert_eq!(f(12.3456789), "12.3457");
        assert_eq!(f(1000000.0), "1.00000e+06");
        assert_eq!(f(99999999.0), "1.00000e+08");
    }

    #[test]
    fn shortest_float_force_decimal_zero_precision() {
        use super::format_float_shortest;
        let f = |x| format_float_shortest(x, 0, Case::Lowercase, ForceDecimal::No);
        assert_eq!(f(0.0), "0");
        assert_eq!(f(1.0), "1");
        assert_eq!(f(100.0), "1e+02");
        assert_eq!(f(123456.789), "1e+05");
        assert_eq!(f(12.3456789), "1e+01");
        assert_eq!(f(1000000.0), "1e+06");
        assert_eq!(f(99999999.0), "1e+08");

        let f = |x| format_float_shortest(x, 0, Case::Lowercase, ForceDecimal::Yes);
        assert_eq!(f(0.0), "0.");
        assert_eq!(f(1.0), "1.");
        assert_eq!(f(100.0), "1.e+02");
        assert_eq!(f(123456.789), "1.e+05");
        assert_eq!(f(12.3456789), "1.e+01");
        assert_eq!(f(1000000.0), "1.e+06");
        assert_eq!(f(99999999.0), "1.e+08");
    }

    #[test]
    fn strip_insignificant_end() {
        use super::strip_fractional_zeroes_and_dot;
        let f = |s| {
            let mut s = String::from(s);
            strip_fractional_zeroes_and_dot(&mut s);
            s
        };
        assert_eq!(&f("1000"), "1000");
        assert_eq!(&f("1000."), "1000");
        assert_eq!(&f("1000.02030"), "1000.0203");
        assert_eq!(&f("1000.00000"), "1000");
    }
}
