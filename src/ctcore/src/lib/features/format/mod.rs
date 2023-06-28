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

//! `printf`-style formatting
//!
//! Rust has excellent formatting capabilities, but the coreutils require very
//! specific formatting that needs to work exactly like the GNU utilities.
//! Naturally, the GNU behavior is based on the C `printf` functionality.
//!
//! Additionally, we need support for escape sequences for the `printf` utility.
//!
//! The [`printf`] and [`sprintf`] functions closely match the behavior of the
//! corresponding C functions: the former renders a formatted string
//! to stdout, the latter renders to a new [`String`] object.
//!
//! There are three kinds of parsing that we might want to do:
//!
//!  1. Parse only `printf` directives (for e.g. `seq`, `dd`)
//!  2. Parse only escape sequences (for e.g. `echo`)
//!  3. Parse both `printf` specifiers and escape sequences (for e.g. `printf`)
//!
//! This module aims to combine all three use cases. An iterator parsing each
//! of these cases is provided by [`parse_escape_only`], [`parse_spec_only`]
//! and [`parse_spec_and_escape`], respectively.
//!
//! There is a special [`Format`] type, which can be used to parse a format
//! string containing exactly one directive and does not use any `*` in that
//! directive. This format can be printed in a type-safe manner without failing
//! (modulo IO errors).

mod argument;
mod escape;
pub mod num_format;
pub mod num_parser;
mod spec;

pub use argument::*;
use spec::Spec;
use std::{
    error::Error,
    fmt::Display,
    io,
    io::{stdout, Write},
    ops::ControlFlow,
};

use crate::ct_error::UError;

use self::{
    escape::{parse_escape_code, EscapedChar},
    num_format::Formatter,
};

#[derive(Debug)]
pub enum FormatError {
    SpecError(Vec<u8>),
    IoError(std::io::Error),
    NoMoreArguments,
    InvalidArgument(FormatArgument),
    TooManySpecs(Vec<u8>),
    NeedAtLeastOneSpec(Vec<u8>),
    WrongSpecType,
}

impl Error for FormatError {}
impl UError for FormatError {}

impl From<std::io::Error> for FormatError {
    fn from(value: std::io::Error) -> Self {
        Self::IoError(value)
    }
}

// impl Display for FormatError {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         match self {
//             Self::SpecError(s) => write!(
//                 f,
//                 "%{}: invalid conversion specification",
//                 String::from_utf8_lossy(s)
//             ),
//             Self::TooManySpecs(s) => write!(
//                 f,
//                 "format '{}' has too many % directives",
//                 String::from_utf8_lossy(s)
//             ),
//             Self::NeedAtLeastOneSpec(s) => write!(
//                 f,
//                 "format '{}' has no % directive",
//                 String::from_utf8_lossy(s)
//             ),
//             // TODO: Error message below needs some work
//             Self::WrongSpecType => write!(f, "wrong % directive type was given"),
//             Self::IoError(_) => write!(f, "io error"),
//             Self::NoMoreArguments => write!(f, "no more arguments"),
//             Self::InvalidArgument(_) => write!(f, "invalid argument"),
//         }
//     }
// }
impl Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::SpecError(s) => write!(
                f,
                "%{}: invalid conversion specification",
                String::from_utf8_lossy(s)
            ),
            FormatError::TooManySpecs(s) => write!(
                f,
                "format '{}' has too many % directives",
                String::from_utf8_lossy(s)
            ),
            FormatError::NeedAtLeastOneSpec(s) => write!(
                f,
                "format '{}' has no % directive",
                String::from_utf8_lossy(s)
            ),
            FormatError::WrongSpecType => f.write_str("wrong % directive type was given"),
            FormatError::IoError(_) => f.write_str("io error"),
            FormatError::NoMoreArguments => f.write_str("no more arguments"),
            FormatError::InvalidArgument(_) => f.write_str("invalid argument"),
        }
    }
}

/// A single item to format
#[derive(Debug, PartialEq)]
pub enum FormatItem<C: FormatChar> {
    /// A format specifier
    Spec(Spec),
    /// A single character
    Char(C),
}

pub trait FormatChar {
    fn write(&self, writer: impl Write) -> io::Result<ControlFlow<()>>;
}

