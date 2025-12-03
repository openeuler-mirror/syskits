/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! Utilities for formatting numbers in various formats

use std::io::Write;

use super::{
    spec::{CanAsterisk, Spec},
    FormatError,
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
    pub precision: usize,
    pub positive_sign: PositiveSign,
    pub alignment: NumberAlignment,
}

impl Formatter for SignedInt {
    type Input = i64;

    // 该函数负责根据结构体中提供的规范格式化无符号整数值。
    fn fmt(&self, mut writer: impl Write, x: Self::Input) -> std::io::Result<()> {
        if x >= 0 {
            match self.positive_sign {
                PositiveSign::None => Ok(()),
                PositiveSign::Plus => write!(writer, "+"),
                PositiveSign::Space => write!(writer, " "),
            }?;
        }

        let s = format!("{:0width$}", x, width = self.precision);

        if self.alignment == NumberAlignment::Left {
            write!(writer, "{s:<width$}", width = self.width)
        } else if self.alignment == NumberAlignment::RightSpace {
            write!(writer, "{s:>width$}", width = self.width)
        } else {
            // self.alignment == NumberAlignment::RightZero
            write!(writer, "{s:0>width$}", width = self.width)
        }
    }

    // 它定义了一个名为 try_from_spec 的方法，用于尝试从 Spec 实例创建一个新的 SignedInt 实例。
    // Spec 结构是各种格式化规范的通用容器。
    fn try_from_spec(s: Spec) -> Result<Self, FormatError> {
        if let Spec::SignedInt {
            width,
            precision,
            positive_sign,
            alignment,
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
                x
            } else if precision.is_none() {
                0
            } else {
                return Err(FormatError::WrongSpecType);
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
    pub precision: usize,
    pub alignment: NumberAlignment,
    pub variant: UnsignedIntVariant,
}

impl Formatter for UnsignedInt {
    type Input = u64;

    // 该函数负责根据提供的规范格式化无符号整数值。
    fn fmt(&self, mut writer: impl Write, x: Self::Input) -> std::io::Result<()> {
        let mut s = String::new();

        if let UnsignedIntVariant::Decimal = self.variant {
            s = format!("{x}");
        } else if let UnsignedIntVariant::Octal(_) = self.variant {
            s = format!("{x:o}");
        } else if let UnsignedIntVariant::Hexadecimal(Case::Lowercase, _) = self.variant {
            s = format!("{x:x}");
        } else if let UnsignedIntVariant::Hexadecimal(Case::Uppercase, _) = self.variant {
            s = format!("{x:X}");
        }

        // Zeroes do not get a prefix. An octal value does also not get a
        // prefix if the padded value will not start with a zero.
        let prefix = match (x, self.variant) {
            (1.., UnsignedIntVariant::Hexadecimal(Case::Lowercase, Prefix::Yes)) => "0x",
            (1.., UnsignedIntVariant::Hexadecimal(Case::Uppercase, Prefix::Yes)) => "0X",
            (1.., UnsignedIntVariant::Octal(Prefix::Yes)) if s.len() >= self.precision => "0",
            _ => "",
        };

        s = format!("{prefix}{s:0>width$}", width = self.precision);

        if self.alignment == NumberAlignment::Left {
            write!(writer, "{s:<width$}", width = self.width)
        } else if self.alignment == NumberAlignment::RightSpace {
            write!(writer, "{s:>width$}", width = self.width)
        } else {
            // self.alignment == NumberAlignment::RightZero
            write!(writer, "{s:0>width$}", width = self.width)
        }
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
        } = s
        {
            Spec::UnsignedInt {
                variant: UnsignedIntVariant::Decimal,
                width,
                precision,
                alignment,
            }
        } else {
            s
        };

        if let Spec::UnsignedInt {
            variant,
            width,
            precision,
            alignment,
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
                x
            } else if precision.is_none() {
                0
            } else {
                return Err(FormatError::WrongSpecType);
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

pub struct Float {
    pub variant: FloatVariant,
    pub case: Case,
    pub force_decimal: ForceDecimal,
    pub width: usize,
    pub positive_sign: PositiveSign,
    pub alignment: NumberAlignment,
    pub precision: usize,
}

impl Default for Float {
    fn default() -> Self {
        Self {
            width: 0,
            precision: 6,
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
                    format_float_decimal(x, self.precision, self.force_decimal)
                } else if self.variant == FloatVariant::Scientific {
                    format_float_scientific(x, self.precision, self.case, self.force_decimal)
                } else if self.variant == FloatVariant::Shortest {
                    format_float_shortest(x, self.precision, self.case, self.force_decimal)
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
                x
            } else if precision.is_none() {
                if matches!(variant, FloatVariant::Shortest) {
                    6
                } else {
                    0
                }
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
        _ => format!("{f:.*}", precision),
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

    let mut exponent: i32 = f.log10().floor() as i32;
    let mut normalized = f / 10.0_f64.powi(exponent);

    // If the normalized value will be rounded to a value greater than 10
    // we need to correct.
    let tmp_value = normalized * 10_f64.powi(precision as i32);
    let value = tmp_value.round() / 10_f64.powi(precision as i32);
    if value >= 10.0 {
        normalized /= 10.0;
        exponent += 1;
    }

    let additional_dot = match (precision, force_decimal) {
        (0, ForceDecimal::Yes) => ".",
        _ => "",
    };

    let exp_char = if Case::Lowercase == case { 'e' } else { 'E' };

    format!(
        "{normalized:.*}{additional_dot}{exp_char}{exponent:+03}",
        precision
    )
}

// 该函数负责将浮点数格式化为最短格式。这种格式是十进制和科学记数法的混合体，
// 但也有一些区别。
fn format_float_shortest(
    f: f64,
    precision: usize,
    case: Case,
    force_decimal: ForceDecimal,
) -> String {
    // Precision here is about how many digits should be displayed
    // instead of how many digits for the fractional part, this means that if
    // we pass this to rust's format string, it's always gonna be one less.
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

    let mut exponent = f.log10().floor() as i32;
    if f.abs() > f64::EPSILON && exponent <= -4 || exponent > precision as i32 {
        // Scientific-ish notation (with a few differences)
        let mut normalized = f / 10.0_f64.powi(exponent);

        // If the normalized value will be rounded to a value greater than 10
        // we need to correct.
        let tmp_value = normalized * 10_f64.powi(precision as i32);
        let value = tmp_value.round() / 10_f64.powi(precision as i32);
        if value >= 10.0 {
            normalized /= 10.0;
            exponent += 1;
        }

        let additional_dot = match (precision, force_decimal) {
            (0, ForceDecimal::Yes) => ".",
            _ => "",
        };

        let mut normalized = format!("{normalized:.*}", precision);

        if ForceDecimal::No == force_decimal {
            strip_fractional_zeroes_and_dot(&mut normalized);
        }

        let exp_char = if case == Case::Lowercase { 'e' } else { 'E' };

        format!("{normalized}{additional_dot}{exp_char}{exponent:+03}")
    } else {
        // Decimal-ish notation with a few differences:
        //  - The precision works differently and specifies the total number
        //    of digits instead of the digits in the fractional part.
        //  - If we don't force the decimal, `.` and trailing `0` in the fractional part
        //    are trimmed.
        let decimal_places = (precision as i32 - exponent) as usize;
        let mut formatted = match (decimal_places, force_decimal) {
            (0, ForceDecimal::Yes) => format!("{f:.0}."),
            _ => format!("{f:.*}", decimal_places),
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
    precision: usize,
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

    let mut s = if precision == 0 {
        if force_decimal == ForceDecimal::No {
            format!("0x{first_digit}p{exponent:+x}")
        } else {
            format!("0x{first_digit}.p{exponent:+x}")
        }
    } else {
        format!("0x{first_digit}.{mantissa:0>13x}p{exponent:+x}")
    };

    if Case::Uppercase == case {
        s.make_ascii_uppercase();
    }
    s
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

