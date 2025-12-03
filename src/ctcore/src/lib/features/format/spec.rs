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

// spell-checker:ignore (vars) intmax ptrdiff padlen

use crate::quoting_style::{escape_name, QuotingStyle};

use super::{
    num_format::{
        self, Case, FloatVariant, ForceDecimal, Formatter, NumberAlignment, PositiveSign, Prefix,
        UnsignedIntVariant,
    },
    parse_escape_only, ArgumentIter, FormatChar, FormatError,
};
use std::{io::Write, ops::ControlFlow};

/// A parsed specification for formatting a value
///
/// This might require more than one argument to resolve width or precision
/// values that are given as `*`.
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

/// Precision and width specified might use an asterisk to indicate that they are
/// determined by an argument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanAsterisk<T> {
    Fixed(T),
    Asterisk,
}

/// Size of the expected type (ignored)
///
/// We ignore this parameter entirely, but we do parse it.
/// It could be used in the future if the need arises.
#[derive(Debug, PartialEq)]
enum Length {
    /// signed/unsigned char ("hh")
    Char,
    /// signed/unsigned short int ("h")
    Short,
    /// signed/unsigned long int ("l")
    Long,
    /// signed/unsigned long long int ("ll")
    LongLong,
    /// intmax_t ("j")
    IntMaxT,
    /// size_t ("z")
    SizeT,
    /// ptrdiff_t ("t")
    PtfDiffT,
    /// long double ("L")
    LongDouble,
}

#[derive(Default, PartialEq, Eq)]
struct Flags {
    minus: bool,
    plus: bool,
    space: bool,
    hash: bool,
    zero: bool,
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
                _ => break,
            }
            *index += 1;
        }

        flags
    }

    /// Whether any of the flags is set to true
    fn any(&self) -> bool {
        self != &Self::default()
    }
}

