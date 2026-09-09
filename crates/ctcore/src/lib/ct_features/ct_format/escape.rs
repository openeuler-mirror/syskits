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

// 转义序列的解析

#[derive(Debug, PartialEq)]
pub enum EscapedChar {
    /// 单个字节
    Byte(u8),
    /// 一个 Unicode 字符
    Char(char),
    /// 完整解析且大于 Unicode 最大标量值的代码点
    Unicode(u32),
    /// 前缀带反斜杠的字符（即无效的转义序列）
    Backslash(u8),
    /// 指定字符串应停止(\c)
    End,
}

#[repr(u8)]
#[derive(Clone, Copy)]
enum Base {
    Oct = 8,
    Hex = 16,
}

impl Base {
    // 所选代码是 Base 枚举的一部分，Base 枚举在所提供的 Rust 代码中定义。
    // Base 枚举有两种变体： 八进制和十六进制。max_digits 函数是 Base 枚举的一个方法，用于返回枚举变体可表示的最大位数。
    // 在本例中，max_digits 函数是作为匹配表达式实现的。匹配表达式检查 Base 枚举的变体，并返回相应的最大位数。
    fn max_digits(&self) -> u8 {
        match self {
            Self::Oct => 3,
            Self::Hex => 2,
        }
    }

    // convert_digit 函数的输入参数是一个单字节 c 和一个枚举 Self。
    // enum Self 是一个枚举，有两种变体： 八进制和十六进制。
    // 如果字节 c 是指定基数（八进制或十六进制）的有效数字，函数将返回一个 Option<u8>，表示该字节的数值。
    fn convert_digit(&self, c: u8) -> Option<u8> {
        match self {
            Self::Oct => {
                if (b'0'..=b'7').contains(&c) {
                    Some(c - b'0')
                } else {
                    None
                }
            }
            Self::Hex => {
                if c.is_ascii_digit() {
                    Some(c - b'0')
                } else if (b'A'..=b'F').contains(&c) {
                    Some(c - b'A' + 10)
                } else if (b'a'..=b'f').contains(&c) {
                    Some(c - b'a' + 10)
                } else {
                    None
                }
            }
        }
    }
}

// 函数负责解析字符串中的转义序列。
// parse_code 函数是一个辅助函数，专门解析 \xHHH 和 \0NNN 转义序列的数字部分。
// parse_code 函数有两个参数：输入和基数。
// 输入参数是对代表输入字符串的字节片段的可变引用。
// base 参数是一个枚举，表示转义序列数字部分的基数。
// 它可以是 Oct（用于八进制序列）或 Hex（用于十六进制序列）。

fn parse_code(input: &mut &[u8], base: Base) -> Option<u8> {
    // 对ret的所有算术运算都需要进行包裹处理，因为八进制输入可以包含3位数字，即9位，
    // 因此超过了u8所能容纳的范围。
    // GNU似乎只是简单地将这些值进行了包裹处理。
    // 注意，如果我们改为将ret设为u32并使用char::from_u32将会得到错误的结果，
    // 因为它会将大于u8::MAX的值解释为Unicode字符。
    if let [c, rest @ ..] = input {
        let mut ret = base.convert_digit(*c)?;
        *input = rest;

        for _ in 1..base.max_digits() {
            if let [c, rest @ ..] = input {
                if let Some(n) = base.convert_digit(*c) {
                    ret = ret.wrapping_mul(base as u8).wrapping_add(n);
                    *input = rest;
                } else {
                    break;
                };
            } else {
                break;
            };
        }

        Some(ret)
    } else {
        None
    }
}

// Parse the exact digit count required by \u and \U while retaining values
// above the Unicode range for GNU's ASCII fallback representation.
fn parse_unicode_escape(input: &mut &[u8], digits: u8, prefix: u8) -> Result<u32, FormatError> {
    let mut value = 0u32;
    for _ in 0..digits {
        let (byte, rest) = input
            .split_first()
            .ok_or(FormatError::MissingHexadecimalNumber)?;
        let digit = Base::Hex
            .convert_digit(*byte)
            .ok_or(FormatError::MissingHexadecimalNumber)?;
        value = value.wrapping_mul(16).wrapping_add(u32::from(digit));
        *input = rest;
    }

    if (0xD800..=0xDFFF).contains(&value) {
        return Err(FormatError::InvalidUniversalCharacterName { prefix, value });
    }
    Ok(value)
}

#[cfg(test)]
fn parse_unicode(input: &mut &[u8], digits: u8) -> Option<char> {
    parse_unicode_escape(input, digits, b'u')
        .ok()
        .and_then(char::from_u32)
}

