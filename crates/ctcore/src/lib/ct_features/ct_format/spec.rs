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

use super::{
    FormatChar, FormatError,
    argument::ArgCursor,
    num_format::{
        self, Case, FloatVariant, ForceDecimal, Formatter, NumberAlignment, PositiveSign, Prefix,
        UnsignedIntVariant,
    },
    parse_escape_only,
};
use crate::ct_quoting_style::{CtQuotingStyle, escape_name};
use std::ffi::OsStr;
use std::{io::Write, ops::ControlFlow};

/// 用于格式化值的已解析说明符
/// 可能需要多个参数来解析以*给出的宽度或精度值
#[derive(Debug, PartialEq)]
pub enum Spec {
    Char {
        width: Option<CanAsterisk<usize>>,
        align_left: bool,
    },
    String {
        precision: Option<CanAsterisk<usize>>,
        width: Option<CanAsterisk<usize>>,
        align_left: bool,
    },
    EscapedString,
    QuotedString,
    SignedInt {
        width: Option<CanAsterisk<usize>>,
        precision: Option<CanAsterisk<usize>>,
        positive_sign: PositiveSign,
        alignment: NumberAlignment,
    },
    UnsignedInt {
        variant: UnsignedIntVariant,
        width: Option<CanAsterisk<usize>>,
        precision: Option<CanAsterisk<usize>>,
        alignment: NumberAlignment,
    },
    Float {
        variant: FloatVariant,
        case: Case,
        force_decimal: ForceDecimal,
        width: Option<CanAsterisk<usize>>,
        positive_sign: PositiveSign,
        alignment: NumberAlignment,
        precision: Option<CanAsterisk<usize>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanAsterisk<T> {
    Fixed(T),
    Asterisk,
}

#[derive(Debug, PartialEq)]
enum Length {
    Char,
    Short,
    Long,
    LongLong,
    IntMaxT,
    SizeT,
    PtfDiffT,
    LongDouble,
}

#[derive(Default, PartialEq, Eq)]
struct Flags {
    minus: bool,
    plus: bool,
    space: bool,
    hash: bool,
    zero: bool,
    quote: bool,
}

impl Flags {
    pub fn parse(rest: &mut &[u8], index: &mut usize) -> Self {
        let mut flags = Self::default();
        while let Some(x) = rest.get(*index) {
            match x {
                b'-' => flags.minus = true,
                b'+' => flags.plus = true,
                b' ' => flags.space = true,
                b'#' => flags.hash = true,
                b'0' => flags.zero = true,
                b'\'' => flags.quote = true,
                _ => break,
            }
            *index += 1;
        }
        flags
    }

    fn any(&self) -> bool {
        self != &Self::default()
    }
}

/// 用于包裹 Spec，注入索引支持，防止破坏底层代码
#[derive(Debug, PartialEq)]
pub struct IndexedSpec {
    pub arg_index: Option<usize>,
    pub width_index: Option<usize>,
    pub precision_index: Option<usize>,
    pub spec: Spec,
}

impl IndexedSpec {
    pub fn parse<'a>(rest: &mut &'a [u8]) -> Result<Self, &'a [u8]> {
        let mut index = 0;
        let start = *rest;

        // 尝试解析参数索引 (如 2$)
        let mut arg_index = None;
        if let Some((num, len)) = peek_number(rest, index) {
            if rest.get(index + len) == Some(&b'$') {
                arg_index = Some(num);
                index += len + 1;
            }
        }

        let flags = Flags::parse(rest, &mut index);

        let positive_sign = if flags.plus {
            PositiveSign::Plus
        } else if flags.space {
            PositiveSign::Space
        } else {
            PositiveSign::None
        };

        // 尝试解析宽度和它的索引 (如 *2$)
        let (width, width_index) = match eat_asterisk_or_number(rest, &mut index) {
            Some((w, idx)) => (Some(w), idx),
            None => (None, None),
        };

        // 尝试解析精度和它的索引
        let (precision, precision_index) = if let Some(b'.') = rest.get(index) {
            index += 1;
            match eat_asterisk_or_number(rest, &mut index) {
                Some((p, idx)) => (Some(p), idx),
                None => (Some(CanAsterisk::Fixed(0)), None),
            }
        } else {
            (None, None)
        };

        let alignment = if flags.minus {
            NumberAlignment::Left
        } else if precision.is_none() && flags.zero {
            NumberAlignment::RightZero
        } else {
            NumberAlignment::RightSpace
        };

        let mut temp_idx = index;
        let _ = Spec::parse_length(rest, &mut temp_idx);
        index = temp_idx;

        let type_spec = match rest.get(index) {
            Some(type_spec) => type_spec,
            None => return Err(&start[..index]),
        };

        index += 1;
        *rest = &start[index..];

        let spec = match type_spec {
            b'c' => {
                // 对于字符类型，单引号标志是非法的
                if flags.hash || flags.zero || flags.quote || precision.is_some() {
                    return Err(&start[..index]);
                }
                Spec::Char {
                    width,
                    align_left: flags.minus,
                }
            }
            b's' => {
                // 对于字符串类型，单引号标志是非法的
                if flags.hash || flags.zero || flags.quote {
                    return Err(&start[..index]);
                }
                Spec::String {
                    precision,
                    width,
                    align_left: flags.minus,
                }
            }
            b'b' => {
                if flags.any() || width.is_some() || precision.is_some() {
                    return Err(&start[..index]);
                }
                Spec::EscapedString
            }
            b'q' => {
                if flags.any() || width.is_some() || precision.is_some() {
                    return Err(&start[..index]);
                }
                Spec::QuotedString
            }
            b'd' | b'i' => {
                if flags.hash {
                    return Err(&start[..index]);
                }
                Spec::SignedInt {
                    width,
                    alignment,
                    precision,
                    positive_sign,
                }
            }
            c @ (b'o' | b'u' | b'x' | b'X') => {
                if flags.hash && *c == b'u' {
                    return Err(&start[..index]);
                }
                // 千分位分组不支持八进制和十六进制 (o, x, X)
                if flags.quote && *c != b'u' {
                    return Err(&start[..index]);
                }
                let prefix = if flags.hash { Prefix::Yes } else { Prefix::No };
                Spec::UnsignedInt {
                    variant: match c {
                        b'o' => UnsignedIntVariant::Octal(prefix),
                        b'u' => UnsignedIntVariant::Decimal,
                        b'x' => UnsignedIntVariant::Hexadecimal(Case::Lowercase, prefix),
                        b'X' => UnsignedIntVariant::Hexadecimal(Case::Uppercase, prefix),
                        _ => unreachable!(),
                    },
                    precision,
                    width,
                    alignment,
                }
            }
            c @ (b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G') => {
                let force_decimal = if flags.hash {
                    ForceDecimal::Yes
                } else {
                    ForceDecimal::No
                };
                let case = if c.is_ascii_uppercase() {
                    Case::Uppercase
                } else {
                    Case::Lowercase
                };
                let variant = match c {
                    b'a' | b'A' => FloatVariant::Hexadecimal,
                    b'e' | b'E' => FloatVariant::Scientific,
                    b'f' | b'F' => FloatVariant::Decimal,
                    b'g' | b'G' => FloatVariant::Shortest,
                    _ => unreachable!(),
                };
                // 注：对于浮点数等支持千分位的，同样的降级处理。
                Spec::Float {
                    width,
                    precision,
                    variant,
                    force_decimal,
                    case,
                    alignment,
                    positive_sign,
                }
            }
            _ => return Err(&start[..index]),
        };

        Ok(Self {
            arg_index,
            width_index,
            precision_index,
            spec,
        })
    }