impl FormatChar for u8 {
    fn write(&self, mut writer: impl Write) -> io::Result<ControlFlow<()>> {
        // writer.write_all(&[*self])?;
        // Ok(ControlFlow::Continue(()))

        let byte_array = [*self]; // Create an array with a single element
        writer
            .write_all(&byte_array)
            .map(|_| ControlFlow::Continue(()))
    }
}

impl FormatChar for EscapedChar {
    fn write(&self, mut writer: impl Write) -> io::Result<ControlFlow<()>> {
        // match self {
        //     Self::Byte(c) => {
        //         writer.write_all(&[*c])?;
        //     }
        //     Self::Char(c) => {
        //         write!(writer, "{c}")?;
        //     }
        //     Self::Backslash(c) => {
        //         writer.write_all(&[b'\\', *c])?;
        //     }
        //     Self::End => return Ok(ControlFlow::Break(())),
        // }
        // Ok(ControlFlow::Continue(()))

        let result = match self {
            EscapedChar::Byte(c) => writer.write_all(&[*c]),
            EscapedChar::Char(c) => write!(writer, "{}", c),
            EscapedChar::Backslash(c) => writer.write_all(&[b'\\', *c]),
            EscapedChar::End => return Ok(ControlFlow::Break(())),
        };

        result.map(|_| ControlFlow::Continue(()))
    }
}

