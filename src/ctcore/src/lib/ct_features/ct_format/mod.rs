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

//!
//! printf风格的格式化
//! Rust具有出色的格式化能力，但coreutils需要非常特定的格式化，要求其行为与GNU实用程序完全一致。自然地，GNU的行为基于C语言的printf功能。
//! 此外，我们需要为printf实用程序支持转义序列。
//! printf和sprintf函数紧密匹配相应C函数的行为：前者将格式化字符串渲染到stdout，后者将格式化字符串渲染到新的String对象。
//! 我们可能想要进行三种类型的解析：
//! 仅解析printf指令（例如seq、dd）
//! 仅解析转义序列（例如echo）
//! 同时解析printf说明符和转义序列（例如printf）
//! 本模块旨在结合这三种用例。分别由parse_escape_only、parse_spec_only和parse_spec_and_escape提供解析每种情况的迭代器。
//! 有一个特殊的Format类型，可用于解析包含恰好一个指令且该指令中不使用任何*的格式字符串。这种格式可以在不失败（除IO错误外）的情况下以类型安全的方式打印。

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
    io::{stdout, Write},
    ops::ControlFlow,
};

use crate::ct_error::CTError;

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
impl CTError for FormatError {}

impl From<std::io::Error> for FormatError {
    fn from(value: std::io::Error) -> Self {
        Self::IoError(value)
    }
}

/**
 * 实现 `Display` trait 用于 `FormatError` 类型，以便错误可以被格式化并显示。
 *
 * `fmt` 函数根据 `FormatError` 的具体类型，生成对应的错误信息，并尝试将其格式化到给定的 `Formatter` 中。
 *
 * @param self `FormatError` 的引用，表示当前发生的错误。
 * @param f `Formatter` 的可变引用，用于指定错误信息的输出格式。
 * @return `std::fmt::Result`，表示格式化操作的结果，成功为 `Ok(())`，失败为 `Err(_)`。
 */
impl Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 根据 `FormatError` 的具体类型，选择错误信息模板
        match self {
            Self::SpecError(s) => write!(
                f,
                "%{}: invalid conversion specification",
                String::from_utf8_lossy(s)
            ), // 当错误为规格说明错误时，提供详细的错误信息
            Self::TooManySpecs(s) => write!(
                f,
                "format '{}' has too many % directives",
                String::from_utf8_lossy(s)
            ), // 当错误为规格说明数量过多时，提示用户格式字符串中有过多的 `%` 指令
            Self::NeedAtLeastOneSpec(s) => write!(
                f,
                "format '{}' has no % directive",
                String::from_utf8_lossy(s)
            ), // 当错误为缺少规格说明时，提示用户格式字符串中缺少 `%` 指令
            Self::WrongSpecType => write!(f, "wrong % directive type was given"), // 当错误为规格说明类型错误时，提示用户给出了错误的 `%` 指令类型
            Self::IoError(_) => write!(f, "io error"), // 当错误为I/O错误时，简单提示用户发生了I/O错误
            Self::NoMoreArguments => write!(f, "no more arguments"), // 当错误为没有更多参数时，提示用户没有提供足够的参数
            Self::InvalidArgument(_) => write!(f, "invalid argument"), // 当错误为参数无效时，简单提示用户参数无效
        }
    }
}
/// 定义了一个格式化项，可以是一个格式规范或单个字符。
pub enum FormatItem<C: FormatChar> {
    /// 一个格式规范。
    Spec(Spec),
    /// 单个字符。
    Char(C),
}

/// 定义了格式化字符的 trait，要求实现写入方法。
pub trait FormatChar {
    /// 将格式化字符写入给定的写入器。
    fn write(&self, writer: impl Write) -> std::io::Result<ControlFlow<()>>;
}

/// `u8` 类型实现了 `FormatChar`，将单个 `u8` 字节写入写入器。
impl FormatChar for u8 {
    fn write(&self, mut writer: impl Write) -> std::io::Result<ControlFlow<()>> {
        writer.write_all(&[*self])?;
        Ok(ControlFlow::Continue(()))
    }
}

