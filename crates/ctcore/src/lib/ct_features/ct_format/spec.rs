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
use crate::ct_format::long_double::GnuFloatFormat;
use crate::ct_quoting_style::{
    CtQuotingStyle, escape_name, escape_unibyte_shell_bytes, uses_unibyte_locale,
};
use std::ffi::CStr;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
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
        thousand_separate: bool,
    },
    UnsignedInt {
        variant: UnsignedIntVariant,
        width: Option<CanAsterisk<usize>>,
        precision: Option<CanAsterisk<usize>>,
        alignment: NumberAlignment,
        thousand_separate: bool,
    },
    Float {
        variant: FloatVariant,
        case: Case,
        force_decimal: ForceDecimal,
        width: Option<CanAsterisk<usize>>,
        positive_sign: PositiveSign,
        alignment: NumberAlignment,
        precision: Option<CanAsterisk<usize>>,
        thousand_separate: bool,
        localized_digits: bool,
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
    localized_digits: bool,
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
                b'I' => flags.localized_digits = true,
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
        let length = Spec::parse_length(rest, &mut temp_idx);
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
                if flags.hash
                    || flags.zero
                    || flags.quote
                    || flags.localized_digits
                    || precision.is_some()
                {
                    return Err(&start[..index]);
                }
                Spec::Char {
                    width,
                    align_left: flags.minus,
                }
            }
            b's' => {
                // 对于字符串类型，单引号标志是非法的
                if flags.hash || flags.zero || flags.quote || flags.localized_digits {
                    return Err(&start[..index]);
                }
                Spec::String {
                    precision,
                    width,
                    align_left: flags.minus,
                }
            }
            b'b' => {
                if flags.any() || width.is_some() || precision.is_some() || length.is_some() {
                    return Err(&start[..index]);
                }
                Spec::EscapedString
            }
            b'q' => {
                if flags.any() || width.is_some() || precision.is_some() || length.is_some() {
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
                    thousand_separate: flags.quote,
                }
            }
            c @ (b'o' | b'u' | b'x' | b'X') => {
                if flags.hash && *c == b'u' {
                    return Err(&start[..index]);
                }
                if flags.localized_digits && *c != b'u' {
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
                    thousand_separate: flags.quote,
                }
            }
            c @ (b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G') => {
                if (flags.localized_digits || flags.quote) && matches!(c, b'a' | b'A' | b'e' | b'E')
                {
                    return Err(&start[..index]);
                }
                let float_alignment = if flags.minus {
                    NumberAlignment::Left
                } else if flags.zero {
                    NumberAlignment::RightZero
                } else {
                    NumberAlignment::RightSpace
                };
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
                    alignment: float_alignment,
                    positive_sign,
                    thousand_separate: flags.quote,
                    localized_digits: flags.localized_digits,
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
    ) -> Result<ControlFlow<()>, FormatError> {
        match &self.spec {
            Spec::Char { width, align_left } => {
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor)?;
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
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor)?;
                let p = resolve_precision(*precision, self.precision_index, cursor)?;
                let bytes = cursor.get_bytes(self.arg_index);
                let truncated = match p {
                    Some(prec) if prec < bytes.len() => &bytes[..prec],
                    _ => bytes,
                };
                write_padded(writer, truncated, w.unwrap_or(0), *align_left || dyn_left)
            }
            Spec::EscapedString => {
                let bytes = cursor.get_bytes(self.arg_index);
                for res in parse_escape_only(bytes) {
                    match res?.write(&mut writer)? {
                        ControlFlow::Continue(()) => {}
                        ControlFlow::Break(()) => return Ok(ControlFlow::Break(())),
                    };
                }
                Ok(())
            }
            Spec::QuotedString => {
                let Some(bytes) = cursor.get_optional_bytes(self.arg_index) else {
                    return Ok(ControlFlow::Continue(()));
                };
                if bytes.is_empty() {
                    writer.write_all(b"''").map_err(FormatError::IoError)
                } else if uses_unibyte_locale() {
                    writer
                        .write_all(escape_unibyte_shell_bytes(bytes).as_bytes())
                        .map_err(FormatError::IoError)
                } else {
                    writer
                        .write_all(
                            escape_name(
                                OsStr::from_bytes(bytes),
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
                thousand_separate,
            } => {
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor)?;
                let p = resolve_precision(*precision, self.precision_index, cursor)?;
                let align = if dyn_left {
                    NumberAlignment::Left
                } else {
                    *alignment
                };
                let i = cursor.get_i64(self.arg_index);
                let width = w.unwrap_or(0);

                if !thousand_separate {
                    num_format::SignedInt {
                        width,
                        precision: p,
                        positive_sign: *positive_sign,
                        alignment: align,
                    }
                    .fmt(writer, i)
                    .map_err(FormatError::IoError)
                } else {
                    let suppress_zero = i == 0 && p == Some(0);
                    let raw_digit_count = if suppress_zero {
                        0
                    } else {
                        i.unsigned_abs().to_string().len()
                    };
                    let mut raw = Vec::new();
                    num_format::SignedInt {
                        width: 0,
                        precision: suppress_zero.then_some(0),
                        positive_sign: *positive_sign,
                        alignment: NumberAlignment::Left,
                    }
                    .fmt(&mut raw, i)
                    .map_err(FormatError::IoError)?;

                    let grouped = group_number_thousands(&String::from_utf8_lossy(&raw));
                    let grouped = add_integer_precision_zeros(&grouped, raw_digit_count, p);
                    write_number_aligned(writer, &grouped, width, align)
                        .map_err(FormatError::IoError)
                }
            }
            Spec::UnsignedInt {
                variant,
                width,
                precision,
                alignment,
                thousand_separate,
            } => {
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor)?;
                let p = resolve_precision(*precision, self.precision_index, cursor)?;
                let align = if dyn_left {
                    NumberAlignment::Left
                } else {
                    *alignment
                };
                let i = cursor.get_u64(self.arg_index);
                let width = w.unwrap_or(0);

                if !thousand_separate {
                    num_format::UnsignedInt {
                        variant: *variant,
                        precision: p,
                        width,
                        alignment: align,
                    }
                    .fmt(writer, i)
                    .map_err(FormatError::IoError)
                } else {
                    let suppress_zero = i == 0 && p == Some(0);
                    let raw_digit_count = if suppress_zero {
                        0
                    } else {
                        i.to_string().len()
                    };
                    let mut raw = Vec::new();
                    num_format::UnsignedInt {
                        variant: *variant,
                        precision: suppress_zero.then_some(0),
                        width: 0,
                        alignment: NumberAlignment::Left,
                    }
                    .fmt(&mut raw, i)
                    .map_err(FormatError::IoError)?;

                    let grouped = group_number_thousands(&String::from_utf8_lossy(&raw));
                    let grouped = add_integer_precision_zeros(&grouped, raw_digit_count, p);
                    write_number_aligned(writer, &grouped, width, align)
                        .map_err(FormatError::IoError)
                }
            }
            Spec::Float {
                variant,
                case,
                force_decimal,
                width,
                positive_sign,
                alignment,
                precision,
                thousand_separate,
                localized_digits,
            } => {
                let (w, dyn_left) = resolve_width(*width, self.width_index, cursor)?;
                let p = resolve_precision(*precision, self.precision_index, cursor)?;
                let align = if dyn_left {
                    NumberAlignment::Left
                } else {
                    *alignment
                };
                let f = cursor.get_long_double(self.arg_index);
                let width = w.unwrap_or(0);
                let format = num_format::Float {
                    width: 0,
                    precision: p,
                    variant: *variant,
                    case: *case,
                    force_decimal: *force_decimal,
                    positive_sign: *positive_sign,
                    alignment: align,
                };
                let format = num_format::Float { width, ..format };
                let mut rendered =
                    GnuFloatFormat::from_float_format(&format, *thousand_separate).format(&f);
                if *localized_digits && printf_uses_empty_outdigits() {
                    rendered = remove_ascii_digits_for_empty_outdigits(&rendered);
                }
                writer.write_all(&rendered).map_err(FormatError::IoError)
            }
        }?;
        Ok(ControlFlow::Continue(()))
    }
}