impl<C: FormatChar> FormatItem<C> {
    pub fn write<'a>(
        &self,
        writer: impl Write,
        args: &mut impl Iterator<Item = &'a FormatArgument>,
    ) -> Result<ControlFlow<()>, FormatError> {
        // match self {
        //     Self::Spec(spec) => spec.write(writer, args)?,
        //     Self::Char(c) => return c.write(writer).map_err(FormatError::IoError),
        // };
        // Ok(ControlFlow::Continue(()))

        match self {
            Self::Spec(spec) => {
                spec.write(writer, args).map_err(FormatError::from)?;
            }
            Self::Char(c) => {
                c.write(writer).map_err(FormatError::IoError)?;
                return Ok(ControlFlow::Continue(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

/// Parse a format string containing % directives and escape sequences
pub fn parse_spec_and_escape(
    fmt: &[u8],
) -> impl Iterator<Item = Result<FormatItem<EscapedChar>, FormatError>> + '_ {
    // let mut current = fmt;
    // std::iter::from_fn(move || match current {
    //     [] => None,
    //     [b'%', b'%', rest @ ..] => {
    //         current = rest;
    //         Some(Ok(FormatItem::Char(EscapedChar::Byte(b'%'))))
    //     }
    //     [b'%', rest @ ..] => {
    //         current = rest;
    //         let spec = match Spec::parse(&mut current) {
    //             Ok(spec) => spec,
    //             Err(slice) => return Some(Err(FormatError::SpecError(slice.to_vec()))),
    //         };
    //         Some(Ok(FormatItem::Spec(spec)))
    //     }
    //     [b'\\', rest @ ..] => {
    //         current = rest;
    //         Some(Ok(FormatItem::Char(parse_escape_code(&mut current))))
    //     }
    //     [c, rest @ ..] => {
    //         current = rest;
    //         Some(Ok(FormatItem::Char(EscapedChar::Byte(*c))))
    //     }
    // })

    let mut current = fmt;
    std::iter::from_fn(move || {
        if current.is_empty() {
            return None;
        }

        let next_char = current[0];
        let mut advance_by = 1; // Default advance by 1 byte

        let item = if next_char == b'%' {
            if current.len() > 1 && current[1] == b'%' {
                // Handle "%%" -> '%'
                advance_by = 2; // Skip both '%' characters
                Ok(FormatItem::Char(EscapedChar::Byte(b'%')))
            } else {
                // Handle format specifier
                let (result, used) = match Spec::parse(&mut &current[1..]) {
                    Ok(spec) => (Ok(FormatItem::Spec(spec)), 1),
                    Err(err) => (Err(FormatError::SpecError(err.to_vec())), 1), // Adjust the error handling as needed
                };
                advance_by += used; // Advance past the specifier
                result
            }
        } else if next_char == b'\\' {
            // Handle escape sequence
            let escaped_char = parse_escape_code(&mut &current[1..]);
            advance_by = 2; // Skip the '\' and the character after it
            Ok(FormatItem::Char(escaped_char))
        } else {
            // Handle regular character
            Ok(FormatItem::Char(EscapedChar::Byte(next_char)))
        };

        current = &current[advance_by..]; // Advance the current slice
        Some(item)
    })
}

/// Parse a format string containing % directives
pub fn parse_spec_only(
    fmt: &[u8],
) -> impl Iterator<Item = Result<FormatItem<u8>, FormatError>> + '_ {
    let mut current_ptr = fmt; // Current slice of the format string being processed
    std::iter::from_fn(move || {
        if current_ptr.is_empty() {
            // End of the format string
            None
        } else if current_ptr.starts_with(b"%%") {
            // Literal percent sign "%%" found
            current_ptr = &current_ptr[2..]; // Skip past the literal percent sign
            Some(Ok(FormatItem::Char(b'%'))) // Return a literal percent as a character
        } else if let [b'%', rest @ ..] = current_ptr {
            // Format specifier starting with '%' found
            current_ptr = rest; // Move past the '%'
            let spec_result = Spec::parse(&mut current_ptr); // Attempt to parse the specifier
            match spec_result {
                Ok(spec) => Some(Ok(FormatItem::Spec(spec))), // Successfully parsed specifier
                Err(slice) => Some(Err(FormatError::SpecError(slice.to_vec()))), // Error parsing specifier
            }
        } else {
            // Regular character found
            let (&first_byte, rest) = current_ptr.split_first().unwrap(); // Safely split off the first byte
            current_ptr = rest; // Update the pointer to the rest of the string
            Some(Ok(FormatItem::Char(first_byte))) // Return the regular character
        }
    })
}

/// Parse a format string containing escape sequences
pub fn parse_escape_only(fmt: &[u8]) -> impl Iterator<Item = EscapedChar> + '_ {
    // let mut current = fmt;
    // std::iter::from_fn(move || match current {
    //     [] => None,
    //     [b'\\', rest @ ..] => {
    //         current = rest;
    //         Some(parse_escape_code(&mut current))
    //     }
    //     [c, rest @ ..] => {
    //         current = rest;
    //         Some(EscapedChar::Byte(*c))
    //     }
    // })
    let mut current_slice = fmt;
    std::iter::from_fn(move || {
        if current_slice.is_empty() {
            None // End of input, stop the iterator
        } else if let [b'\\', rest @ ..] = current_slice {
            // If the current slice starts with an escape character '\'
            current_slice = rest; // Move past the escape character for next iteration
            let escaped_character = parse_escape_code(&mut current_slice); // Parse the escape code starting from the next character
            Some(escaped_character) // Return the parsed escape character
        } else {
            // If the current slice does not start with an escape character
            let (first_byte, rest) = current_slice.split_first().unwrap(); // Safely unwrap because slice is not empty
            current_slice = rest; // Update the slice to exclude the processed character
            Some(EscapedChar::Byte(*first_byte)) // Return the current character as is
        }
    })
}
/// Write a formatted string to stdout.
///
/// `format_string` contains the template and `args` contains the
/// arguments to render into the template.
///
/// See also [`sprintf`], which creates a new formatted [`String`].
///
/// # Examples
///
/// ```rust
/// use ctcore::format::{printf, FormatArgument};
///
/// printf("hello %s", &[FormatArgument::String("world".into())]).unwrap();
/// // prints "hello world"
/// ```
pub fn printf<'a>(
    format_str: impl AsRef<[u8]>,
    args: impl IntoIterator<Item = &'a FormatArgument>,
) -> Result<(), FormatError> {
    printf_writer(stdout(), format_str, args)
}

fn printf_writer<'a>(
    mut writer: impl Write,
    format_str: impl AsRef<[u8]>,
    args: impl IntoIterator<Item = &'a FormatArgument>,
) -> Result<(), FormatError> {
    let format_data = format_str.as_ref();
    let mut args_iter = args.into_iter();

    // Iterate through the parsed format items
    for format_item in parse_spec_only(format_data) {
        let item = format_item?;
        item.write(&mut writer, &mut args_iter)?;
    }

    Ok(())
}