/// `EscapedChar` 类型实现了 `FormatChar`，可以处理转义字符的写入。
impl FormatChar for EscapedChar {
    fn write(&self, mut writer: impl Write) -> std::io::Result<ControlFlow<()>> {
        match self {
            Self::Byte(c) => {
                writer.write_all(&[*c])?;
            }
            Self::Char(c) => {
                write!(writer, "{c}")?;
            }
            Self::Backslash(c) => {
                writer.write_all(&[b'\\', *c])?;
            }
            Self::End => return Ok(ControlFlow::Break(())),
        }
        Ok(ControlFlow::Continue(()))
    }
}

/// `FormatItem` 的实现，提供了将格式化项写入指定写入器的方法。
impl<C: FormatChar> FormatItem<C> {
    /// 将格式化项写入给定的写入器，可能需要迭代格式化参数。
    pub fn write<'a>(
        &self,
        writer: impl Write,
        args: &mut impl Iterator<Item = &'a FormatArgument>,
    ) -> Result<ControlFlow<()>, FormatError> {
        match self {
            Self::Spec(spec) => spec.write(writer, args)?,
            Self::Char(c) => return c.write(writer).map_err(FormatError::IoError),
        };
        Ok(ControlFlow::Continue(()))
    }
}

/// 解析包含 % 指令和转义序列的格式字符串。
pub fn parse_spec_and_escape(
    fmt: &[u8],
) -> impl Iterator<Item = Result<FormatItem<EscapedChar>, FormatError>> + '_ {
    // 根据格式字符串中的 % 指令和转义序列，生成格式化项的迭代器
    let mut current = fmt;
    std::iter::from_fn(move || match current {
        [] => None,
        [b'%', b'%', rest @ ..] => {
            current = rest;
            Some(Ok(FormatItem::Char(EscapedChar::Byte(b'%'))))
        }
        [b'%', rest @ ..] => {
            current = rest;
            let spec = match Spec::parse(&mut current) {
                Ok(spec) => spec,
                Err(slice) => return Some(Err(FormatError::SpecError(slice.to_vec()))),
            };
            Some(Ok(FormatItem::Spec(spec)))
        }
        [b'\\', rest @ ..] => {
            current = rest;
            Some(Ok(FormatItem::Char(parse_escape_code(&mut current))))
        }
        [c, rest @ ..] => {
            current = rest;
            Some(Ok(FormatItem::Char(EscapedChar::Byte(*c))))
        }
    })
}

/// 解析只包含 % 指令的格式字符串。
pub fn parse_spec_only(
    fmt: &[u8],
) -> impl Iterator<Item = Result<FormatItem<u8>, FormatError>> + '_ {
    // 生成只解析 % 指令的格式化项迭代器
    let mut current = fmt;
    std::iter::from_fn(move || match current {
        [] => None,
        [b'%', b'%', rest @ ..] => {
            current = rest;
            Some(Ok(FormatItem::Char(b'%')))
        }
        [b'%', rest @ ..] => {
            current = rest;
            let spec = match Spec::parse(&mut current) {
                Ok(spec) => spec,
                Err(slice) => return Some(Err(FormatError::SpecError(slice.to_vec()))),
            };
            Some(Ok(FormatItem::Spec(spec)))
        }
        [c, rest @ ..] => {
            current = rest;
            Some(Ok(FormatItem::Char(*c)))
        }
    })
}

/// 解析只包含转义序列的格式字符串。
pub fn parse_escape_only(fmt: &[u8]) -> impl Iterator<Item = EscapedChar> + '_ {
    // 解析只包含转义序列的格式字符串
    let mut current = fmt;
    std::iter::from_fn(move || match current {
        [] => None,
        [b'\\', rest @ ..] => {
            current = rest;
            Some(parse_escape_code(&mut current))
        }
        [c, rest @ ..] => {
            current = rest;
            Some(EscapedChar::Byte(*c))
        }
    })
}