impl Spec {
    pub fn parse<'a>(rest: &mut &'a [u8]) -> Result<Self, &'a [u8]> {
        // Based on the C++ reference, the spec format looks like:
        //
        //   %[flags][width][.precision][length]specifier
        //
        // However, we have already parsed the '%'.
        let mut index = 0;
        let start = *rest;

        let flags = Flags::parse(rest, &mut index);

        let positive_sign = if flags.plus {
            PositiveSign::Plus
        } else if flags.space {
            PositiveSign::Space
        } else {
            PositiveSign::None
        };

        let width = eat_asterisk_or_number(rest, &mut index);

        let precision = if let Some(b'.') = rest.get(index) {
            index += 1;
            let asterisk = eat_asterisk_or_number(rest, &mut index);
            Some(asterisk.unwrap_or(CanAsterisk::Fixed(0)))
        } else {
            None
        };

        // The `0` flag is ignored if `-` is given or a precision is specified.
        // So the only case for RightZero, is when `-` is not given and the
        // precision is none.
        let alignment = if flags.minus {
            NumberAlignment::Left
        } else if precision.is_none() && flags.zero {
            NumberAlignment::RightZero
        } else {
            NumberAlignment::RightSpace
        };

        // We ignore the length. It's not really relevant to printf
        let _ = Self::parse_length(rest, &mut index);

        // let Some(type_spec) = rest.get(index) else {
        //     return Err(&start[..index]);
        // };

        let type_spec = match rest.get(index) {
            Some(type_spec) => type_spec,
            None => {
                return Err(&start[..index]);
            }
        };

        index += 1;
        *rest = &start[index..];

        Ok(match type_spec {
            // GNU accepts minus, plus and space even though they are not used
            b'c' => {
                if flags.hash || flags.zero {
                    return Err(&start[..index]);
                }

                if precision.is_some() {
                    return Err(&start[..index]);
                }

                let align_left = flags.minus;
                Self::Char { width, align_left }
            }
            b's' => {
                if flags.hash || flags.zero {
                    return Err(&start[..index]);
                }

                let align_left = flags.minus;
                Self::String {
                    precision,
                    width,
                    align_left,
                }
            }
            b'b' => {
                if flags.any() {
                    return Err(&start[..index]);
                }
                if width.is_some() || precision.is_some() {
                    return Err(&start[..index]);
                }
                Self::EscapedString
            }
            b'q' => {
                if flags.any() {
                    return Err(&start[..index]);
                }
                if width.is_some() || precision.is_some() {
                    return Err(&start[..index]);
                }
                Self::QuotedString
            }
            b'd' | b'i' => {
                if flags.hash {
                    return Err(&start[..index]);
                }

                Self::SignedInt {
                    width,
                    alignment,
                    precision,
                    positive_sign,
                }
            }
            c @ (b'o' | b'u' | b'x' | b'X') => {
                // Normal unsigned integer cannot have a prefix
                if flags.hash && *c == b'u' {
                    return Err(&start[..index]);
                }

                let prefix = match flags.hash {
                    true => Prefix::Yes,
                    false => Prefix::No,
                };

                Self::UnsignedInt {
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
                let force_decimal = match flags.hash {
                    true => ForceDecimal::Yes,
                    false => ForceDecimal::No,
                };

                let case = match c.is_ascii_uppercase() {
                    true => Case::Uppercase,
                    false => Case::Lowercase,
                };

                let variant = match c {
                    b'a' | b'A' => FloatVariant::Hexadecimal,
                    b'e' | b'E' => FloatVariant::Scientific,
                    b'f' | b'F' => FloatVariant::Decimal,
                    b'g' | b'G' => FloatVariant::Shortest,

                    _ => unreachable!(),
                };

                Self::Float {
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
        })
    }

    fn parse_length(rest: &mut &[u8], index: &mut usize) -> Option<Length> {
        // Parse 0..N length options, keep the last one
        // Even though it is just ignored. We might want to use it later and we
        // should parse those characters.
        //
        // TODO: This needs to be configurable: `seq` accepts only one length
        //       param
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

    pub fn write<'a>(
        &self,
        mut writer: impl Write,
        mut args: impl ArgumentIter<'a>,
    ) -> Result<(), FormatError> {
        match self {
            Self::Char { width, align_left } => {
                let width = resolve_asterisk(*width, &mut args)?.unwrap_or(0);
                write_padded(writer, &[args.get_char()], width, *align_left)
            }
            Self::String {
                width,
                align_left,
                precision,
            } => {
                let width = resolve_asterisk(*width, &mut args)?.unwrap_or(0);

                // GNU does do this truncation on a byte level, see for instance:
                //     printf "%.1s" 🙃
                //     > �
                // For now, we let printf panic when we truncate within a code point.
                // TODO: We need to not use Rust's formatting for aligning the output,
                // so that we can just write bytes to stdout without panicking.
                let precision = resolve_asterisk(*precision, &mut args)?;
                let s = args.get_str();
                let truncated = match precision {
                    Some(p) if p < s.len() => &s[..p],
                    _ => s,
                };
                write_padded(writer, truncated.as_bytes(), width, *align_left)
            }
            Self::EscapedString => {
                let s = args.get_str();
                let mut parsed = Vec::new();
                for c in parse_escape_only(s.as_bytes()) {
                    match c.write(&mut parsed)? {
                        ControlFlow::Continue(()) => {}
                        ControlFlow::Break(()) => {
                            // TODO: This should break the _entire execution_ of printf
                            break;
                        }
                    };
                }
                writer.write_all(&parsed).map_err(FormatError::IoError)
            }
            Self::QuotedString => {
                let s = args.get_str();
                writer
                    .write_all(
                        escape_name(
                            s.as_ref(),
                            &QuotingStyle::Shell {
                                escape: true,
                                always_quote: false,
                                show_control: false,
                            },
                        )
                        .as_bytes(),
                    )
                    .map_err(FormatError::IoError)
            }
            Self::SignedInt {
                width,
                precision,
                positive_sign,
                alignment,
            } => {
                let width = resolve_asterisk(*width, &mut args)?.unwrap_or(0);
                let precision = resolve_asterisk(*precision, &mut args)?.unwrap_or(0);
                let i = args.get_i64();

                num_format::SignedInt {
                    width,
                    precision,
                    positive_sign: *positive_sign,
                    alignment: *alignment,
                }
                .fmt(writer, i)
                .map_err(FormatError::IoError)
            }
            Self::UnsignedInt {
                variant,
                width,
                precision,
                alignment,
            } => {
                let width = resolve_asterisk(*width, &mut args)?.unwrap_or(0);
                let precision = resolve_asterisk(*precision, &mut args)?.unwrap_or(0);
                let i = args.get_u64();

                num_format::UnsignedInt {
                    variant: *variant,
                    precision,
                    width,
                    alignment: *alignment,
                }
                .fmt(writer, i)
                .map_err(FormatError::IoError)
            }
            Self::Float {
                variant,
                case,
                force_decimal,
                width,
                positive_sign,
                alignment,
                precision,
            } => {
                let width = resolve_asterisk(*width, &mut args)?.unwrap_or(0);
                let precision = resolve_asterisk(*precision, &mut args)?.unwrap_or(6);
                let f = args.get_f64();

                num_format::Float {
                    width,
                    precision,
                    variant: *variant,
                    case: *case,
                    force_decimal: *force_decimal,
                    positive_sign: *positive_sign,
                    alignment: *alignment,
                }
                .fmt(writer, f)
                .map_err(FormatError::IoError)
            }
        }
    }
}

// 它负责在 CanAsterisk 枚举变体是星号时对其进行解析。resolve_asterisk 函数将一个 Option<CanAsterisk<usize>>
// 和一个 ArgumentIter 作为输入。函数返回一个 Option<usize> 或一个 FormatError。
// 函数首先检查 CanAsterisk 枚举变量是否为 None。如果是，函数返回 None。如果不是 None，函数将检查变量是否为
// CanAsterisk::Asterisk。如果是，函数将尝试使用 get_u64 方法解析 ArgumentIter 中的宽度。如果解析成功，
// 函数将返回用 Some(usize) 包装的解析宽度。如果解析失败，函数将返回 None。
// 如果变量不是 CanAsterisk::Asterisk，则必须是 CanAsterisk::Fixed(w)，其中 w 是固定宽度。在这种情况下，函数将返回 Some(w)。
// 函数使用 Result 类型处理错误。如果在解析过程中出现错误，函数将返回一个用 Err 包装的 FormatError。
fn resolve_asterisk<'a>(
    option: Option<CanAsterisk<usize>>,
    mut args: impl ArgumentIter<'a>,
) -> Result<Option<usize>, FormatError> {
    Ok(match option {
        None => None,
        Some(CanAsterisk::Asterisk) => Some(usize::try_from(args.get_u64()).ok().unwrap_or(0)),
        Some(CanAsterisk::Fixed(w)) => Some(w),
    })
}