/// Create a new formatted string.
///
/// `format_string` contains the template and `args` contains the
/// arguments to render into the template.
///
/// See also [`printf`], which prints to stdout.
///
/// # Examples
///
/// ```rust
/// use ctcore::format::{sprintf, FormatArgument};
///
/// let s = sprintf("hello %s", &[FormatArgument::String("ctyunos".into())]).unwrap();
/// let s = std::str::from_utf8(&s).unwrap();
/// assert_eq!(s, "hello ctyunos");
/// ```
pub fn sprintf<'a>(
    format_str: impl AsRef<[u8]>,
    args: impl IntoIterator<Item = &'a FormatArgument>,
) -> Result<Vec<u8>, FormatError> {
    let mut writer = Vec::new();
    printf_writer(&mut writer, format_str, args)?;
    Ok(writer)
}

/// A parsed format for a single float value
///
/// This is used by `seq`. It can be constructed with [`Format::parse`]
/// and can write a value with [`Format::fmt`].
///
/// It can only accept a single specification without any asterisk parameters.
/// If it does get more specifications, it will return an error.
pub struct Format<F: Formatter> {
    prefix: Vec<u8>,
    suffix: Vec<u8>,
    formatter: F,
}

impl<F: Formatter> Format<F> {
    pub fn parse(format_str: impl AsRef<[u8]>) -> Result<Self, FormatError> {
        // 将输入转换为字节切片
        let bytes = format_str.as_ref();

        // 分离格式化指令和文本
        let mut prefix = Vec::new();
        let mut spec = None;
        let mut suffix = Vec::new();

        let mut parsing_prefix = true;

        for item in parse_spec_only(bytes) {
            let item = item?;
            match item {
                FormatItem::Spec(s) if parsing_prefix => {
                    spec = Some(s);
                    parsing_prefix = false; // Once a spec is found, switch to parsing suffix
                }
                FormatItem::Spec(_) => {
                    // 如果找到了第二个格式化指令
                    return Err(FormatError::TooManySpecs(bytes.to_vec()));
                }
                FormatItem::Char(c) => {
                    if parsing_prefix {
                        prefix.push(c);
                    } else {
                        suffix.push(c);
                    }
                }
            }
        }

        // 检查是否至少发现了一个格式化指令
        let spec = spec.ok_or_else(|| FormatError::NeedAtLeastOneSpec(bytes.to_vec()))?;

        // 尝试从格式化指令创建 Formatter
        let formatter = F::try_from_spec(spec)?;

        // 构建并返回 Format 实例
        Ok(Self {
            prefix,
            suffix,
            formatter,
        })
    }