    pub fn write<'a>(
        &self,
        mut writer: impl Write,
        cursor: &mut ArgCursor<'a>,
    ) -> Result<(), FormatError> {
        match &self.spec {
            Spec::Char { width, align_left } => {
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor);
                write_padded(
                    writer,
                    &[cursor.get_char(self.arg_index)],
                    w.unwrap_or(0),
                    *align_left || dyn_left,
                )
            }
            Spec::String {
                width,
                align_left,
                precision,
            } => {
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor);
                let p = resolve_precision(*precision, self.precision_index, cursor);
                let s = cursor.get_str(self.arg_index);
                let truncated = match p {
                    Some(prec) if prec < s.len() => &s[..prec],
                    _ => s,
                };
                write_padded(
                    writer,
                    truncated.as_bytes(),
                    w.unwrap_or(0),
                    *align_left || dyn_left,
                )
            }
            Spec::EscapedString => {
                let s = cursor.get_str(self.arg_index);
                let mut parsed = Vec::new();
                for res in parse_escape_only(s.as_bytes()) {
                    match res?.write(&mut parsed)? {
                        ControlFlow::Continue(()) => {}
                        ControlFlow::Break(()) => break,
                    };
                }
                writer.write_all(&parsed).map_err(FormatError::IoError)
            }
            Spec::QuotedString => {
                let s = cursor.get_str(self.arg_index);
                if s.is_empty() {
                    writer.write_all(b"''").map_err(FormatError::IoError)
                } else {
                    writer
                        .write_all(
                            escape_name(
                                OsStr::new(s),
                                &CtQuotingStyle::Shell {
                                    escape: true,
                                    always_quote: false,
                                    show_control: true,
                                },
                            )
                            .as_bytes(),
                        )
                        .map_err(FormatError::IoError)
                }
            }
            Spec::SignedInt {
                width,
                precision,
                positive_sign,
                alignment,
            } => {
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor);
                let p = resolve_precision(*precision, self.precision_index, cursor).unwrap_or(0);
                let align = if dyn_left {
                    NumberAlignment::Left
                } else {
                    *alignment
                };
                let i = cursor.get_i64(self.arg_index);
                num_format::SignedInt {
                    width: w.unwrap_or(0),
                    precision: p,
                    positive_sign: *positive_sign,
                    alignment: align,
                }
                .fmt(writer, i)
                .map_err(FormatError::IoError)
            }
            Spec::UnsignedInt {
                variant,
                width,
                precision,
                alignment,
            } => {
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor);
                let p = resolve_precision(*precision, self.precision_index, cursor).unwrap_or(0);
                let align = if dyn_left {
                    NumberAlignment::Left
                } else {
                    *alignment
                };
                let i = cursor.get_u64(self.arg_index);
                num_format::UnsignedInt {
                    variant: *variant,
                    precision: p,
                    width: w.unwrap_or(0),
                    alignment: align,
                }
                .fmt(writer, i)
                .map_err(FormatError::IoError)
            }
            Spec::Float {
                variant,
                case,
                force_decimal,
                width,
                positive_sign,
                alignment,
                precision,
            } => {
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor);
                let p = resolve_precision(*precision, self.precision_index, cursor).unwrap_or(6);
                let align = if dyn_left {
                    NumberAlignment::Left
                } else {
                    *alignment
                };
                let f = cursor.get_f64(self.arg_index);
                num_format::Float {
                    width: w.unwrap_or(0),
                    precision: p,
                    variant: *variant,
                    case: *case,
                    force_decimal: *force_decimal,
                    positive_sign: *positive_sign,
                    alignment: align,
                }
                .fmt(writer, f)
                .map_err(FormatError::IoError)
            }
        }
    }
}