// 该函数用于向写入器写入字符串，并根据 left 参数的指定在字符串的左侧或右侧填充空格。

// 函数首先从所需宽度减去文本长度，计算出要填充的空格数（padlen）。然后，它调用写入器的写入方法，传入文本和计算出的填充长度。
// 如果 left 为 true，文本将在左侧填充空格；否则，文本将在右侧填充空格。write！宏用于格式化填充后的文本，并将其写入写入器。
// 最后，函数会返回一个 Result，说明操作是否成功，并使用 map_err 方法将可能出现的 I/O 错误映射到 FormatError 值。
// fn write_padded(
//     mut writer: impl Write,
//     text: &[u8],
//     width: usize,
//     left: bool,
// ) -> Result<(), FormatError> {
//     let padlen = width.saturating_sub(text.len());
//     if left {
//         writer.write_all(text)?;
//         write!(writer, "{: <padlen$}", "")
//     } else {
//         write!(writer, "{: >padlen$}", "")?;
//         writer.write_all(text)
//     }
//     .map_err(FormatError::IoError)
// }
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
// 该函数检查当前字符是否为星号 (*)，如果是，则吃掉星号并返回一个 CanAsterisk::Asterisk 值。这意味着宽度或精度值由参数决定。
//
// 如果当前字符不是星号，函数会调用 eat_number 函数解析数字，并返回一个包含解析数字的 CanAsterisk::Fixed 值。
// fn eat_asterisk_or_number(rest: &mut &[u8], index: &mut usize) -> Option<CanAsterisk<usize>> {
//     if let Some(b'*') = rest.get(*index) {
//         *index += 1;
//         Some(CanAsterisk::Asterisk)
//     } else {
//         eat_number(rest, index).map(CanAsterisk::Fixed)
//     }
// }
fn eat_asterisk_or_number(rest: &mut &[u8], index: &mut usize) -> Option<CanAsterisk<usize>> {
    // 检查`rest`是否为空，避免因索引访问而导致的panic
    if rest.is_empty() {
        return None; // 早期返回None，表示无法继续处理
    }

    // 使用match替换if let来提高代码的可读性
    match rest.get(*index) {
        Some(b'*') => {
            *index += 1;
            Some(CanAsterisk::Asterisk)
        }
        _ => {
            // 调用`eat_number`来尝试解析一个数字
            eat_number(rest, index).map(CanAsterisk::Fixed)
        }
    }
}