    //该函数是一个格式化输出函数，它将指定的内容按照一定的格式写入到给定的写入器中。函数接受三个参数：self、mut w和f，
    // 其中self是对象自身，mut w是实现了Write trait的写入器，f是格式化参数。
    // 函数首先将self.prefix写入到w中，然后调用self.formatter.fmt方法将格式化后的内容写入到w中，
    // 最后将self.suffix写入到w中，并返回Ok(())表示操作成功。
    pub fn fmt(&self, mut w: impl Write, f: F::Input) -> io::Result<()> {
        // w.write_all(&self.prefix)?;
        // self.formatter.fmt(&mut w, f)?;
        // w.write_all(&self.suffix)?;
        // Ok(())
        // 尝试写入前缀，并在失败时提供具体的错误上下文
        w.write_all(&self.prefix)
            .map_err(|e| io::Error::new(e.kind(), format!("Failed to write prefix: {}", e)))?;

        // 调用 formatter 来处理主体格式化，同样提供错误上下文
        self.formatter
            .fmt(&mut w, f)
            .map_err(|e| io::Error::new(e.kind(), format!("Failed to format content: {}", e)))?;

        // 尝试写入后缀，并处理可能的错误
        w.write_all(&self.suffix)
            .map_err(|e| io::Error::new(e.kind(), format!("Failed to write suffix: {}", e)))?;

        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as FmtWrite;
    use std::io::Cursor; // Import the Write trait from std::fmt for String
    struct MockFormatter;

    impl Formatter for MockFormatter {
        type Input = i32;

        fn try_from_spec(_spec: Spec) -> Result<Self, FormatError> {
            Ok(MockFormatter)
        }

        fn fmt(&self, mut w: impl Write, f: Self::Input) -> io::Result<()> {
            write!(w, "{}", f)
        }
    }

    #[test]
    fn test_spec_error_display() {
        let error = FormatError::SpecError(vec![b'a']);
        let mut output = String::new();
        write!(output, "{}", error).unwrap();
        assert_eq!(output, "%a: invalid conversion specification");
    }

    #[test]
    fn test_too_many_specs_display() {
        let error = FormatError::TooManySpecs(vec![b'f', b'o', b'o']);
        let mut output = String::new();
        write!(output, "{}", error).unwrap();
        assert_eq!(output, "format 'foo' has too many % directives");
    }

    #[test]
    fn test_need_at_least_one_spec_display() {
        let error = FormatError::NeedAtLeastOneSpec(vec![b'b', b'a', b'r']);
        let mut output = String::new();
        write!(output, "{}", error).unwrap();
        assert_eq!(output, "format 'bar' has no % directive");
    }

    #[test]
    fn test_wrong_spec_type_display() {
        let error = FormatError::WrongSpecType;
        let mut output = String::new();
        write!(output, "{}", error).unwrap();
        assert_eq!(output, "wrong % directive type was given");
    }

    #[test]
    fn test_io_error_display() {
        let error = FormatError::IoError(io::Error::new(io::ErrorKind::Other, "test"));
        let mut output = String::new();
        write!(output, "{}", error).unwrap();
        assert_eq!(output, "io error");
    }

    #[test]
    fn test_no_more_arguments_display() {
        let error = FormatError::NoMoreArguments;
        let mut output = String::new();
        write!(output, "{}", error).unwrap();
        assert_eq!(output, "no more arguments");
    }

    #[test]
    fn test_invalid_argument_display() {
        let error = FormatError::InvalidArgument(FormatArgument::String("example".into()));
        let mut output = String::new();
        write!(output, "{}", error).unwrap();
        assert_eq!(output, "invalid argument");
    }

    #[test]
    fn test_sprintf() {
        let format_string = "Hello, %s!";
        let args = vec![FormatArgument::String("World".into())];
        let result = sprintf(format_string, &args).unwrap();
        let result_str = String::from_utf8(result).unwrap();
        assert_eq!(result_str, "Hello, World!");
    }
    #[test]
    fn test_printf_writer() {
        let mut output = Vec::new();
        let format_string = "Hello, %s!";
        let args = vec![FormatArgument::String("Rust".into())];
        printf_writer(&mut output, format_string, &args).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "Hello, Rust!");
    }

    #[test]
    fn test_parse_escape_only() {
        let format_string = b"Hello, \\nWorld!";
        let result: Vec<_> = parse_escape_only(format_string).collect();
        assert_eq!(
            result,
            vec![
                EscapedChar::Byte(b'H'),
                EscapedChar::Byte(b'e'),
                EscapedChar::Byte(b'l'),
                EscapedChar::Byte(b'l'),
                EscapedChar::Byte(b'o'),
                EscapedChar::Byte(b','),
                EscapedChar::Byte(b' '),
                EscapedChar::Byte(b'\n'),
                EscapedChar::Byte(b'W'),
                EscapedChar::Byte(b'o'),
                EscapedChar::Byte(b'r'),
                EscapedChar::Byte(b'l'),
                EscapedChar::Byte(b'd'),
                EscapedChar::Byte(b'!'),
            ]
        );
    }
    #[test]
    fn test_parse_spec_only() {
        let format_string = b"%s %d";
        let result: Vec<_> = parse_spec_only(format_string).map(|r| r.unwrap()).collect();
        assert_eq!(result.len(), 3); // 验证结果中有两个 FormatItem，一个是 %s，另一个是 %d
    }
    #[test]
    fn test_parse_spec_and_escape() {
        let format_string = b"Hello, %s\\n";
        let result: Vec<_> = parse_spec_and_escape(format_string)
            .map(|r| r.unwrap())
            .collect();

        assert!(matches!(
            result.last(),
            Some(FormatItem::Char(EscapedChar::Byte(10)))
        ));
        // 确保正确解析 %s 指令和 \n 转义序列
    }

    #[test]
    fn test_sprintf_empty_format() {
        let format_string = "";
        let args = vec![];
        let result = sprintf(format_string, &args).unwrap();
        assert!(result.is_empty(), "空格式字符串应该返回空字符串");
    }
    #[test]
    fn test_sprintf_too_many_args() {
        let format_string = "%s";
        let args = vec![
            FormatArgument::String("One".into()),
            FormatArgument::String("Two".into()),
        ];
        // 假设对于额外的参数，函数设计是忽略它们，那么测试预期是成功的
        // 如果设计是报错，则需要修改测试预期
        let result = sprintf(format_string, &args).unwrap();
        assert_eq!(
            String::from_utf8(result).unwrap(),
            "One",
            "多余的参数应该被忽略"
        );
    }

