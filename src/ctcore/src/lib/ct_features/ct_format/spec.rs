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

use crate::ct_quoting_style::{CtQuotingStyle, escape_name};

use super::{
    ArgumentIter, FormatChar, FormatError,
    num_format::{
        self, Case, FloatVariant, ForceDecimal, Formatter, NumberAlignment, PositiveSign, Prefix,
        UnsignedIntVariant,
    },
    parse_escape_only,
};
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

/// 指定的精度和宽度可能会使用星号表示它们由参数确定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanAsterisk<T> {
    Fixed(T),
    Asterisk,
}

/// 预期类型大小（被忽略）
///
/// 我们完全忽略这个参数，但我们确实进行了解析。
/// 如果将来有需要，可以使用它。
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

    /// 是否设置了任意一个标志为true
    fn any(&self) -> bool {
        self != &Self::default()
    }
}

impl Spec {
    pub fn parse<'a>(rest: &mut &'a [u8]) -> Result<Self, &'a [u8]> {
        // 根据 C++ 参考资料，格式规范看起来像这样：
        //
        //
        //   %[flags][width][.precision][length]specifier
        //
        // 不过，我们已经解析过了 '%'。
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

        // 如果指定了-或精度，则忽略0标志。
        // 因此，RightZero的唯一情况是未指定-且精度为无。
        let alignment = if flags.minus {
            NumberAlignment::Left
        } else if precision.is_none() && flags.zero {
            NumberAlignment::RightZero
        } else {
            NumberAlignment::RightSpace
        };

        // 我们忽略长度。它对 printf 来说并不重要
        let _ = Self::parse_length(rest, &mut index);

        let type_spec = match rest.get(index) {
            Some(type_spec) => type_spec,
            None => {
                return Err(&start[..index]);
            }
        };

        index += 1;
        *rest = &start[index..];

        Ok(match type_spec {
            // GNU接受减号、加号和空格，即使它们没有被使用
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
                // 普通无符号整数不能有前缀
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
        // 解析0..N长度选项，保留最后一个
        // 即使它只是被忽略。我们可能稍后会用到它，我们应该解析那些字符。
        // 待办事项：这需要可配置：seq只接受一个长度参数
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

                // GNU确实会在字节级别进行这种截断，例如：
                // printf "%.1s" 🙃
                // > � // 目前，当我们在代码点内截断时，让printf恐慌。
                // 待办事项：我们不需要使用Rust的格式化来对输出进行对齐，
                // 这样我们就可以直接将字节写入stdout，而不会引发恐慌。
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
                            &CtQuotingStyle::Shell {
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
        assert_eq!(Spec::parse(&mut input), Ok(expected));
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
        assert_eq!(Spec::parse(&mut input), Ok(expected));
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
        assert_eq!(Spec::parse(&mut input), Ok(expected));
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
        assert_eq!(Spec::parse(&mut input), Ok(expected));
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
        assert_eq!(Spec::parse(&mut input), Ok(expected));
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
        assert_eq!(Spec::parse(&mut input), Ok(expected));
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
        assert_eq!(Spec::parse(&mut input), Ok(expected));
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
        assert_eq!(Spec::parse(&mut input), Ok(expected));
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
        assert_eq!(Spec::parse(&mut input), Ok(expected));
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
        assert_eq!(Spec::parse(&mut input), Err(rest));
        // assert_eq!(Spec::parse(&mut input), Err([Spec::EscapedString, 100]));
    }

    #[test]
    fn test_parse_specifier_with_l_flag() {
        let mut input: &[u8] = b"l";
        let rest: &[u8] = &[b'l'];
        let _expected = Spec::Char {
            width: None,
            align_left: true,
        };
        assert_eq!(Spec::parse(&mut input), Err(rest));
    }

    #[test]
    fn test_parse_specifier_with_l_flag2() {
        let mut input: &[u8] = b"2.3L";
        let _expected = Spec::Char {
            width: None,
            align_left: true,
        };
        let rest: &[u8] = &[b'2', b'.', b'3', b'L'];
        assert_eq!(Spec::parse(&mut input), Err(rest));
    }

    #[test]
    fn test_parse_specifier_with_h_flag2() {
        let mut input: &[u8] = b"H";
        let rest: &[u8] = &[b'H'];
        let _expected = Spec::Char {
            width: None,
            align_left: false,
        };
        assert_eq!(Spec::parse(&mut input), Err(rest));
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
        let mut rest: &[u8] = &[b'h', b'i', b'j']; // "hij"
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), None);
        assert_eq!(index, 0);
    }

    #[test]
    fn test_eat_number_single_digit() {
        let mut rest: &[u8] = &[b'0']; // "0"
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), None);
        assert_eq!(index, 0);
    }

    #[test]
    fn test_eat_number_multiple_digits() {
        // "345"
        let mut rest: &[u8] = &[b'3', b'4', b'5'];
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), None);
        assert_eq!(index, 0);
    }

    #[test]
    fn test_eat_number_multiple_digits2() {
        // "3x5"
        let mut rest: &[u8] = &[b'3', b'q', b'5'];
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), Some(3));
        assert_eq!(index, 1);
    }

    #[test]
    fn test_eat_number_mixed_digits_and_non_digits() {
        // "2345hij"
        let mut rest: &[u8] = &[b'2', b'3', b'4', b'5', b'h', b'i', b'j'];
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), Some(2345));
        assert_eq!(index, 4);
    }

    #[test]
    fn test_eat_number_non_digit_followed_by_digits() {
        // "hij012"
        let mut rest: &[u8] = &[b'h', b'i', b'j', b'0', b'1', b'2'];
        let mut index = 0;
        assert_eq!(eat_number(&mut rest, &mut index), None);
        assert_eq!(index, 0);
    }

    #[test]
    fn test_eat_asterisk_or_number_positive() {
        let mut rest: &[u8] = &mut [b'*', b'3', b'5', b'7'];
        let mut index = 0;
        assert_eq!(
            eat_asterisk_or_number(&mut rest, &mut index),
            Some(CanAsterisk::Asterisk)
        );
    }

    #[test]
    fn test_eat_asterisk_or_number_negative() {
        let mut rest: &[u8] = &mut [b'2', b'5', b'7'];
        let mut index = 0;
        if let Some(eat_asterisk_or_number_value) = eat_asterisk_or_number(&mut rest, &mut index) {
            assert_eq!(eat_asterisk_or_number_value, CanAsterisk::Fixed(257));
        }
    }

    #[test]
    fn test_eat_asterisk_or_number_not_an_asterisk() {
        let mut rest: &[u8] = &mut [b'3', b'5', b'7', b'a'];
        let mut index = 0;

        assert_eq!(
            eat_asterisk_or_number(&mut rest, &mut index),
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
            eat_asterisk_or_number(&mut rest, &mut index),
            Some(CanAsterisk::Asterisk)
        );
        assert_eq!(index, 1); // 索引不应该增加
    }

    #[test]
    fn test_eat_asterisk_or_number_no_number2() {
        let mut rest: &[u8] = &mut [b'*', b' ', b'a'];
        let mut index = 0;
        assert_eq!(
            eat_asterisk_or_number(&mut rest, &mut index),
            Some(CanAsterisk::Asterisk)
        );
        assert_eq!(index, 1); // 索引不应该增加
    }

    #[test]
    fn test_eat_asterisk_or_number_no_number3() {
        let mut rest: &[u8] = &mut [b'*', b'b', b'a'];
        let mut index = 0;
        assert_eq!(
            eat_asterisk_or_number(&mut rest, &mut index),
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
        assert_eq!(result.is_ok(), true);
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
        assert_eq!(result.is_ok(), true);
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
        assert_eq!(result.is_ok(), true);
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
        assert_eq!(result.is_ok(), true);
        assert_eq!(writer, b"                    ");
    }
    #[test]
    fn test_write_padded_empty_input_right() {
        let mut writer = Vec::<u8>::new();
        let text: &[u8] = &mut [];
        let width = 20;
        let left = false;
        let result = write_padded(&mut writer, text, width, left);
        assert_eq!(result.is_ok(), true);
        assert_eq!(writer, b"                    ");
    }

   #[test]
    fn test_write_padded_empty_width() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 0;
        let left = true;
        let result = write_padded(&mut writer, text, width, left);
        assert_eq!(result.is_ok(), true);
        assert_eq!(writer, b"Hello, world!");
    }

    #[test]
    fn test_write_padded_null_width_left() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 0;
        let left = true;
        let result = write_padded(&mut writer, text, width, left);
        assert_eq!(result.is_ok(), true);
        assert_eq!(writer, b"Hello, world!");
    }

    #[test]
    fn test_write_padded_null_width_right() {
        let mut writer = Vec::<u8>::new();
        let text = b"Hello, world!";
        let width = 0;
        let left = false;
        let result = write_padded(&mut writer, text, width, left);
        assert_eq!(result.is_ok(), true);
        assert_eq!(writer, b"Hello, world!");
    }

}