fn printf_uses_empty_outdigits() -> bool {
    let locale = unsafe { crate::libc::setlocale(crate::libc::LC_CTYPE, std::ptr::null()) };
    if locale.is_null() {
        return false;
    }
    matches!(
        unsafe { CStr::from_ptr(locale) }.to_bytes(),
        b"C" | b"POSIX"
    )
}

fn remove_ascii_digits_for_empty_outdigits(value: &[u8]) -> Vec<u8> {
    value
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_digit())
        .collect()
}

fn group_number_thousands(number: &str) -> String {
    let Some((sep, grouping)) = locale_thousands_grouping() else {
        return number.to_string();
    };

    group_number_thousands_with(number, &sep, &grouping)
}

fn add_integer_precision_zeros(
    grouped: &str,
    raw_digit_count: usize,
    precision: Option<usize>,
) -> String {
    let zero_count = precision.unwrap_or(0).saturating_sub(raw_digit_count);
    if zero_count == 0 {
        return grouped.to_string();
    }

    let sign_length = usize::from(matches!(
        grouped.as_bytes().first(),
        Some(b'+') | Some(b'-') | Some(b' ')
    ));
    let mut output = String::with_capacity(grouped.len() + zero_count);
    output.push_str(&grouped[..sign_length]);
    output.push_str(&"0".repeat(zero_count));
    output.push_str(&grouped[sign_length..]);
    output
}