    #[test]
    fn test_printf_writer_io_error() {
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::Other, "失败的写入"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let format_string = "Hello";
        let args = vec![];
        let result = printf_writer(FailingWriter, format_string, &args);
        assert!(result.is_err(), "预期写入失败");
    }

    #[test]
    fn test_can_parse_escape_only_invalid_escape() {
        let format_string = b"Hello \\xZZ";
        let result: Vec<_> = parse_escape_only(format_string).collect();
        // println!("{:?}", result);
        assert_eq!(
            result,
            [
                EscapedChar::Byte(72),
                EscapedChar::Byte(101),
                EscapedChar::Byte(108),
                EscapedChar::Byte(108),
                EscapedChar::Byte(111),
                EscapedChar::Byte(32),
                EscapedChar::Backslash(120),
                EscapedChar::Byte(90),
                EscapedChar::Byte(90)
            ]
        );
    }

    #[test]
    fn test_can_parse_spec_and_escape_invalid_mix() {
        let format_string = b"Hello \\xZZ %z World";
        let result: Vec<_> = parse_spec_and_escape(format_string).collect();
        // println!("{:?}", result);
        // 根据你的实现，这里需要检查对于混合无效转义序列和格式指令的处理
        assert!(matches!(
            result.first(),
            Some(Ok(FormatItem::Char(EscapedChar::Byte(72))))
        ));
        assert!(matches!(
            result.last(),
            Some(Ok(FormatItem::Char(EscapedChar::Byte(100))))
        ));
    }

    #[test]
    fn test_empty_format_string() {
        let format_str = b"";
        let format = Format::<MockFormatter>::parse(format_str);
        assert!(
            format.is_err(),
            "应当返回错误，因为空的格式字符串不包含任何格式化指令"
        );
    }

    #[test]
    fn test_format_string_with_only_prefix() {
        let format_str = b"Hello World!";
        let format = Format::<MockFormatter>::parse(format_str);
        assert!(
            format.is_err(),
            "应当返回错误，因为格式字符串不包含任何格式化指令"
        );
    }

    #[test]
    fn test_format_string_with_only_prefix_or_suffix() {
        // 仅包含前缀
        let format_str_prefix = b"Hello ";
        let format = Format::<MockFormatter>::parse(format_str_prefix);
        assert!(format.is_err(), "预期出错：格式字符串不包含格式化指令");

        // 仅包含后缀
        let format_str_suffix = b" World!";
        let format = Format::<MockFormatter>::parse(format_str_suffix);
        assert!(format.is_err(), "预期出错：格式字符串不包含格式化指令");
    }
    #[test]
    fn test_can_parse_invalid_format_specifiers() {
        let format_str = b"Hello %q World"; // %q 是一个无效的格式指令
        let result = Format::<MockFormatter>::parse(format_str);
        assert!(result.is_ok(), "预期能处理格式化指令");
    }

    #[test]
    fn test_malformed_escape_sequences() {
        let format_str = b"Hello \\xWorld"; // 不完整的转义序列
        let result = Format::<MockFormatter>::parse(format_str);
        assert!(result.is_err(), "预期出错：无效的格式化指令");

        // 因为当前实现并不解析转义序列，所以这个测试可能需要根据实际行为进行调整
        // 例如，如果实现了转义序列的解析，那么应该检查是否正确处理或返回错误
    }

    #[test]
    fn test_parse_format() {
        let format_str = b"Hello %d World!";
        let format = Format::<MockFormatter>::parse(format_str).unwrap();

        assert_eq!(format.prefix, b"Hello ".to_vec());
        assert_eq!(format.suffix, b" World!".to_vec());
    }

    #[test]
    fn test_fmt() {
        let format_str = b"Value: %d.";
        let format = Format::<MockFormatter>::parse(format_str).unwrap();

        let mut buffer = Cursor::new(Vec::new());
        format.fmt(&mut buffer, 42).unwrap();

        let result = String::from_utf8(buffer.into_inner()).unwrap();
        assert_eq!(&result, "Value: 42.");
    }
}
