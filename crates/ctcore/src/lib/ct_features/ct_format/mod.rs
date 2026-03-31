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
use spec::IndexedSpec; // 引入包装后的 IndexedSpec
use std::{
    error::Error,
    fmt::Display,
    io::{Write, stdout},
    ops::ControlFlow,
};

use crate::ct_error::CTError;

use self::{
    escape::{EscapedChar, parse_escape_code},
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

impl Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpecError(s) => write!(
                f,
                "%{}: invalid conversion specification",
                String::from_utf8_lossy(s)
            ),
            Self::TooManySpecs(s) => write!(
                f,
                "format '{}' has too many % directives",
                String::from_utf8_lossy(s)
            ),
            Self::NeedAtLeastOneSpec(s) => write!(
                f,
                "format '{}' has no % directive",
                String::from_utf8_lossy(s)
            ),
            Self::WrongSpecType => write!(f, "wrong % directive type was given"),
            Self::IoError(_) => write!(f, "io error"),
            Self::NoMoreArguments => write!(f, "no more arguments"),
            Self::InvalidArgument(_) => write!(f, "invalid argument"),
        }
    }
}

/// 定义了一个格式化项，可以是一个格式规范或单个字符。
pub enum FormatItem<C: FormatChar> {
    /// 一个格式规范 (替换为了支持索引的 IndexedSpec)
    Spec(IndexedSpec),
    /// 单个字符。
    Char(C),
}

pub trait FormatChar {
    fn write(&self, writer: impl Write) -> std::io::Result<ControlFlow<()>>;
}

impl FormatChar for u8 {
    fn write(&self, mut writer: impl Write) -> std::io::Result<ControlFlow<()>> {
        writer.write_all(&[*self])?;
        Ok(ControlFlow::Continue(()))
    }
}

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

impl<C: FormatChar> FormatItem<C> {
    pub fn write<'a>(
        &self,
        writer: impl Write,
        cursor: &mut ArgCursor<'a>, // <--- 改用游标传递
    ) -> Result<ControlFlow<()>, FormatError> {
        match self {
            Self::Spec(spec) => spec.write(writer, cursor)?,
            Self::Char(c) => return c.write(writer).map_err(FormatError::IoError),
        };
        Ok(ControlFlow::Continue(()))
    }
}

pub fn parse_spec_and_escape(
    fmt: &[u8],
) -> impl Iterator<Item = Result<FormatItem<EscapedChar>, FormatError>> + '_ {
    let mut current = fmt;
    std::iter::from_fn(move || match current {
        [] => None,
        [b'%', b'%', rest @ ..] => {
            current = rest;
            Some(Ok(FormatItem::Char(EscapedChar::Byte(b'%'))))
        }
        [b'%', rest @ ..] => {
            current = rest;
            let spec = match IndexedSpec::parse(&mut current) {
                Ok(spec) => spec,
                Err(slice) => return Some(Err(FormatError::SpecError(slice.to_vec()))),
            };
            Some(Ok(FormatItem::Spec(spec)))
        }
        [b'\\', rest @ ..] => {
            current = rest;
            // 格式化字符串传入 false
            Some(parse_escape_code(&mut current, false).map(FormatItem::Char))
        }
        [c, rest @ ..] => {
            current = rest;
            Some(Ok(FormatItem::Char(EscapedChar::Byte(*c))))
        }
    })
}