fn group_number_thousands_with(number: &str, sep: &str, grouping: &[u8]) -> String {
    if sep.is_empty() || grouping.is_empty() {
        return number.to_string();
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
        return number.to_string();
    }

    let (int_part, frac_part) = match rest.find('.') {
        Some(dot_idx) => (&rest[..dot_idx], &rest[dot_idx..]),
        None => (rest, ""),
    };

    if int_part.len() <= 3 || !int_part.bytes().all(|c| c.is_ascii_digit()) {
        return number.to_string();
    }

    let grouped = apply_locale_grouping(int_part, sep, grouping);
    if grouped == int_part {
        return number.to_string();
    }

    let mut output =
        String::with_capacity(sign.len() + grouped.len() + frac_part.len() + exp.len());
    output.push_str(sign);
    output.push_str(&grouped);
    output.push_str(frac_part);
    output.push_str(exp);
    output
}

fn locale_thousands_grouping() -> Option<(String, Vec<u8>)> {
    unsafe {
        let lc = crate::libc::localeconv();
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

fn apply_locale_grouping(int_part: &str, sep: &str, grouping: &[u8]) -> String {
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

fn write_number_aligned(
    mut writer: impl Write,
    text: &str,
    width: usize,
    alignment: NumberAlignment,
) -> std::io::Result<()> {
    if text.len() >= width {
        return writer.write_all(text.as_bytes());
    }

    let pad_len = width - text.len();
    match alignment {
        NumberAlignment::Left => {
            writer.write_all(text.as_bytes())?;
            write!(writer, "{: >pad_len$}", "", pad_len = pad_len)
        }
        NumberAlignment::RightSpace => {
            write!(writer, "{: >pad_len$}", "", pad_len = pad_len)?;
            writer.write_all(text.as_bytes())
        }
        NumberAlignment::RightZero => {
            let mut sign_len = 0;
            let bytes = text.as_bytes();
            if matches!(bytes.first(), Some(b'+') | Some(b'-') | Some(b' ')) {
                sign_len = 1;
            }

            let mut prefix_len = sign_len;
            if text[sign_len..].starts_with("0x") || text[sign_len..].starts_with("0X") {
                prefix_len += 2;
            }

            writer.write_all(&bytes[..prefix_len])?;
            write!(writer, "{:0>pad_len$}", "", pad_len = pad_len)?;
            writer.write_all(&bytes[prefix_len..])
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
) -> Result<(Option<usize>, bool), FormatError> {
    match option {
        None => Ok((None, false)),
        Some(CanAsterisk::Asterisk) => {
            let (v, source) = cursor.get_i64_with_source(idx);
            if v < i64::from(i32::MIN) || v > i64::from(i32::MAX) {
                return Err(FormatError::InvalidFieldWidth(source));
            }
            if v == i64::from(i32::MIN) {
                return Err(FormatError::WriteError);
            }
            if v < 0 {
                Ok((Some(v.unsigned_abs() as usize), true))
            } else {
                Ok((Some(v as usize), false))
            }
        }
        Some(CanAsterisk::Fixed(w)) => Ok((Some(w), false)),
    }
}

// 精度如果接收到负数，直接当作被忽略 (None) 处理
fn resolve_precision<'a>(
    option: Option<CanAsterisk<usize>>,
    idx: Option<usize>,
    cursor: &mut ArgCursor<'a>,
) -> Result<Option<usize>, FormatError> {
    match option {
        None => Ok(None),
        Some(CanAsterisk::Asterisk) => {
            let (v, source) = cursor.get_i64_with_source(idx);
            if v < 0 {
                Ok(None)
            } else if v > i64::from(i32::MAX) {
                Err(FormatError::InvalidPrecision(source))
            } else {
                Ok(Some(v as usize))
            }
        }
        Some(CanAsterisk::Fixed(p)) => Ok(Some(p)),
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
    use crate::ct_format::argument::FormatArgument;

    #[test]
    fn quoted_string_escapes_non_printable_multibyte_bytes() {
        let mut input: &[u8] = b"q";
        let spec = IndexedSpec::parse(&mut input).unwrap();
        let arguments = [FormatArgument::Bytes(vec![0xc2, 0x81])];
        let mut cursor = ArgCursor::new(&arguments);
        let mut output = Vec::new();

        assert!(matches!(
            spec.write(&mut output, &mut cursor),
            Ok(ControlFlow::Continue(()))
        ));
        assert_eq!(output, b"''$'\\302\\201'");
    }

    #[test]
    fn dynamic_width_and_precision_reject_values_above_int_max() {
        let arguments = [FormatArgument::Bytes(b"2147483648".to_vec())];
        let mut width_cursor = ArgCursor::new(&arguments);
        let mut precision_cursor = ArgCursor::new(&arguments);

        assert!(matches!(
            resolve_width(
                Some(CanAsterisk::Asterisk),
                None,
                &mut width_cursor
            ),
            Err(FormatError::InvalidFieldWidth(value)) if value == b"2147483648"
        ));
        assert!(matches!(
            resolve_precision(
                Some(CanAsterisk::Asterisk),
                None,
                &mut precision_cursor
            ),
            Err(FormatError::InvalidPrecision(value)) if value == b"2147483648"
        ));
    }

    #[test]
    fn dynamic_width_int_min_reports_write_error() {
        let arguments = [FormatArgument::Unparsed(i32::MIN.to_string())];
        let mut cursor = ArgCursor::new(&arguments);

        assert!(matches!(
            resolve_width(Some(CanAsterisk::Asterisk), None, &mut cursor),
            Err(FormatError::WriteError)
        ));
    }

    #[test]
    fn grouped_integer_precision_zeros_are_not_grouped() {
        let grouped = group_number_thousands_with("1234", ",", &[3, 0]);
        assert_eq!(
            add_integer_precision_zeros(&grouped, 4, Some(12)),
            "000000001,234"
        );

        let grouped_negative = group_number_thousands_with("-1234", ",", &[3, 0]);
        assert_eq!(
            add_integer_precision_zeros(&grouped_negative, 4, Some(12)),
            "-000000001,234"
        );
    }

    #[test]
    fn test_parse_simple_specifier() {
        let mut input: &[u8] = b"d";
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::None,
            thousand_separate: false,
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
            thousand_separate: false,
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
            thousand_separate: false,
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
            thousand_separate: false,
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
            thousand_separate: false,
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
            thousand_separate: false,
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
            thousand_separate: false,
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
            thousand_separate: false,
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
            thousand_separate: false,
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
            thousand_separate: false,
        };
        assert_eq!(IndexedSpec::parse(&mut input), Err(rest));
        // assert_eq!(Spec::parse(&mut input), Err([Spec::EscapedString, 100]));
    }

    #[test]
    fn test_parse_specifier_with_quote_flag() {
        let mut input: &[u8] = b"'d";
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::None,
            thousand_separate: true,
        };
        assert_eq!(
            IndexedSpec::parse(&mut input).map(|is| is.spec),
            Ok(expected)
        );
    }

    #[test]
    fn test_parse_specifier_with_localized_digits_flag() {
        let mut input: &[u8] = b"Id";
        let expected = Spec::SignedInt {
            width: None,
            precision: None,
            alignment: NumberAlignment::RightSpace,
            positive_sign: PositiveSign::None,
            thousand_separate: false,
        };

        assert_eq!(
            IndexedSpec::parse(&mut input).map(|indexed| indexed.spec),
            Ok(expected)
        );
    }

    #[test]
    fn localized_digits_flag_rejects_character_and_string_conversions() {
        for specifier in [b"Ic".as_slice(), b"Is".as_slice()] {
            let mut input = specifier;

            assert_eq!(IndexedSpec::parse(&mut input), Err(specifier));
        }
    }

    #[test]
    fn empty_outdigits_remove_digits_after_float_width_formatting() {
        assert_eq!(
            remove_ascii_digits_for_empty_outdigits(b"     +12.30e-04"),
            b"     +.e-"
        );
    }

    #[test]
    fn grouping_flag_rejects_hexadecimal_and_scientific_float_conversions() {
        for specifier in [
            b"'a".as_slice(),
            b"'A".as_slice(),
            b"'e".as_slice(),
            b"'E".as_slice(),
        ] {
            let mut input = specifier;

            assert_eq!(IndexedSpec::parse(&mut input), Err(specifier));
        }
    }

    #[test]
    fn length_modifiers_reject_escaped_and_quoted_string_extensions() {
        for specifier in [b"Lb".as_slice(), b"zq".as_slice()] {
            let mut input = specifier;

            assert_eq!(IndexedSpec::parse(&mut input), Err(specifier));
        }
    }

    #[test]
    fn test_group_number_thousands_with_standard_grouping() {
        assert_eq!(
            group_number_thousands_with("123456789", ",", &[3, 0]),
            "123,456,789"
        );
        assert_eq!(
            group_number_thousands_with("-1234567.89", ",", &[3, 0]),
            "-1,234,567.89"
        );
        assert_eq!(
            group_number_thousands_with("1234567e+08", ",", &[3, 0]),
            "1,234,567e+08"
        );
    }

    #[test]
    fn test_group_number_thousands_with_non_repeating_grouping() {
        assert_eq!(
            group_number_thousands_with("123456789", ",", &[3, 2]),
            "12,34,56,789"
        );
    }

    #[test]
    fn test_write_specifier_with_quote_flag_groups_signed_integer() {
        let mut input: &[u8] = b"'d";
        let spec = IndexedSpec::parse(&mut input).unwrap();
        let args = vec![FormatArgument::SignedInt(1_234_567)];
        let mut cursor = ArgCursor::new(&args);
        let mut out = Vec::new();
        assert_eq!(
            spec.write(&mut out, &mut cursor).unwrap(),
            ControlFlow::Continue(())
        );
        let out = String::from_utf8(out).unwrap();
        if let Some((sep, _)) = locale_thousands_grouping() {
            assert!(out.contains(&sep), "output should contain locale separator");
        } else {
            assert_eq!(out, "1234567");
        }
    }

    #[test]
    fn test_write_specifier_with_quote_flag_groups_unsigned_integer() {
        let mut input: &[u8] = b"'u";
        let spec = IndexedSpec::parse(&mut input).unwrap();
        let args = vec![FormatArgument::UnsignedInt(1_234_567)];
        let mut cursor = ArgCursor::new(&args);
        let mut out = Vec::new();
        assert_eq!(
            spec.write(&mut out, &mut cursor).unwrap(),
            ControlFlow::Continue(())
        );
        let out = String::from_utf8(out).unwrap();
        if let Some((sep, _)) = locale_thousands_grouping() {
            assert!(out.contains(&sep), "output should contain locale separator");
        } else {
            assert_eq!(out, "1234567");
        }
    }

    #[test]
    fn test_write_specifier_with_quote_flag_groups_float() {
        let mut input: &[u8] = b"'.2f";
        let spec = IndexedSpec::parse(&mut input).unwrap();
        let args = vec![FormatArgument::Float(12_345.678)];
        let mut cursor = ArgCursor::new(&args);
        let mut out = Vec::new();
        assert_eq!(
            spec.write(&mut out, &mut cursor).unwrap(),
            ControlFlow::Continue(())
        );
        let out = String::from_utf8(out).unwrap();
        if let Some((sep, _)) = locale_thousands_grouping() {
            assert!(out.contains(&sep), "output should contain locale separator");
        } else {
            assert_eq!(out, "12345.68");
        }
    }

    #[test]
    fn test_write_specifier_with_quote_flag_zero_padding_after_sign() {
        let mut input: &[u8] = b"'+010d";
        let spec = IndexedSpec::parse(&mut input).unwrap();
        let args = vec![FormatArgument::SignedInt(1234)];
        let mut cursor = ArgCursor::new(&args);
        let mut out = Vec::new();
        assert_eq!(
            spec.write(&mut out, &mut cursor).unwrap(),
            ControlFlow::Continue(())
        );
        let out = String::from_utf8(out).unwrap();
        if let Some((sep, _)) = locale_thousands_grouping() {
            assert!(
                out.starts_with("+") && out.contains(&sep),
                "output should preserve sign and contain locale separator"
            );
        } else {
            assert_eq!(out, "+000001234");
        }
    }

    #[test]
    fn test_write_escaped_string_propagates_end() {
        let mut input: &[u8] = b"b";
        let spec = IndexedSpec::parse(&mut input).unwrap();
        let args = vec![FormatArgument::Unparsed("a\\cb".to_string())];
        let mut cursor = ArgCursor::new(&args);
        let mut out = Vec::new();

        assert_eq!(
            spec.write(&mut out, &mut cursor).unwrap(),
            ControlFlow::Break(())
        );
        assert_eq!(out, b"a");
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