impl Spec {
    fn parse_length(rest: &mut &[u8], index: &mut usize) -> Option<Length> {
        let mut length = None;
        loop {
            let new_length = rest.get(*index).and_then(|c| {
                Some(match c {
                    b'h' => match rest.get(*index + 1) {
                        Some(b'h') => {
                            *index += 1;
                            Length::Char
                        }
                        _ => Length::Short,
                    },
                    b'l' => match rest.get(*index + 1) {
                        Some(b'h') => {
                            *index += 1;
                            Length::Long
                        }
                        _ => Length::LongLong,
                    },
                    b'z' => Length::SizeT,
                    b'j' => Length::IntMaxT,
                    b't' => Length::PtfDiffT,
                    b'L' => Length::LongDouble,
                    _ => return None,
                })
            });

            if new_length.is_none() {
                break;
            } else {
                *index += 1;
                length = new_length;
            }
        }
        length
    }
}

fn resolve_width<'a>(
    option: Option<CanAsterisk<usize>>,
    idx: Option<usize>,
    cursor: &mut ArgCursor<'a>,
) -> (Option<usize>, bool) {
    match option {
        None => (None, false),
        Some(CanAsterisk::Asterisk) => {
            let v = cursor.get_i64(idx);
            if v < 0 {
                (Some(v.unsigned_abs() as usize), true)
            } else {
                (Some(v as usize), false)
            }
        }
        Some(CanAsterisk::Fixed(w)) => (Some(w), false),
    }
}