pub fn parse_spec_only(
    fmt: &[u8],
) -> impl Iterator<Item = Result<FormatItem<u8>, FormatError>> + '_ {
    let mut current = fmt;
    std::iter::from_fn(move || match current {
        [] => None,
        [b'%', b'%', rest @ ..] => {
            current = rest;
            Some(Ok(FormatItem::Char(b'%')))
        }
        [b'%', rest @ ..] => {
            current = rest;
            let spec = match IndexedSpec::parse(&mut current) {
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

pub fn parse_escape_only(
    fmt: &[u8],
) -> impl Iterator<Item = Result<EscapedChar, FormatError>> + '_ {
    let mut current = fmt;
    std::iter::from_fn(move || match current {
        [] => None,
        [b'\\', rest @ ..] => {
            current = rest;
            // %b 内部字符串传入 true
            Some(parse_escape_code(&mut current, true))
        }
        [c, rest @ ..] => {
            current = rest;
            Some(Ok(EscapedChar::Byte(*c)))
        }
    })
}

pub fn printf<'a>(
    format_string: impl AsRef<[u8]>,
    arguments: impl IntoIterator<Item = &'a FormatArgument>,
) -> Result<(), FormatError> {
    printf_writer(stdout(), format_string, arguments)
}

fn printf_writer<'a>(
    mut writer: impl Write,
    format_string: impl AsRef<[u8]>,
    args: impl IntoIterator<Item = &'a FormatArgument>,
) -> Result<(), FormatError> {
    let var: Vec<FormatArgument> = args.into_iter().cloned().collect();
    let mut cursor = ArgCursor::new(&var);
    for item in parse_spec_only(format_string.as_ref()) {
        if let ControlFlow::Break(()) = item?.write(&mut writer, &mut cursor)? {
            break;
        }
    }
    Ok(())
}

pub fn sprintf<'a>(
    format_string: impl AsRef<[u8]>,
    arguments: impl IntoIterator<Item = &'a FormatArgument>,
) -> Result<Vec<u8>, FormatError> {
    let mut writer = Vec::new();
    printf_writer(&mut writer, format_string, arguments)?;
    Ok(writer)
}

pub struct Format<F: Formatter> {
    prefix: Vec<u8>,
    suffix: Vec<u8>,
    formatter: F,
}

impl<F: Formatter> Format<F> {
    pub fn parse(format_string: impl AsRef<[u8]>) -> Result<Self, FormatError> {
        let mut iter = parse_spec_only(format_string.as_ref());

        let mut prefix = Vec::new();
        let mut spec = None;
        for item in &mut iter {
            match item? {
                FormatItem::Spec(s) => {
                    spec = Some(s.spec); // 将底层的 Spec 剥离出来，完美兼容
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

    pub fn fmt(&self, mut w: impl Write, f: F::Input) -> std::io::Result<()> {
        w.write_all(&self.prefix)?;
        self.formatter.fmt(&mut w, f)?;
        w.write_all(&self.suffix)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ct_format::spec::Spec;
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
            write!(w, "{f}")
        }
    }

    #[test]
    fn test_spec_error_display() {
        let error = FormatError::SpecError(vec![b'a']);
        let mut output = String::new();
        write!(output, "{error}").unwrap();
        assert_eq!(output, "%a: invalid conversion specification");
    }

    #[test]
    fn test_too_many_specs_display() {
        let error = FormatError::TooManySpecs(vec![b'f', b'o', b'o']);
        let mut output = String::new();
        write!(output, "{error}").unwrap();
        assert_eq!(output, "format 'foo' has too many % directives");
    }

    #[test]
    fn test_need_at_least_one_spec_display() {
        let error = FormatError::NeedAtLeastOneSpec(vec![b'b', b'a', b'r']);
        let mut output = String::new();
        write!(output, "{error}").unwrap();
        assert_eq!(output, "format 'bar' has no % directive");
    }

    #[test]
    fn test_wrong_spec_type_display() {
        let error = FormatError::WrongSpecType;
        let mut output = String::new();
        write!(output, "{error}").unwrap();
        assert_eq!(output, "wrong % directive type was given");
    }

    #[test]
    fn test_io_error_display() {
        let error = FormatError::IoError(io::Error::other("test"));
        let mut output = String::new();
        write!(output, "{error}").unwrap();
        assert_eq!(output, "io error");
    }

    #[test]
    fn test_no_more_arguments_display() {
        let error = FormatError::NoMoreArguments;
        let mut output = String::new();
        write!(output, "{error}").unwrap();
        assert_eq!(output, "no more arguments");
    }

    #[test]
    fn test_invalid_argument_display() {
        let error = FormatError::InvalidArgument(FormatArgument::String("example".into()));
        let mut output = String::new();
        write!(output, "{error}").unwrap();
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
        let result: Vec<_> = parse_escape_only(format_string)
            .map(|r| r.unwrap())
            .collect();
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
                Err(io::Error::other("失败的写入"))
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
        let result: Vec<_> = parse_escape_only(format_string)
            .map(|r| r.unwrap())
            .collect();
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
