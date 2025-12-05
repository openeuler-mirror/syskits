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

//! Parsing of escape sequences

#[derive(Debug, PartialEq)]
pub enum EscapedChar {
    /// A single byte
    Byte(u8),
    /// A unicode character
    Char(char),
    /// A character prefixed with a backslash (i.e. an invalid escape sequence)
    Backslash(u8),
    /// Specifies that the string should stop (`\c`)
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
/// Parse the numeric part of the `\xHHH` and `\0NNN` escape sequences
fn parse_code(input: &mut &[u8], base: Base) -> Option<u8> {
    // All arithmetic on `ret` needs to be wrapping, because octal input can
    // take 3 digits, which is 9 bits, and therefore more than what fits in a
    // `u8`. GNU just seems to wrap these values.
    // Note that if we instead make `ret` a `u32` and use `char::from_u32` will
    // yield incorrect results because it will interpret values larger than
    // `u8::MAX` as unicode.
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

//所选代码是所提供 Rust 代码中 parse_unicode 函数的一部分。该函数负责解析形式为 \uHHHH 和 \UHHHHH 的 Unicode 转义序列。
//
// 下面是所选代码的明细：
// 1. 该函数接收两个参数：代表输入字符串的字节片段的可变引用（&mut [u8]），以及 Unicode 转义序列中的十六进制数字个数（u8）。
// 2. 函数首先使用 split_first()? 方法从输入片段中取出第一个字节。该字节代表 Unicode 转义序列的第一个十六进制数字。
// 3. 然后，函数使用 Base 枚举的 convert_digit()? 方法将第一个字节转换为相应的数值。该方法会检查字节是否为有效的十六进制数字，如果是则返回相应的数值（0-15），否则返回 None。
// 4. 然后，函数将返回的数值转换为 u32，并将其存储在 ret 变量中。
// 5. 然后，函数进入一个循环，迭代 Unicode 转义序列的剩余十六进制数字。在每次迭代中，函数都会从输入片段中取出下一个字节，使用 Base 枚举的 convert_digit()? 方法将其转换为相应的数值，并使用位运算将该数值乘加到 ret 变量中。
// 6. 循环结束后，函数最后使用 char::from_u32() 方法将数字值转换为 Unicode 字符并返回。
// TODO: This should print warnings and possibly halt execution when it fails to parse
// TODO: If the character cannot be converted to u32, the input should be printed.
fn parse_unicode(input: &mut &[u8], digits: u8) -> Option<char> {
    let (c, rest) = input.split_first()?;
    let mut ret = Base::Hex.convert_digit(*c)? as u32;
    *input = rest;

    for _ in 1..digits {
        let (c, rest) = input.split_first()?;
        let n = Base::Hex.convert_digit(*c)?;
        ret = ret.wrapping_mul(Base::Hex as u32).wrapping_add(n as u32);
        *input = rest;
    }

    // let mut ret = 0;
    // for (c, rest) in input.iter().take(digits - 1).enumerate() {
    //     let n = Base::Hex.convert_digit(*c)?;
    //     ret = ret.wrapping_mul(Base::Hex as u32).wrapping_add(n as u32);
    //     *input = rest;
    // }

    char::from_u32(ret)
}

// parse_escape_code 将字节片段（&mut [u8]）的可变引用作为输入，并返回一个 EscapedChar 枚举。
// 该函数负责解析字符串中的转义序列。
pub fn parse_escape_code(rest: &mut &[u8]) -> EscapedChar {
    if let [c, new_rest @ ..] = rest {
        // This is for the \NNN syntax for octal sequences.
        // Note that '0' is intentionally omitted because that
        // would be the \0NNN syntax.
        if let b'1'..=b'7' = c {
            let parse_value = parse_code(rest, Base::Oct);
            if let Some(parsed) = parse_value {
                return EscapedChar::Byte(parsed);
            }
        }

        *rest = new_rest;
        match c {
            b'\\' => EscapedChar::Byte(b'\\'),
            b'"' => EscapedChar::Byte(b'"'),
            b'a' => EscapedChar::Byte(b'\x07'),
            b'b' => EscapedChar::Byte(b'\x08'),
            b'c' => EscapedChar::End,
            b'e' => EscapedChar::Byte(b'\x1b'),
            b'f' => EscapedChar::Byte(b'\x0c'),
            b'n' => EscapedChar::Byte(b'\n'),
            b'r' => EscapedChar::Byte(b'\r'),
            b't' => EscapedChar::Byte(b'\t'),
            b'v' => EscapedChar::Byte(b'\x0b'),
            b'x' => match parse_code(rest, Base::Hex) {
                Some(c) => EscapedChar::Byte(c),
                None => EscapedChar::Backslash(b'x'),
            },
            b'0' => match parse_code(rest, Base::Oct) {
                Some(c) => EscapedChar::Byte(c),
                None => EscapedChar::Byte(b'\0'),
            },
            b'u' => match parse_unicode(rest, 4) {
                Some(c) => EscapedChar::Char(c),
                None => EscapedChar::Char('\0'),
            },
            b'U' => match parse_unicode(rest, 8) {
                Some(c) => EscapedChar::Char(c),
                None => EscapedChar::Char('\0'),
            },
            c => EscapedChar::Backslash(*c),
        }
    } else {
        EscapedChar::Byte(b'\\')
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
        assert_eq!(parse_escape_code(&mut input), EscapedChar::Byte(b'S'));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_hex() {
        let mut input: &[u8] = b"x1F";
        assert_eq!(parse_escape_code(&mut input), EscapedChar::Byte(0x1F));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_unicode() {
        let mut input: &[u8] = b"u0041";
        assert_eq!(parse_escape_code(&mut input), EscapedChar::Char('\u{0041}'));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_invalid_unicode() {
        let mut input: &[u8] = b"uXXXX";
        assert_eq!(parse_escape_code(&mut input), EscapedChar::Char('\0'));
        assert_eq!(input, b"XXXX");
    }

    #[test]
    fn test_parse_escape_code_backslash() {
        let mut input: &[u8] = b"\\";
        assert_eq!(parse_escape_code(&mut input), EscapedChar::Byte(b'\\'));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_double_quote() {
        let mut input: &[u8] = b"\"";
        assert_eq!(parse_escape_code(&mut input), EscapedChar::Byte(b'"'));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_control_char() {
        let mut input: &[u8] = b"a";
        assert_eq!(parse_escape_code(&mut input), EscapedChar::Byte(b'\x07'));
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_end() {
        let mut input: &[u8] = b"c";
        assert_eq!(parse_escape_code(&mut input), EscapedChar::End);
        assert_eq!(input, b"");
    }

    #[test]
    fn test_parse_escape_code_invalid_backslash() {
        let mut input: &[u8] = b"\\x";
        assert_eq!(parse_escape_code(&mut input), EscapedChar::Byte(92));
        assert_eq!(input, b"x");
    }
}