// 精度如果接收到负数，直接当作被忽略 (None) 处理
fn resolve_precision<'a>(
    option: Option<CanAsterisk<usize>>,
    idx: Option<usize>,
    cursor: &mut ArgCursor<'a>,
) -> Option<usize> {
    match option {
        None => None,
        Some(CanAsterisk::Asterisk) => {
            let v = cursor.get_i64(idx);
            if v < 0 { None } else { Some(v as usize) }
        }
        Some(CanAsterisk::Fixed(p)) => Some(p),
    }
}

fn write_padded(
    mut writer_io: impl Write,
    text: &[u8],
    width: usize,
    align_left: bool,
) -> Result<(), FormatError> {
    let pad_len = width.saturating_sub(text.len());
    if align_left {
        writer_io.write_all(text)?;
        write!(writer_io, "{: <pad_len$}", "", pad_len = pad_len)
    } else {
        write!(writer_io, "{: >pad_len$}", "", pad_len = pad_len)?;
        writer_io.write_all(text)
    }
    .map_err(FormatError::IoError)
}

fn peek_number(rest: &[u8], index: usize) -> Option<(usize, usize)> {
    let mut len = 0;
    while let Some(&b) = rest.get(index + len) {
        if b.is_ascii_digit() {
            len += 1;
        } else {
            break;
        }
    }
    if len == 0 {
        return None;
    }
    let s = std::str::from_utf8(&rest[index..index + len]).unwrap();
    s.parse::<usize>().ok().map(|v| (v, len))
}

fn eat_asterisk_or_number(
    rest: &mut &[u8],
    index: &mut usize,
) -> Option<(CanAsterisk<usize>, Option<usize>)> {
    if rest.is_empty() {
        return None;
    }
    match rest.get(*index) {
        Some(b'*') => {
            *index += 1;
            if let Some((num, len)) = peek_number(rest, *index) {
                if rest.get(*index + len) == Some(&b'$') {
                    *index += len + 1;
                    return Some((CanAsterisk::Asterisk, Some(num)));
                }
            }
            Some((CanAsterisk::Asterisk, None))
        }
        _ => eat_number(rest, index).map(|n| (CanAsterisk::Fixed(n), None)),
    }
}

/**
 * 从字节切片中解析数字，并更新解析位置。
 */