use super::FormatError;

// parse_escape_code 将字节片段（&mut [u8]）的可变引用作为输入，并返回一个 EscapedChar 枚举。
// 该函数负责解析字符串中的转义序列。
pub fn parse_escape_code(rest: &mut &[u8], is_b_format: bool) -> Result<EscapedChar, FormatError> {
    if let [c, new_rest @ ..] = rest {
        if is_b_format && *c == b'0' {
            // 对于 %b，\0 后面最多跟 3 位八进制数字
            *rest = new_rest;
            match parse_code(rest, Base::Oct) {
                Some(val) => return Ok(EscapedChar::Byte(val)),
                None => return Ok(EscapedChar::Byte(b'\0')),
            }
        } else if let b'0'..=b'7' = c {
            // 普通格式化字符串，\NNN 包括首位在内最多 3 位
            if let Some(parsed) = parse_code(rest, Base::Oct) {
                return Ok(EscapedChar::Byte(parsed));
            }
        }

        *rest = new_rest;
        match c {
            b'\\' => Ok(EscapedChar::Byte(b'\\')),
            b'"' => Ok(EscapedChar::Byte(b'"')),
            b'a' => Ok(EscapedChar::Byte(b'\x07')),
            b'b' => Ok(EscapedChar::Byte(b'\x08')),
            b'c' => Ok(EscapedChar::End),
            b'e' => Ok(EscapedChar::Byte(b'\x1b')),
            b'f' => Ok(EscapedChar::Byte(b'\x0c')),
            b'n' => Ok(EscapedChar::Byte(b'\n')),
            b'r' => Ok(EscapedChar::Byte(b'\r')),
            b't' => Ok(EscapedChar::Byte(b'\t')),
            b'v' => Ok(EscapedChar::Byte(b'\x0b')),
            b'x' => match parse_code(rest, Base::Hex) {
                Some(c) => Ok(EscapedChar::Byte(c)),
                None => Err(FormatError::MissingHexadecimalNumber),
            },
            b'u' => parse_unicode_escape(rest, 4, b'u').map(|value| {
                char::from_u32(value)
                    .map(EscapedChar::Char)
                    .unwrap_or(EscapedChar::Unicode(value))
            }),
            b'U' => parse_unicode_escape(rest, 8, b'U').map(|value| {
                char::from_u32(value)
                    .map(EscapedChar::Char)
                    .unwrap_or(EscapedChar::Unicode(value))
            }),
            c => Ok(EscapedChar::Backslash(*c)),
        }
    } else {
        Ok(EscapedChar::Byte(b'\\'))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_convert_digit_octal() {
        let result = Base::Oct.convert_digit(b'1');
        assert_eq!(result, Some(1));

        let result = Base::Oct.convert_digit(b'2');
        assert_eq!(result, Some(2));

        let result = Base::Oct.convert_digit(b'3');
        assert_eq!(result, Some(3));

        let result = Base::Oct.convert_digit(b'0');
        assert_eq!(result, Some(0));

        let result = Base::Oct.convert_digit(b'4');
        assert_eq!(result, Some(4));

        let result = Base::Oct.convert_digit(b'5');
        assert_eq!(result, Some(5));

        let result = Base::Oct.convert_digit(b'6');
        assert_eq!(result, Some(6));

        let result = Base::Oct.convert_digit(b'7');
        assert_eq!(result, Some(7));
        let result = Base::Oct.convert_digit(b'D');
        assert_eq!(result, None);

        let result = Base::Oct.convert_digit(b'E');
        assert_eq!(result, None);

        let result = Base::Oct.convert_digit(b'F');
        assert_eq!(result, None);
    }

    #[test]
    fn test_convert_digit_hexadecimal() {
        let result = Base::Hex.convert_digit(b'1');
        assert_eq!(result, Some(1));

        let result = Base::Hex.convert_digit(b'8');
        assert_eq!(result, Some(8));

        let result = Base::Hex.convert_digit(b'9');
        assert_eq!(result, Some(9));

        let result = Base::Hex.convert_digit(b'A');
        assert_eq!(result, Some(10));

        let result = Base::Hex.convert_digit(b'B');
        assert_eq!(result, Some(11));

        let result = Base::Hex.convert_digit(b'C');
        assert_eq!(result, Some(12));

        let result = Base::Hex.convert_digit(b'D');
        assert_eq!(result, Some(13));

        let result = Base::Hex.convert_digit(b'E');
        assert_eq!(result, Some(14));

        let result = Base::Hex.convert_digit(b'F');
        assert_eq!(result, Some(15));

        let result = Base::Hex.convert_digit(b'0');
        assert_eq!(result, Some(0));

        let result = Base::Hex.convert_digit(b'G');
        assert_eq!(result, None);

        let result = Base::Hex.convert_digit(b'H');
        assert_eq!(result, None);

        let result = Base::Hex.convert_digit(b'I');
        assert_eq!(result, None);

        let result = Base::Hex.convert_digit(b'J');
        assert_eq!(result, None);
    }
    #[test]
    fn test_max_digits_octal() {
        assert_eq!(Base::Oct.max_digits(), 3);
    }

    #[test]
    fn test_max_digits_hexadecimal() {
        assert_eq!(Base::Hex.max_digits(), 2);
    }

    #[test]
    fn test_parse_code_octal_1() {
        let mut input: &[u8] = b"1";
        assert_eq!(parse_code(&mut input, Base::Oct), Some(1));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_code_octal_2() {
        let mut input: &[u8] = b"12";
        assert_eq!(parse_code(&mut input, Base::Oct), Some(10));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_code_octal_3() {
        let mut input: &[u8] = b"123";
        assert_eq!(parse_code(&mut input, Base::Oct), Some(83));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_code_octal_4() {
        let mut input: &[u8] = b"1234";
        assert_eq!(parse_code(&mut input, Base::Oct), Some(83));
        assert_eq!(input, b"4");
    }

    #[test]
    fn test_parse_code_hex_1() {
        let mut input: &[u8] = b"a";
        assert_eq!(parse_code(&mut input, Base::Hex), Some(10));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_code_hex_2() {
        let mut input: &[u8] = b"12";
        assert_eq!(parse_code(&mut input, Base::Hex), Some(18));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_code_hex_3() {
        let mut input: &[u8] = b"123";
        assert_eq!(parse_code(&mut input, Base::Hex), Some(18));
        assert_eq!(input, b"3");
    }

    #[test]
    fn test_parse_code_hex_4() {
        let mut input: &[u8] = b"1234";
        assert_eq!(parse_code(&mut input, Base::Hex), Some(18));
        assert_eq!(input, b"34");
    }

    #[test]
    fn test_parse_code_invalid_input_length() {
        let mut input: &[u8] = b"x";
        assert_eq!(parse_code(&mut input, Base::Hex), None);
        assert_eq!(input, b"x");
    }

    #[test]
    fn test_parse_code_invalid_input_length_max() {
        let mut input: &[u8] = b"x{10FFFF}";
        assert_eq!(parse_code(&mut input, Base::Hex), None);
        assert_eq!(input, b"x{10FFFF}");
    }

    #[test]
    fn test_parse_code_invalid_input_length_min() {
        let mut input: &[u8] = b"x";
        assert_eq!(parse_code(&mut input, Base::Hex), None);
        assert_eq!(input, b"x");
    }

    #[test]
    fn test_parse_code_invalid_input_length_min_hex() {
        let mut input: &[u8] = b"x";
        assert_eq!(parse_code(&mut input, Base::Hex), None);
        assert_eq!(input, b"x");
    }

    #[test]
    fn test_parse_code_invalid_input_length_max_hex() {
        let mut input: &[u8] = b"x{10FFFF}";
        assert_eq!(parse_code(&mut input, Base::Hex), None);
        assert_eq!(input, b"x{10FFFF}");
    }

    #[test]
    fn test_parse_code_invalid_input_length_max_octal() {
        let mut input: &[u8] = b"123456789";
        assert_eq!(parse_code(&mut input, Base::Oct), Some(83));
        assert_eq!(input, b"456789");
    }

    #[test]
    fn test_parse_code_invalid_input_length_min_octal() {
        let mut input: &[u8] = b"1";
        assert_eq!(parse_code(&mut input, Base::Oct), Some(1));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_code_invalid_input_length_max_octal_2() {
        let mut input: &[u8] = b"12345678";
        assert_eq!(parse_code(&mut input, Base::Oct), Some(83));
        assert_eq!(input, b"45678");
    }

    #[test]
    fn test_parse_code_invalid_input_length_min_hex_octal() {
        let mut input: &[u8] = b"x1";
        assert_eq!(parse_code(&mut input, Base::Hex), None);
        assert_eq!(input, b"x1");
    }

    #[test]
    fn test_parse_code_invalid_input_length_max_hex_octal() {
        let mut input: &[u8] = b"x{10FFFF}";
        assert_eq!(parse_code(&mut input, Base::Hex), None);
        assert_eq!(input, b"x{10FFFF}");
    }

    #[test]
    fn test_parse_code_invalid_input_length_min_hex_octal_invalid_hex() {
        let mut input: &[u8] = b"xZZ";
        assert_eq!(parse_code(&mut input, Base::Hex), None);
        assert_eq!(input, b"xZZ");
    }

    #[test]
    fn test_parse_code_invalid_input_length_max_hex_octal_invalid_hex() {
        let mut input: &[u8] = b"x{10FFFF}ZZ";
        assert_eq!(parse_code(&mut input, Base::Hex), None);
        assert_eq!(input, b"x{10FFFF}ZZ");
    }

    #[test]
    fn test_parse_code_invalid_input_length_min_hex_octal_invalid_octal() {
        let mut input: &[u8] = b"1ZZ";
        assert_eq!(parse_code(&mut input, Base::Oct), Some(1));
        assert_eq!(input, b"ZZ");
    }

    #[test]
    fn test_parse_code_invalid_input_length_max_hex_octal_invalid_octal() {
        let mut input: &[u8] = b"123ZZ";
        assert_eq!(parse_code(&mut input, Base::Oct), Some(83));
        assert_eq!(input, b"ZZ");
    }

    #[test]
    fn test_parse_unicode_empty_input() {
        let mut input: &[u8] = &[];
        assert_eq!(parse_unicode(&mut input, 4), None);
    }

    #[test]
    fn test_parse_unicode_invalid_hex_digits() {
        let mut input: &[u8] = b"uZZZZ";
        assert_eq!(parse_unicode(&mut input, 4), None);
    }

    #[test]
    fn test_parse_unicode_invalid_unicode_range() {
        let mut input: &[u8] = b"\\U0001F602";
        assert_eq!(parse_unicode(&mut input, 8), None);
    }

    #[test]
    fn test_parse_unicode_valid_unicode_range() {
        let mut input: &[u8] = b"0041";
        assert_eq!(parse_unicode(&mut input, 4), Some('\u{0041}'));
    }

    #[test]
    fn test_parse_unicode_valid_unicode_range2() {
        let mut input: &[u8] = b"004100";
        assert_eq!(parse_unicode(&mut input, 4), Some('\u{0041}'));
    }
    #[test]
    fn test_parse_unicode_valid_unicode_range4() {
        let mut input: &[u8] = b"004100";
        assert_eq!(parse_unicode(&mut input, 6), Some('\u{004100}'));
    }
    #[test]
    fn test_parse_unicode_valid_unicode_range3() {
        let mut input: &[u8] = b"004101";
        assert_eq!(parse_unicode(&mut input, 4), Some('\u{0041}'));
    }

    #[test]
    fn test_parse_unicode_valid_unicode_range5() {
        let mut input: &[u8] = b"004101";
        assert_eq!(parse_unicode(&mut input, 6), Some('\u{004101}'));
    }

    #[test]
    fn test_parse_unicode_valid_unicode_range_max() {
        let mut input: &[u8] = b"10FFFF";
        assert_eq!(parse_unicode(&mut input, 8), None);
    }

    #[test]
    fn test_parse_unicode_valid_unicode_range_max2() {
        let mut input: &[u8] = b"10FFFFF";
        assert_eq!(parse_unicode(&mut input, 8), None);
    }

    #[test]
    fn test_parse_unicode_invalid_input_length() {
        let mut input: &[u8] = b"u";
        assert_eq!(parse_unicode(&mut input, 4), None);
    }

    #[test]
    fn test_parse_unicode_invalid_input_length_max() {
        let mut input: &[u8] = b"u{10FFFF}";
        assert_eq!(parse_unicode(&mut input, 9), None);
    }

    #[test]
    fn test_parse_unicode_invalid_input_length_min() {
        let mut input: &[u8] = b"u";
        assert_eq!(parse_unicode(&mut input, 1), None);
    }

    #[test]
    fn test_parse_unicode_invalid_input_length_min_hex() {
        let mut input: &[u8] = b"\\u";
        assert_eq!(parse_unicode(&mut input, 1), None);
    }

    #[test]
    fn test_parse_unicode_invalid_input_length_min_unicode() {
        let mut input: &[u8] = b"\\U";
        assert_eq!(parse_unicode(&mut input, 1), None);
    }

    #[test]
    fn test_parse_unicode_invalid_input_length_max_hex() {
        let mut input: &[u8] = b"\\u{10FFFF}";
        assert_eq!(parse_unicode(&mut input, 10), None);
    }

    #[test]
    fn test_parse_unicode_invalid_input_length_max_unicode() {
        let mut input: &[u8] = b"\\U{10FFFF}";
        assert_eq!(parse_unicode(&mut input, 17), None);
    }

    #[test]
    fn test_parse_escape_code_octal() {
        let mut input: &[u8] = b"123";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'S')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_hex() {
        let mut input: &[u8] = b"x1F";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(0x1F)
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_hex_requires_digit() {
        let mut input: &[u8] = b"x";
        assert!(matches!(
            parse_escape_code(&mut input, false),
            Err(FormatError::MissingHexadecimalNumber)
        ));
        assert_eq!(input, b"");

        let mut input: &[u8] = b"xZ";
        assert!(matches!(
            parse_escape_code(&mut input, true),
            Err(FormatError::MissingHexadecimalNumber)
        ));
        assert_eq!(input, b"Z");
    }

    #[test]
    fn test_parse_escape_code_unicode() {
        let mut input: &[u8] = b"u0041";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Char('\u{0041}')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn unicode_escape_distinguishes_incomplete_surrogate_and_out_of_range_values() {
        let mut incomplete: &[u8] = b"u123";
        assert!(matches!(
            parse_escape_code(&mut incomplete, false),
            Err(FormatError::MissingHexadecimalNumber)
        ));

        let mut surrogate: &[u8] = b"uD800";
        assert!(matches!(
            parse_escape_code(&mut surrogate, false),
            Err(FormatError::InvalidUniversalCharacterName {
                prefix: b'u',
                value: 0xD800
            })
        ));

        let mut out_of_range: &[u8] = b"U00110000";
        assert_eq!(
            parse_escape_code(&mut out_of_range, false).unwrap(),
            EscapedChar::Unicode(0x0011_0000)
        );
        assert_eq!(out_of_range, b"");
    }

    #[test]
    fn test_parse_escape_code_invalid_unicode() {
        let mut input: &[u8] = b"uXXXX";
        assert!(matches!(
            parse_escape_code(&mut input, false),
            Err(FormatError::MissingHexadecimalNumber)
        ));
        assert_eq!(input, b"XXXX");
    }

    #[test]
    fn test_parse_escape_code_backslash() {
        let mut input: &[u8] = b"\\";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\\')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_double_quote() {
        let mut input: &[u8] = b"\"";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'"')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_control_char() {
        let mut input: &[u8] = b"a";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\x07')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_end() {
        let mut input: &[u8] = b"c";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::End
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_invalid_backslash() {
        let mut input: &[u8] = b"\\x";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(92)
        );
        assert_eq!(input, b"x");
    }

    #[test]
    fn test_parse_escape_code_form_feed() {
        let mut input: &[u8] = b"f";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\x0c')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_newline() {
        let mut input: &[u8] = b"n";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\n')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_carriage_return() {
        let mut input: &[u8] = b"r";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\r')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_horizontal_tab() {
        let mut input: &[u8] = b"t";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\t')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_vertical_tab() {
        let mut input: &[u8] = b"v";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\x0b')
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_invalid_octal() {
        let mut input: &[u8] = b"08";
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\0')
        );
        assert_eq!(input, b"8");
    }

    // Add more test cases for other escape sequences and edge cases...

    #[test]
    fn test_parse_escape_code_boundary_octal_max() {
        let mut input: &[u8] = b"777"; // Max octal value
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(255)
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_boundary_hex_max() {
        let mut input: &[u8] = b"xFF"; // Max hexadecimal value
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(255)
        );
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_boundary_unicode_max() {
        let mut input: &[u8] = b"u{10FFFF}"; // Max Unicode code point
        assert!(matches!(
            parse_escape_code(&mut input, false),
            Err(FormatError::MissingHexadecimalNumber)
        ));
        assert_eq!(input, b"{10FFFF}");
    }

    #[test]
    fn test_parse_escape_code_invalid_escape_sequence() {
        let mut input: &[u8] = b"\\xyz"; // Invalid escape sequence
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\\')
        );
        assert_eq!(input, b"xyz");
    }

    #[test]
    fn test_parse_escape_code_incomplete_escape_sequence() {
        let mut input: &[u8] = b"\\u123"; // Incomplete unicode escape sequence
        assert_eq!(
            parse_escape_code(&mut input, false).unwrap(),
            EscapedChar::Byte(b'\\')
        );
        assert_eq!(input, b"u123");
    }
}