/// 将格式化的字符串写入 stdout。
///
/// `format_string` 包含模板，`args` 包含渲染模板所需的参数。
///
/// 参见 [`sprintf`]，它创建一个新的格式化 [`String`]。
///
/// # 示例
///
///
///
/// ```rust
/// use ctcore::ct_format::{printf, FormatArgument};
///
/// printf("hello %s", &[FormatArgument::String("world".into())]).unwrap();
/// // prints "hello world"
/// ```
pub fn printf<'a>(
    format_string: impl AsRef<[u8]>,
    arguments: impl IntoIterator<Item = &'a FormatArgument>,
) -> Result<(), FormatError> {
    printf_writer(stdout(), format_string, arguments)
}

/// 写入格式化字符串到指定的写入器。
fn printf_writer<'a>(
    mut writer: impl Write,
    format_string: impl AsRef<[u8]>,
    args: impl IntoIterator<Item = &'a FormatArgument>,
) -> Result<(), FormatError> {
    let mut args = args.into_iter();
    for item in parse_spec_only(format_string.as_ref()) {
        item?.write(&mut writer, &mut args)?;
    }
    Ok(())
}

/// 创建一个新的格式化字符串。
///
/// `format_string` 包含模板，`args` 包含渲染模板所需的参数。
///
/// 参见 [`printf`]，它将内容打印到 stdout。
///
/// # 示例
///
///
///
/// ```rust
/// use ctcore::ct_format::{sprintf, FormatArgument};
///
/// let s = sprintf("hello %s", &[FormatArgument::String("world".into())]).unwrap();
/// let s = std::str::from_utf8(&s).unwrap();
/// assert_eq!(s, "hello world");
/// ```
pub fn sprintf<'a>(
    format_string: impl AsRef<[u8]>,
    arguments: impl IntoIterator<Item = &'a FormatArgument>,
) -> Result<Vec<u8>, FormatError> {
    let mut writer = Vec::new();
    printf_writer(&mut writer, format_string, arguments)?;
    Ok(writer)
}

/// 为单个浮点值解析格式。
///
/// 这被 `seq` 使用。可以通过 [`Format::parse`] 构造它，并使用
/// [`Format::fmt`] 写入一个值。
///
/// 它只能接受一个没有星号参数的规范。如果它得到更多规范，就会返回错误。
pub struct Format<F: Formatter> {
    prefix: Vec<u8>,
    suffix: Vec<u8>,
    formatter: F,
}

impl<F: Formatter> Format<F> {
    /// 解析一个格式字符串。
    pub fn parse(format_string: impl AsRef<[u8]>) -> Result<Self, FormatError> {
        // 解析格式字符串，提取前缀、规范和后缀
        let mut iter = parse_spec_only(format_string.as_ref());

        let mut prefix = Vec::new();
        let mut spec = None;
        for item in &mut iter {
            match item? {
                FormatItem::Spec(s) => {
                    spec = Some(s);
                    break;
                }
                FormatItem::Char(c) => prefix.push(c),
            }
        }

        let Some(spec) = spec else {
            return Err(FormatError::NeedAtLeastOneSpec(
                format_string.as_ref().to_vec(),
            ));
        };

        let formatter = F::try_from_spec(spec)?;

        let mut suffix = Vec::new();
        for item in &mut iter {
            match item? {
                FormatItem::Spec(_) => {
                    return Err(FormatError::TooManySpecs(format_string.as_ref().to_vec()));
                }
                FormatItem::Char(c) => suffix.push(c),
            }
        }

        Ok(Self {
            prefix,
            suffix,
            formatter,
        })
    }

    /// 格式化并写入给定的值。
    pub fn fmt(&self, mut w: impl Write, f: F::Input) -> std::io::Result<()> {
        // 将前缀、值（通过 formatter）和后缀写入给定的写入器
        w.write_all(&self.prefix)?;
        self.formatter.fmt(&mut w, f)?;
        w.write_all(&self.suffix)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as FmtWrite;
    use std::io;
    use std::io::Cursor;
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