/**
 * 从字节切片中解析数字，并更新解析位置。
 * eat_number 函数是用于解析数字的辅助函数。它在输入片段中查找第一个非数字字符，并尝试解析到该字符为止的数字。如果解析成功，则返回解析后的数字，否则返回 None。
 * 此函数会从给定的字节切片中，从指定的位置开始寻找数字，并解析为 usize 类型。
 * 一旦遇到非数字字符，即停止解析，并返回解析得到的数字以及更新后的索引位置。
 *
 * @param rest 待解析的字节切片的引用。
 * @param index 当前解析的位置索引的引用，函数会更新此索引至解析结束的位置。
 * @return 返回一个选项，如果解析成功，则为包含解析结果的 Some(usize)，
 *         如果无法解析数字（例如，没有数字或解析过程中发生溢出），则为 None。
 */
// fn eat_number(rest: &mut &[u8], index: &mut usize) -> Option<usize> {
//     match rest[*index..].iter().position(|b| !b.is_ascii_digit()) {
//         None | Some(0) => None,
//         Some(i) => {
//             // TODO: This might need to handle errors better
//             // For example in case of overflow.
//             let parsed = std::str::from_utf8(&rest[*index..(*index + i)])
//                 .unwrap()
//                 .parse()
//                 .unwrap();
//             *index += i;
//             Some(parsed)
//         }
//     }
// }
fn eat_number(rest: &mut &[u8], index: &mut usize) -> Option<usize> {
    // 查找第一个非数字字符的位置
    match rest[*index..].iter().position(|b| !b.is_ascii_digit()) {
        Some(0) => None,
        None => None,
        Some(i) => {
            // 尝试从字节切片中解析数字，改进错误处理
            let slice = &rest[*index..(*index + i)];
            match std::str::from_utf8(slice) {
                Ok(str_slice) => {
                    match str_slice.parse() {
                        Ok(parsed) => {
                            *index += i;
                            Some(parsed)
                        }
                        Err(_) => {
                            // 解析错误，返回None
                            None
                        }
                    }
                }
                Err(_) => {
                    // 字符串不是有效的UTF-8，返回None
                    None
                }
            }
        }
    }
}