fn eat_number(rest: &mut &[u8], index: &mut usize) -> Option<usize> {
    match rest[*index..].iter().position(|b| !b.is_ascii_digit()) {
        Some(0) | None => None,
        Some(i) => {
            let slice = &rest[*index..(*index + i)];
            match std::str::from_utf8(slice) {
                Ok(str_slice) => match str_slice.parse() {
                    Ok(parsed) => {
                        *index += i;
                        Some(parsed)
                    }
                    Err(_) => None,
                },
                Err(_) => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_specifier() {
        let mut input: &[u8] = b"d";
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::None,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_width() {
        let mut input: &[u8] = b"d3";
        let _width_value: CanAsterisk<usize> = CanAsterisk::Asterisk;
        let _precision_value: CanAsterisk<usize> = CanAsterisk::Fixed(2);
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::None,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_precision() {
        let mut input: &[u8] = b"d3.2";
        let _width_value: CanAsterisk<usize> = CanAsterisk::Asterisk;
        let _precision_value: CanAsterisk<usize> = CanAsterisk::Fixed(2);
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::None,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_width_and_precision() {
        let mut input: &[u8] = b"d3.2.3";
        let _width_value: CanAsterisk<usize> = CanAsterisk::Asterisk;
        let _precision_value: CanAsterisk<usize> = CanAsterisk::Fixed(2);
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::None,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_minus_flag() {
        let mut input: &[u8] = b"-d3.2";
        let _width_value: CanAsterisk<usize> = CanAsterisk::Fixed(3);
        let _precision_value: CanAsterisk<usize> = CanAsterisk::Fixed(2);
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::Left,
            positive_sign: PositiveSign::None,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_plus_flag() {
        let mut input: &[u8] = b"+d3.2";
        let _width_value: CanAsterisk<usize> = CanAsterisk::Asterisk;
        let _precision_value: CanAsterisk<usize> = CanAsterisk::Fixed(2);
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::Plus,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_space_flag() {
        let mut input: &[u8] = b" d3.2";
        let _width_value: CanAsterisk<usize> = CanAsterisk::Asterisk;
        let _precision_value: CanAsterisk<usize> = CanAsterisk::Fixed(2);
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::Space,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_asterisk_flag() {
        let mut input: &[u8] = b"*d3.2";
        let width_value: CanAsterisk<usize> = CanAsterisk::Asterisk;
        let _precision_value: CanAsterisk<usize> = CanAsterisk::Fixed(2);
        let expected = Spec::SignedInt {
            width: Some(width_value),
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::None,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_zero_flag() {
        let _width_value: CanAsterisk<usize> = CanAsterisk::Asterisk;
        let _precision_value: CanAsterisk<usize> = CanAsterisk::Fixed(2);
        let mut input: &[u8] = b"0d3.2";
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightZero,
            positive_sign: PositiveSign::None,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_hash_flag() {
        let width_value: CanAsterisk<usize> = CanAsterisk::Asterisk;
        let precision_value: CanAsterisk<usize> = CanAsterisk::Fixed(2);
        let mut input: &[u8] = b"#d3.2";
        let rest: &[u8] = &[35, 100];
        let _expected = Spec::SignedInt {
            width: Some(width_value),
            precision: Some(precision_value),
            alignment: NumberAlignment::Left,
            positive_sign: PositiveSign::None,
        };
        assert_eq!(IndexedSpec::parse(&mut input), Err(rest));
        // assert_eq!(Spec::parse(&mut input), Err([Spec::EscapedString, 100]));
    }

    #[test]
    fn test_parse_specifier_with_l_flag() {
        let mut input: &[u8] = b"l";
        let rest: &[u8] = b"l";
        let _expected = Spec::Char {
            width: None,
            align_left: true,
        };
        assert_eq!(IndexedSpec::parse(&mut input), Err(rest));
    }

    #[test]
    fn test_parse_specifier_with_l_flag2() {
        let mut input: &[u8] = b"2.3L";
        let _expected = Spec::Char {
            width: None,
            align_left: true,
        };
        let rest: &[u8] = b"2.3L";
        assert_eq!(IndexedSpec::parse(&mut input), Err(rest));
    }

    #[test]
    fn test_parse_specifier_with_h_flag2() {
        let mut input: &[u8] = b"H";
        let rest: &[u8] = b"H";
        let _expected = Spec::Char {
            width: None,
            align_left: false,
        };
        assert_eq!(IndexedSpec::parse(&mut input), Err(rest));
    }

    #[test]
    fn test_parse_length_char() {
        let mut rest: &[u8] = b"hh";
        let mut index = 0;
        assert_eq!(
            Spec::parse_length(&mut rest, &mut index),
            Some(Length::Char)
        );
    }

    #[test]
    fn test_parse_length_short() {
        let mut rest: &[u8] = b"h";
        let mut index = 0;
        assert_eq!(
            Spec::parse_length(&mut rest, &mut index),
            Some(Length::Short)
        );
    }

    // Add more tests for other length options (Long, LongLong, IntMaxT, etc.)

    #[test]
    fn test_parse_length_invalid() {
        let mut rest: &[u8] = b"abc"; // invalid length option
        let mut index = 0;
        assert_eq!(Spec::parse_length(&mut rest, &mut index), None);
    }

    #[test]
    fn test_parse_length_no_length() {
        let mut rest: &[u8] = b"";
        let mut index = 0;
        assert_eq!(Spec::parse_length(&mut rest, &mut index), None);
    }

    #[test]
    fn test_parse_length_with_other_specifiers() {
        let mut rest: &[u8] = b"zhlt"; // mixed length and other specifiers
        let mut index = 0;
        assert_eq!(
            Spec::parse_length(&mut rest, &mut index),
            Some(Length::PtfDiffT)
        );
        assert_eq!(index, 4); // Make sure only the length specifier is consumed
    }

    #[test]
    fn test_eat_number_empty_input() {
        let mut rest: &[u8] = &[];
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), None);
    }

    #[test]
    fn test_eat_number_no_digits() {
        let mut rest: &[u8] = b"hij"; // "hij"
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), None);
        assert_eq!(index, 0);
    }

    #[test]
    fn test_eat_number_single_digit() {
        let mut rest: &[u8] = b"0"; // "0"
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), None);
        assert_eq!(index, 0);
    }

    #[test]
    fn test_eat_number_multiple_digits() {
        // "345"
        let mut rest: &[u8] = b"345";
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), None);
        assert_eq!(index, 0);
    }

    #[test]
    fn test_eat_number_multiple_digits2() {
        // "3x5"
        let mut rest: &[u8] = b"3q5";
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), Some(3));
        assert_eq!(index, 1);
    }

    #[test]
    fn test_eat_number_mixed_digits_and_non_digits() {
        // "2345hij"
        let mut rest: &[u8] = b"2345hij";
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), Some(2345));
        assert_eq!(index, 4);
    }

    #[test]
    fn test_eat_number_non_digit_followed_by_digits() {
        // "hij012"
        let mut rest: &[u8] = b"hij012";
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), None);
        assert_eq!(index, 0);
    }

    #[test]
    fn test_eat_asterisk_or_number_positive() {
        let mut rest: &[u8] = &mut [b'*', b'3', b'5', b'7'];
        let mut index = 0;
        assert_eq!(
            eat_asterisk_or_number(&mut rest, &mut index).map(|(v, _)| v),
            Some(CanAsterisk::Asterisk)
        );
    }

    #[test]
    fn test_eat_asterisk_or_number_negative() {
        let mut rest: &[u8] = &mut [b'2', b'5', b'7'];
        let mut index = 0;
        if let Some((eat_asterisk_or_number_value, _)) =
            eat_asterisk_or_number(&mut rest, &mut index)
        {
            assert_eq!(eat_asterisk_or_number_value, CanAsterisk::Fixed(257));
        }
    }

    #[test]
    fn test_eat_asterisk_or_number_not_an_asterisk() {
        let mut rest: &[u8] = &mut [b'3', b'5', b'7', b'a'];
        let mut index = 0;

        assert_eq!(
            eat_asterisk_or_number(&mut rest, &mut index).map(|(v, _)| v),
            Some(CanAsterisk::Fixed(357))
        );
    }
    #[test]
    fn test_eat_asterisk_or_number_no_asterisk() {
        let mut rest: &[u8] = &mut [b'2', b'5', b'7'];
        let mut index = 0;
        assert_eq!(eat_asterisk_or_number(&mut rest, &mut index), None);
        assert_eq!(index, 0); // 索引不应该增加
    }

    #[test]
    fn test_eat_asterisk_or_number_no_data() {
        let mut rest: &[u8] = &mut [];
        let mut index = 0;
        assert_eq!(eat_asterisk_or_number(&mut rest, &mut index), None);
        assert_eq!(index, 0); // 索引不应该增加
    }

    #[test]
    fn test_eat_asterisk_or_number_no_number_err() {
        let mut rest: &[u8] = &mut [b'a'];
        let mut index = 0;
        assert_eq!(eat_asterisk_or_number(&mut rest, &mut index), None);
        assert_eq!(index, 0); // 索引不应该增加
    }

    #[test]
    fn test_eat_asterisk_or_number_no_number() {
        let mut rest: &[u8] = &mut [b'*', b'a'];
        let mut index = 0;
        assert_eq!(
            eat_asterisk_or_number(&mut rest, &mut index).map(|(v, _)| v),
            Some(CanAsterisk::Asterisk)
        );
        assert_eq!(index, 1); // 索引不应该增加
    }

    #[test]
    fn test_eat_asterisk_or_number_no_number2() {
        let mut rest: &[u8] = &mut [b'*', b' ', b'a'];
        let mut index = 0;
        assert_eq!(
            eat_asterisk_or_number(&mut rest, &mut index).map(|(v, _)| v),
            Some(CanAsterisk::Asterisk)
        );
        assert_eq!(index, 1); // 索引不应该增加
    }

    #[test]
    fn test_eat_asterisk_or_number_no_number3() {
        let mut rest: &[u8] = &mut [b'*', b'b', b'a'];
        let mut index = 0;
        assert_eq!(
            eat_asterisk_or_number(&mut rest, &mut index).map(|(v, _)| v),
            Some(CanAsterisk::Asterisk)
        );
        assert_eq!(index, 1); // 索引不应该增加
    }

    #[test]
    fn test_write_padded_left_align() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 20;
        let left = true;
        let expected = b"Hello, world!       ";
        let result = write_padded(&mut writer, text, width, left);
        assert!(result.is_ok());
        assert_eq!(writer, expected);
    }

    #[test]
    fn test_write_padded_right_align() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 20;
        let left = false;
        let expected = b"       Hello, world!";
        let result = write_padded(&mut writer, text, width, left);
        assert!(result.is_ok());
        assert_eq!(writer, expected);
    }

    #[test]
    fn test_write_padded_text_too_long() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 5;
        let left = true;
        let expected = b"Hello, world!";
        let result = write_padded(&mut writer, text, width, left);
        assert!(result.is_ok());
        assert_eq!(writer, expected);
    }

    #[test]
    fn test_write_padded_io_error() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 20;
        let left = true;
        let result = write_padded(&mut writer, text, width, left);
        assert!(result.is_ok());
    }

    #[test]
    fn test_write_padded_empty_input_left() {
        let mut writer = Vec::<u8>::new();
        let text = &[];
        let width = 20;
        let left = true;
        let result = write_padded(&mut writer, text, width, left);
        assert!(result.is_ok());
        assert_eq!(writer, b"                    ");
    }
    #[test]
    fn test_write_padded_empty_input_right() {
        let mut writer = Vec::<u8>::new();
        let text: &[u8] = &mut [];
        let width = 20;
        let left = false;
        let result = write_padded(&mut writer, text, width, left);
        assert!(result.is_ok());
        assert_eq!(writer, b"                    ");
    }

    #[test]
    fn test_write_padded_empty_width() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 0;
        let left = true;
        let result = write_padded(&mut writer, text, width, left);
        assert!(result.is_ok());
        assert_eq!(writer, b"Hello, world!");
    }

    #[test]
    fn test_write_padded_null_width_left() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 0;
        let left = true;
        let result = write_padded(&mut writer, text, width, left);
        assert!(result.is_ok());
        assert_eq!(writer, b"Hello, world!");
    }

    #[test]
    fn test_write_padded_null_width_right() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 0;
        let left = false;
        let result = write_padded(&mut writer, text, width, left);
        assert!(result.is_ok());
        assert_eq!(writer, b"Hello, world!");
    }
}
