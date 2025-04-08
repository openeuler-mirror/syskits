/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

/// join 命令的实现 - 合并两个有序文件的相关行
///
/// # 功能描述
/// 此模块实现了类 Unix join 命令的功能，用于合并两个有序文件中具有相同连接字段的行。
///
/// # 主要特性
/// - 支持指定连接字段
/// - 支持自定义字段分隔符
/// - 支持自定义输出格式
/// - 支持处理未匹配行
/// - 支持排序检查
/// - 支持标题行处理
///
/// # 实现细节
/// - 使用流式处理方式，避免将整个文件加载到内存
/// - 支持多种分隔符模式（空白字符、自定义字符）
/// - 提供灵活的输出格式控制
/// - 实现了详细的错误处理和报告
///
/// # 注意事项
/// - 输入文件必须按照连接字段排序
/// - 默认使用第一个字段作为连接键
/// - 文件必须使用相同的字段分隔符
use clap::builder::ValueParser;
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo, set_ct_exit_code};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::{ct_crash_if_err, ct_format_usage, ct_help_about, ct_help_usage};
use memchr::{memchr_iter, memchr3_iter};
use std::cmp::Ordering;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Split, Stdin, Write, stdin, stdout};
use std::num::IntErrorKind;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

const JOIN_ABOUT: &str = ct_help_about!("join.md");
const JOIN_USAGE: &str = ct_help_usage!("join.md");

#[derive(Debug)]
enum JoinError {
    IOError(std::io::Error),
    UnorderedInput(String),
}

impl CTError for JoinError {
    fn code(&self) -> i32 {
        1
    }
}

impl Error for JoinError {}

impl Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IOError(e) => write!(f, "io error: {e}"),
            Self::UnorderedInput(e) => f.write_str(e),
        }
    }
}

impl From<std::io::Error> for JoinError {
    fn from(error: std::io::Error) -> Self {
        Self::IOError(error)
    }
}

#[derive(Copy, Clone, PartialEq)]
enum FileNum {
    File1,
    File2,
}

#[derive(Copy, Clone, PartialEq, Debug)]
enum Sep {
    Char(u8),
    Line,
    Whitespaces,
}

#[derive(Copy, Clone, PartialEq, Debug)]
enum CheckOrder {
    Default,
    Disabled,
    Enabled,
}

/// join 命令的配置选项
///
/// 存储所有用于控制 join 操作行为的配置参数
struct JoinSettings {
    /// 第一个文件的连接键字段索引
    key1: usize,
    /// 第二个文件的连接键字段索引
    key2: usize,
    /// 是否打印第一个文件中不匹配的行
    is_print_unpaired1: bool,
    /// 是否打印第二个文件中不匹配的行
    is_print_unpaired2: bool,
    /// 是否打印匹配的行
    is_print_joined: bool,
    /// 是否忽略大小写
    is_ignore_case: bool,
    /// 行终止符类型
    line_ending: CtLineEnding,
    /// 字段分隔符
    separator: Sep,
    /// 是否使用自动格式化
    is_autoformat: bool,
    /// 输出格式规范
    format: Vec<JoinSpec>,
    /// 空字段替换值
    empty: Vec<u8>,
    /// 排序检查模式
    check_order: CheckOrder,
    /// 是否处理标题行
    is_headers: bool,
}

impl Default for JoinSettings {
    fn default() -> Self {
        Self {
            key1: 0,
            key2: 0,
            is_print_unpaired1: false,
            is_print_unpaired2: false,
            is_print_joined: true,
            is_ignore_case: false,
            line_ending: CtLineEnding::Newline,
            separator: Sep::Whitespaces,
            is_autoformat: false,
            format: vec![],
            empty: vec![],
            check_order: CheckOrder::Default,
            is_headers: false,
        }
    }
}

/// 输出表示和格式化
///
/// 处理所有输出相关的操作，包括字段格式化和行终止符处理
struct JoinRepr<'a> {
    /// 行终止符类型
    line_ending: CtLineEnding,
    /// 输出字段分隔符
    separator: u8,
    /// 输出格式规范
    format: &'a [JoinSpec],
    /// 空字段替换值
    empty: &'a [u8],
}

impl<'a> JoinRepr<'a> {
    fn new(
        line_ending: CtLineEnding,
        separator: u8,
        format: &'a [JoinSpec],
        empty: &'a [u8],
    ) -> JoinRepr<'a> {
        JoinRepr {
            line_ending,
            separator,
            format,
            empty,
        }
    }

    fn uses_format(&self) -> bool {
        !self.format.is_empty()
    }

    /// 打印字段内容或空字段替代值
    ///
    /// # 参数
    /// * `writer` - 输出写入器
    /// * `field` - 要打印的字段内容，None 表示使用空字段替代值
    ///
    /// # 返回值
    /// 返回 IO 操作的结果
    fn print_field(
        &self,
        writer: &mut impl Write,
        field: Option<&[u8]>,
    ) -> Result<(), std::io::Error> {
        let content = field.unwrap_or(self.empty);
        writer.write_all(content)
    }

    /// 打印除指定索引外的所有字段
    ///
    /// # 参数
    /// * `writer` - 输出写入器
    /// * `line` - 包含所有字段的行
    /// * `skip_index` - 要跳过的字段索引
    ///
    /// # 返回值
    /// 返回 IO 操作的结果
    fn print_fields(
        &self,
        writer: &mut impl Write,
        line: &Line,
        skip_index: usize,
    ) -> Result<(), std::io::Error> {
        // 遍历所有字段
        for (index, _) in line.field_ranges.iter().enumerate() {
            // 跳过指定索引的字段
            if index == skip_index {
                continue;
            }

            // 写入分隔符和字段内容
            writer.write_all(&[self.separator])?;
            if let Some(field) = line.get_field(index) {
                writer.write_all(field)?;
            }
        }
        Ok(())
    }

    /// 按指定格式打印字段
    ///
    /// # 参数
    /// * `writer` - 输出写入器
    /// * `field_getter` - 获取字段内容的闭包函数
    ///
    /// # 返回值
    /// 返回 IO 操作的结果
    fn print_format<F>(
        &self,
        writer: &mut impl Write,
        field_getter: F,
    ) -> Result<(), std::io::Error>
    where
        F: Fn(&JoinSpec) -> Option<&'a [u8]>,
    {
        // 如果没有格式定义，直接返回
        if self.format.is_empty() {
            return Ok(());
        }

        // 打印第一个字段(无需分隔符)
        let first_content = field_getter(&self.format[0]).unwrap_or(self.empty);
        writer.write_all(first_content)?;

        // 打印剩余字段(带分隔符)
        for spec in &self.format[1..] {
            writer.write_all(&[self.separator])?;
            let content = field_getter(spec).unwrap_or(self.empty);
            writer.write_all(content)?;
        }

        Ok(())
    }

    fn print_line_ending(&self, writer: &mut impl Write) -> Result<(), std::io::Error> {
        writer.write_all(&[self.line_ending as u8])
    }
}

/// 输入处理参数
///
/// 控制输入文件的读取和处理行为
struct JoinInput {
    /// 输入字段分隔符
    separator: Sep,
    /// 是否忽略大小写
    ignore_case: bool,
    /// 排序检查模式
    check_order: CheckOrder,
}

impl JoinInput {
    fn new(separator: Sep, ignore_case: bool, check_order: CheckOrder) -> Self {
        Self {
            separator,
            ignore_case,
            check_order,
        }
    }

    fn compare(&self, field1: Option<&[u8]>, field2: Option<&[u8]>) -> Ordering {
        if let (Some(field1), Some(field2)) = (field1, field2) {
            if self.ignore_case {
                field1
                    .to_ascii_lowercase()
                    .cmp(&field2.to_ascii_lowercase())
            } else {
                field1.cmp(field2)
            }
        } else {
            match field1 {
                Some(_) => Ordering::Greater,
                None => match field2 {
                    Some(_) => Ordering::Less,
                    None => Ordering::Equal,
                },
            }
        }
    }
}

#[derive(Clone)]
enum JoinSpec {
    Key,
    Field(FileNum, usize),
}

impl JoinSpec {
    /// 解析格式化规范字符串
    ///
    /// # 参数
    /// * `format` - 格式化规范字符串，格式为:
    ///   - "0" 表示键字段
    ///   - "1.N" 表示第一个文件的第N个字段
    ///   - "2.N" 表示第二个文件的第N个字段
    ///
    /// # 返回值
    /// 返回解析后的 JoinSpec 枚举值
    ///
    /// # 错误
    /// - 格式字符串不符合规范
    /// - 文件编号无效
    /// - 字段编号无效
    fn parse(format: &str) -> CTResult<Self> {
        let mut chars = format.chars();

        // 解析文件编号部分
        let file_num = match chars.next() {
            Some('0') => {
                // "0" 必须单独存在，不能有字段编号
                if chars.next().is_none() {
                    return Ok(Self::Key);
                }
                return Err(CtSimpleError::new(
                    1,
                    format!("invalid field specifier: {}", format.quote()),
                ));
            }
            Some('1') => FileNum::File1,
            Some('2') => FileNum::File2,
            _ => {
                return Err(CtSimpleError::new(
                    1,
                    format!("invalid file number in field spec: {}", format.quote()),
                ));
            }
        };

        // 检查并解析字段编号部分
        match chars.next() {
            Some('.') => Ok(Self::Field(file_num, parse_field_number(chars.as_str())?)),
            _ => Err(CtSimpleError::new(
                1,
                format!("invalid field specifier: {}", format.quote()),
            )),
        }
    }
}

struct Line {
    field_ranges: Vec<(usize, usize)>,
    string: Vec<u8>,
}

impl Line {
    fn new(string: Vec<u8>, separator: Sep, len_guess: usize) -> Self {
        let mut field_ranges = Vec::with_capacity(len_guess);
        let mut last_end = 0;
        if separator == Sep::Whitespaces {
            // GNU join uses Bourne shell field splitters by default
            for i in memchr3_iter(b' ', b'\t', b'\n', &string) {
                if i > last_end {
                    field_ranges.push((last_end, i));
                }
                last_end = i + 1;
            }
        } else if let Sep::Char(sep) = separator {
            for i in memchr_iter(sep, &string) {
                field_ranges.push((last_end, i));
                last_end = i + 1;
            }
        }
        field_ranges.push((last_end, string.len()));

        Self {
            field_ranges,
            string,
        }
    }

    /// Get field at index.
    fn get_field(&self, index: usize) -> Option<&[u8]> {
        if index < self.field_ranges.len() {
            let (low, high) = self.field_ranges[index];
            Some(&self.string[low..high])
        } else {
            None
        }
    }
}

/// 文件状态管理
///
/// 管理单个输入文件的状态，包括行缓冲和处理
struct JoinState<'a> {
    /// 连接键字段索引
    key: usize,
    /// 文件名
    file_name: &'a str,
    /// 文件编号(File1或File2)
    file_num: FileNum,
    /// 是否打印不匹配的行
    print_unpaired: bool,
    /// 行迭代器
    lines: Split<Box<dyn BufRead + 'a>>,
    /// 预估的最大行长度
    max_len: usize,
    /// 当前行序列缓冲
    seq: Vec<Line>,
    /// 当前行号
    line_num: usize,
    /// 是否发生排序错误
    has_failed: bool,
    /// 是否存在不匹配的行
    has_unpaired: bool,
}

impl<'a> JoinState<'a> {
    /// 创建新的文件状态实例
    ///
    /// # 参数
    /// * `file_num` - 文件编号
    /// * `name` - 文件名
    /// * `stdin` - 标准输入引用
    /// * `key` - 连接键字段索引
    /// * `line_ending` - 行终止符类型
    /// * `print_unpaired` - 是否打印不匹配行
    fn new(
        file_num: FileNum,
        name: &'a str,
        stdin: &'a Stdin,
        key: usize,
        line_ending: CtLineEnding,
        print_unpaired: bool,
    ) -> CTResult<JoinState<'a>> {
        // 根据文件名创建适当的读取器
        let file_buf = if name == "-" {
            // 如果是标准输入，使用 stdin
            Box::new(stdin.lock()) as Box<dyn BufRead>
        } else {
            // 否则打开文件并创建缓冲读取器
            let file = File::open(name).map_err_context(|| format!("{}", name.maybe_quote()))?;
            Box::new(BufReader::new(file)) as Box<dyn BufRead>
        };

        // 创建并返回新的状态实例
        Ok(JoinState {
            key,
            file_name: name,
            file_num,
            print_unpaired,
            lines: file_buf.split(line_ending as u8),
            max_len: 1,
            seq: Vec::new(),
            line_num: 0,
            has_failed: false,
            has_unpaired: false,
        })
    }

    /// 跳过当前不匹配的行
    fn skip_line(
        &mut self,
        writer: &mut impl Write,
        input: &JoinInput,
        repr: &JoinRepr,
    ) -> CTResult<()> {
        // 如果需要打印不匹配的行
        if self.print_unpaired {
            self.print_first_line(writer, repr)?;
        }

        // 移动到下一行
        self.reset_next_line(input)?;
        Ok(())
    }

    /// 扩展当前行序列直到键值改变
    fn extend(&mut self, input: &JoinInput) -> CTResult<Option<Line>> {
        // 持续读取行，直到键值改变或文件结束
        while let Some(line) = self.next_line(input)? {
            // 比较当前键和新行的键
            let diff = input.compare(self.get_current_key(), line.get_field(self.key));

            if diff == Ordering::Equal {
                // 如果键相同，添加到序列
                self.seq.push(line);
            } else {
                // 如果键不同，返回这一行
                return Ok(Some(line));
            }
        }

        // 文件结束，返回 None
        Ok(None)
    }

    /// 打印标题行
    fn print_headers(
        &self,
        writer: &mut impl Write,
        other: &JoinState,
        repr: &JoinRepr,
    ) -> Result<(), std::io::Error> {
        if self.has_line() {
            if other.has_line() {
                // 如果两个文件都有行，合并打印
                self.combine(writer, other, repr)?;
            } else {
                // 如果只有第一个文件有行，只打印第一个
                self.print_first_line(writer, repr)?;
            }
        } else if other.has_line() {
            // 如果只有第二个文件有行，只打印第二个
            other.print_first_line(writer, repr)?;
        }

        Ok(())
    }

    /// 合并两个行序列
    fn combine(
        &self,
        writer: &mut impl Write,
        other: &JoinState,
        repr: &JoinRepr,
    ) -> Result<(), std::io::Error> {
        // 获取当前键值
        let key = self.get_current_key();

        // 对两个序列中的每一行进行组合
        for line1 in &self.seq {
            for line2 in &other.seq {
                if repr.uses_format() {
                    // 使用指定格式打印
                    repr.print_format(writer, |spec| match *spec {
                        JoinSpec::Key => key,
                        JoinSpec::Field(file_num, field_num) => {
                            // 根据文件编号选择正确的行
                            if file_num == self.file_num {
                                return line1.get_field(field_num);
                            }
                            if file_num == other.file_num {
                                return line2.get_field(field_num);
                            }
                            None
                        }
                    })?;
                } else {
                    repr.print_field(writer, key)?;
                    repr.print_fields(writer, line1, self.key)?;
                    repr.print_fields(writer, line2, other.key)?;
                }

                repr.print_line_ending(writer)?;
            }
        }

        Ok(())
    }

    /// Reset with the next line.
    fn reset(&mut self, next_line: Option<Line>) {
        self.seq.clear();

        if let Some(line) = next_line {
            self.seq.push(line);
        }
    }

    fn reset_read_line(&mut self, input: &JoinInput) -> Result<(), std::io::Error> {
        let line = self.read_line(input.separator)?;
        self.reset(line);
        Ok(())
    }

    fn reset_next_line(&mut self, input: &JoinInput) -> Result<(), JoinError> {
        let line = self.next_line(input)?;
        self.reset(line);
        Ok(())
    }

    fn has_line(&self) -> bool {
        !self.seq.is_empty()
    }

    /// 初始化文件处理
    ///
    /// # 参数
    /// * `separator` - 字段分隔符
    /// * `is_autoformat` - 是否使用自动格式化
    ///
    /// # 返回值
    /// 返回最大字段数
    fn initialize(&mut self, read_sep: Sep, autoformat: bool) -> usize {
        if let Some(line) = ct_crash_if_err!(1, self.read_line(read_sep)) {
            self.seq.push(line);

            if autoformat {
                return self.seq[0].field_ranges.len();
            }
        }
        0
    }

    fn finalize(
        &mut self,
        writer: &mut impl Write,
        input: &JoinInput,
        repr: &JoinRepr,
    ) -> CTResult<()> {
        if self.has_line() {
            if self.print_unpaired {
                self.print_first_line(writer, repr)?;
            }

            let mut next_line = self.next_line(input)?;
            while let Some(line) = &next_line {
                if self.print_unpaired {
                    self.print_line(writer, line, repr)?;
                }
                self.reset(next_line);
                next_line = self.next_line(input)?;
            }
        }

        Ok(())
    }

    /// Get the next line without the order check.
    fn read_line(&mut self, sep: Sep) -> Result<Option<Line>, std::io::Error> {
        match self.lines.next() {
            Some(value) => {
                self.line_num += 1;
                let line = Line::new(value?, sep, self.max_len);
                if line.field_ranges.len() > self.max_len {
                    self.max_len = line.field_ranges.len();
                }
                Ok(Some(line))
            }
            None => Ok(None),
        }
    }

    /// 获取下一行并检查排序顺序
    ///
    /// # 参数
    /// * `input` - 输入处理参数
    ///
    /// # 返回值
    /// 返回下一行，如果发现排序错误则设置错误标志
    fn next_line(&mut self, input: &JoinInput) -> Result<Option<Line>, JoinError> {
        // 读取下一行
        if let Some(line) = self.read_line(input.separator)? {
            // 如果不需要检查排序顺序，直接返回
            if input.check_order == CheckOrder::Disabled {
                return Ok(Some(line));
            }

            let diff = input.compare(self.get_current_key(), line.get_field(self.key));

            if diff == Ordering::Greater
                && (input.check_order == CheckOrder::Enabled
                    || (self.has_unpaired && !self.has_failed))
            {
                let err_msg = format!(
                    "{}:{}: is not sorted: {}",
                    self.file_name.maybe_quote(),
                    self.line_num,
                    String::from_utf8_lossy(&line.string)
                );
                // This is fatal if the check is enabled.
                if input.check_order == CheckOrder::Enabled {
                    return Err(JoinError::UnorderedInput(err_msg));
                }
                eprintln!("{}: {}", ctcore::ct_execute_phrase(), err_msg);
                self.has_failed = true;
            }

            Ok(Some(line))
        } else {
            Ok(None)
        }
    }

    /// 打印单行数据
    ///
    /// # 参数
    /// * `writer` - 输出写入器
    /// * `line` - 要打印的行
    /// * `repr` - 输出格式控制
    fn print_line(
        &self,
        writer: &mut impl Write,
        line: &Line,
        repr: &JoinRepr,
    ) -> Result<(), std::io::Error> {
        // 如果使用自定义格式，按格式打印
        if repr.uses_format() {
            repr.print_format(writer, |spec| match *spec {
                JoinSpec::Key => line.get_field(self.key),
                JoinSpec::Field(file_num, field_num) => {
                    if file_num == self.file_num {
                        line.get_field(field_num)
                    } else {
                        None
                    }
                }
            })?;
        } else {
            // 否则打印所有字段
            repr.print_field(writer, line.get_field(self.key))?;
            repr.print_fields(writer, line, self.key)?;
        }
        repr.print_line_ending(writer)
    }

    fn print_first_line(
        &self,
        writer: &mut impl Write,
        repr: &JoinRepr,
    ) -> Result<(), std::io::Error> {
        self.print_line(writer, &self.seq[0], repr)
    }

    /// Gets the key value of the lines stored in seq.
    fn get_current_key(&self) -> Option<&[u8]> {
        self.seq[0].get_field(self.key)
    }
}

fn parse_separator(value_os: &OsString) -> CTResult<Sep> {
    #[cfg(unix)]
    let value = value_os.as_bytes();
    #[cfg(not(unix))]
    let value = match value_os.to_str() {
        Some(value) => value.as_bytes(),
        None => {
            return Err(CtSimpleError::new(
                1,
                "unprintable field separators are only supported on unix-like platforms",
            ));
        }
    };
    match value.len() {
        0 => Ok(Sep::Line),
        1 => Ok(Sep::Char(value[0])),
        2 if value[0] == b'\\' && value[1] == b'0' => Ok(Sep::Char(0)),
        _ => Err(CtSimpleError::new(
            1,
            format!("multi-character tab {}", value_os.to_string_lossy()),
        )),
    }
}

fn parse_print_settings(matches: &clap::ArgMatches) -> CTResult<(bool, bool, bool)> {
    let mut print_joined = true;
    let mut print_unpaired1 = false;
    let mut print_unpaired2 = false;

    let v_values = matches.get_many::<String>("v");
    if v_values.is_some() {
        print_joined = false;
    }

    let unpaired = v_values
        .unwrap_or_default()
        .chain(matches.get_many("a").unwrap_or_default());
    for file_num in unpaired {
        match parse_file_number(file_num)? {
            FileNum::File1 => print_unpaired1 = true,
            FileNum::File2 => print_unpaired2 = true,
        }
    }

    Ok((print_joined, print_unpaired1, print_unpaired2))
}

fn get_and_parse_field_number(matches: &clap::ArgMatches, key: &str) -> CTResult<Option<usize>> {
    let value = matches.get_one::<String>(key).map(|s| s.as_str());
    parse_field_number_option(value)
}

impl JoinSettings {
    /// 从命令行参数解析并创建 JoinSettings 实例
    ///
    /// # 参数
    /// * `matches` - 命令行参数匹配结果
    ///
    /// # 返回值
    /// 返回 `CTResult<JoinSettings>`，包含解析后的设置
    pub fn new(matches: &clap::ArgMatches) -> CTResult<Self> {
        let mut settings = JoinSettings::default();

        let keys = get_and_parse_field_number(matches, "j")?;
        let key1 = get_and_parse_field_number(matches, "1")?;
        let key2 = get_and_parse_field_number(matches, "2")?;

        let (print_joined, print_unpaired1, print_unpaired2) = parse_print_settings(matches)?;

        settings.is_print_joined = print_joined;
        settings.is_print_unpaired1 = print_unpaired1;
        settings.is_print_unpaired2 = print_unpaired2;

        settings.is_ignore_case = matches.get_flag("i");
        settings.key1 = get_field_number(keys, key1)?;
        settings.key2 = get_field_number(keys, key2)?;
        if let Some(value_os) = matches.get_one::<OsString>("t") {
            settings.separator = parse_separator(value_os)?;
        }
        if let Some(format) = matches.get_one::<String>("o") {
            if format == "auto" {
                settings.is_autoformat = true;
            } else {
                let mut specs = vec![];
                for part in format.split([' ', ',', '\t']) {
                    specs.push(JoinSpec::parse(part)?);
                }
                settings.format = specs;
            }
        }

        if let Some(empty) = matches.get_one::<String>("e") {
            settings.empty = empty.as_bytes().to_vec();
        }

        if matches.get_flag("nocheck-order") {
            settings.check_order = CheckOrder::Disabled;
        }

        if matches.get_flag("check-order") {
            settings.check_order = CheckOrder::Enabled;
        }

        if matches.get_flag("header") {
            settings.is_headers = true;
        }

        settings.line_ending = CtLineEnding::from_zero_flag(matches.get_flag("z"));

        Ok(settings)
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    join_main(args)
}

pub fn join_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let settings = JoinSettings::new(&matches)?;

    let file1 = matches.get_one::<String>("file1").unwrap();
    let file2 = matches.get_one::<String>("file2").unwrap();

    if file1 == "-" && file2 == "-" {
        return Err(CtSimpleError::new(1, "both files cannot be standard input"));
    }

    join_exec(file1, file2, settings)
}

#[derive(Default)]
pub struct Join;
impl Tool for Join {
    fn name(&self) -> &'static str {
        "join"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        join_main(args.iter().cloned())
    }
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new("a")
            .short('a')
            .action(ArgAction::Append)
            .num_args(1)
            .value_parser(["1", "2"])
            .value_name("FILENUM")
            .help(
                "also print unpairable lines from file FILENUM, where
FILENUM is 1 or 2, corresponding to FILE1 or FILE2",
            ),
        Arg::new("v")
            .short('v')
            .action(ArgAction::Append)
            .num_args(1)
            .value_parser(["1", "2"])
            .value_name("FILENUM")
            .help("like -a FILENUM, but suppress joined output lines"),
        Arg::new("e")
            .short('e')
            .value_name("EMPTY")
            .help("replace missing input fields with EMPTY"),
        Arg::new("i")
            .short('i')
            .long("ignore-case")
            .help("ignore differences in case when comparing fields")
            .action(ArgAction::SetTrue),
        Arg::new("j")
            .short('j')
            .value_name("FIELD")
            .help("equivalent to '-1 FIELD -2 FIELD'"),
        Arg::new("o")
            .short('o')
            .value_name("FORMAT")
            .help("obey FORMAT while constructing output line"),
        Arg::new("t")
            .short('t')
            .value_name("CHAR")
            .value_parser(ValueParser::os_string())
            .help("use CHAR as input and output field separator"),
        Arg::new("1")
            .short('1')
            .value_name("FIELD")
            .help("join on this FIELD of file 1"),
        Arg::new("2")
            .short('2')
            .value_name("FIELD")
            .help("join on this FIELD of file 2"),
        Arg::new("check-order")
            .long("check-order")
            .help(
                "check that the input is correctly sorted, \
            even if all input lines are pairable",
            )
            .action(ArgAction::SetTrue),
        Arg::new("nocheck-order")
            .long("nocheck-order")
            .help("do not check that the input is correctly sorted")
            .action(ArgAction::SetTrue),
        Arg::new("header")
            .long("header")
            .help(
                "treat the first line in each file as field headers, \
            print them without trying to pair them",
            )
            .action(ArgAction::SetTrue),
        Arg::new("z")
            .short('z')
            .long("zero-terminated")
            .help("line delimiter is NUL, not newline")
            .action(ArgAction::SetTrue),
        Arg::new("file1")
            .required(true)
            .value_name("FILE1")
            .value_hint(clap::ValueHint::FilePath)
            .hide(true),
        Arg::new("file2")
            .required(true)
            .value_name("FILE2")
            .value_hint(clap::ValueHint::FilePath)
            .hide(true),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(JOIN_ABOUT)
        .override_usage(ct_format_usage(JOIN_USAGE))
        .infer_long_args(true)
        .args(args)
}

/// 执行文件连接操作
///
/// # 参数
/// * `file1` - 第一个输入文件路径
/// * `file2` - 第二个输入文件路径
/// * `settings` - 连接操作的配置选项
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功
fn join_exec(file1: &str, file2: &str, settings: JoinSettings) -> CTResult<()> {
    let stdin = stdin(); // Move stdin here to extend its lifetime
    let (mut state1, mut state2, input, repr, mut writer) =
        init_join_states(file1, file2, &settings, &stdin)?;

    // 处理标题行
    if settings.is_headers {
        process_headers(&mut state1, &mut state2, &mut writer, &input, &repr)?;
    }

    // 执行主连接循环
    process_join_loop(
        &mut state1,
        &mut state2,
        &mut writer,
        &input,
        &repr,
        &settings,
    )?;

    // 处理剩余行并完成操作
    finalize_join(&mut state1, &mut state2, &mut writer, &input, &repr)?;

    Ok(())
}

/// 初始化连接操作的状态和输出
fn init_join_states<'a>(
    file1: &'a str,
    file2: &'a str,
    settings: &'a JoinSettings,
    stdin: &'a Stdin, // Add stdin parameter
) -> CTResult<(
    JoinState<'a>,
    JoinState<'a>,
    JoinInput,
    JoinRepr<'a>,
    BufWriter<std::io::StdoutLock<'a>>,
)> {
    // 初始化文件状态
    let mut state1 = JoinState::new(
        FileNum::File1,
        file1,
        stdin,
        settings.key1,
        settings.line_ending,
        settings.is_print_unpaired1,
    )?;

    let mut state2 = JoinState::new(
        FileNum::File2,
        file2,
        stdin,
        settings.key2,
        settings.line_ending,
        settings.is_print_unpaired2,
    )?;

    // 创建输入处理器
    let input = JoinInput::new(
        settings.separator,
        settings.is_ignore_case,
        settings.check_order,
    );

    // 准备输出格式
    let _ = prepare_format(&mut state1, &mut state2, settings);

    let repr = JoinRepr::new(
        settings.line_ending,
        match settings.separator {
            Sep::Char(sep) => sep,
            _ => b' ',
        },
        &settings.format, // Reference format from settings
        &settings.empty,
    );

    // 准备输出缓冲
    let stdout = stdout();
    let writer = BufWriter::new(stdout.lock());

    Ok((state1, state2, input, repr, writer))
}

/// 准备输出格式
fn prepare_format(
    state1: &mut JoinState,
    state2: &mut JoinState,
    settings: &JoinSettings,
) -> Vec<JoinSpec> {
    if settings.is_autoformat {
        let mut format = vec![JoinSpec::Key];
        let mut initialize = |state: &mut JoinState| {
            let max_fields = state.initialize(settings.separator, settings.is_autoformat);
            for i in 0..max_fields {
                if i != state.key {
                    format.push(JoinSpec::Field(state.file_num, i));
                }
            }
        };
        initialize(state1);
        initialize(state2);
        format
    } else {
        state1.initialize(settings.separator, settings.is_autoformat);
        state2.initialize(settings.separator, settings.is_autoformat);
        settings.format.clone()
    }
}

/// 处理标题行
///
/// # 参数
/// * `state1` - 第一个文件的状态
/// * `state2` - 第二个文件的状态
/// * `writer` - 输出写入器
/// * `input` - 输入处理参数
/// * `repr` - 输出格式控制
fn process_headers(
    state1: &mut JoinState,
    state2: &mut JoinState,
    writer: &mut BufWriter<std::io::StdoutLock>,
    input: &JoinInput,
    repr: &JoinRepr,
) -> CTResult<()> {
    // 打印标题行
    state1.print_headers(writer, state2, repr)?;
    // 重置两个文件的读取位置
    state1.reset_read_line(input)?;
    state2.reset_read_line(input)?;
    Ok(())
}

/// 执行主连接循环
///
/// # 参数
/// * `state1` - 第一个文件的状态
/// * `state2` - 第二个文件的状态
/// * `writer` - 输出写入器
/// * `input` - 输入处理参数
/// * `repr` - 输出格式控制
/// * `settings` - 连接操作的配置
fn process_join_loop(
    state1: &mut JoinState,
    state2: &mut JoinState,
    writer: &mut BufWriter<std::io::StdoutLock>,
    input: &JoinInput,
    repr: &JoinRepr,
    settings: &JoinSettings,
) -> CTResult<()> {
    // 当两个文件都还有数据时继续处理
    while state1.has_line() && state2.has_line() {
        // 比较两个文件当前行的键
        let diff = input.compare(state1.get_current_key(), state2.get_current_key());
        // 根据比较结果处理不同情况
        match diff {
            Ordering::Less => process_less_case(state1, state2, writer, input, repr)?,
            Ordering::Greater => process_greater_case(state1, state2, writer, input, repr)?,
            Ordering::Equal => process_equal_case(state1, state2, writer, input, repr, settings)?,
        }
    }
    Ok(())
}

/// 处理第一个文件键值小于第二个文件的情况
///
/// # 参数
/// * `state1` - 第一个文件的状态
/// * `state2` - 第二个文件的状态
/// * `writer` - 输出写入器
/// * `input` - 输入处理参数
/// * `repr` - 输出格式控制
fn process_less_case(
    state1: &mut JoinState,
    state2: &mut JoinState,
    writer: &mut BufWriter<std::io::StdoutLock>,
    input: &JoinInput,
    repr: &JoinRepr,
) -> CTResult<()> {
    // 尝试跳过第一个文件的当前行
    if let Err(e) = state1.skip_line(writer, input, repr) {
        writer.flush()?;
        return Err(e);
    }
    // 标记两个文件都有不匹配的行
    state1.has_unpaired = true;
    state2.has_unpaired = true;
    Ok(())
}

/// 处理第一个文件键值大于第二个文件的情况
///
/// # 参数
/// * `state1` - 第一个文件的状态
/// * `state2` - 第二个文件的状态
/// * `writer` - 输出写入器
/// * `input` - 输入处理参数
/// * `repr` - 输出格式控制
fn process_greater_case(
    state1: &mut JoinState,
    state2: &mut JoinState,
    writer: &mut BufWriter<std::io::StdoutLock>,
    input: &JoinInput,
    repr: &JoinRepr,
) -> CTResult<()> {
    // 尝试跳过第二个文件的当前行
    if let Err(e) = state2.skip_line(writer, input, repr) {
        writer.flush()?;
        return Err(e);
    }
    // 标记两个文件都有不匹配的行
    state1.has_unpaired = true;
    state2.has_unpaired = true;
    Ok(())
}

/// 处理两个文件键值相等的情况
///
/// # 参数
/// * `state1` - 第一个文件的状态
/// * `state2` - 第二个文件的状态
/// * `writer` - 输出写入器
/// * `input` - 输入处理参数
/// * `repr` - 输出格式控制
/// * `settings` - 连接操作的配置
fn process_equal_case(
    state1: &mut JoinState,
    state2: &mut JoinState,
    writer: &mut BufWriter<std::io::StdoutLock>,
    input: &JoinInput,
    repr: &JoinRepr,
    settings: &JoinSettings,
) -> CTResult<()> {
    // 扩展第一个文件的行序列
    let next_line1 = match state1.extend(input) {
        Ok(line) => line,
        Err(e) => {
            writer.flush()?;
            return Err(e);
        }
    };
    // 扩展第二个文件的行序列
    let next_line2 = match state2.extend(input) {
        Ok(line) => line,
        Err(e) => {
            writer.flush()?;
            return Err(e);
        }
    };

    // 如果需要打印匹配的行
    if settings.is_print_joined {
        state1.combine(writer, state2, repr)?;
    }

    // 重置两个文件的状态
    state1.reset(next_line1);
    state2.reset(next_line2);
    Ok(())
}

/// 完成连接操作
///
/// # 参数
/// * `state1` - 第一个文件的状态
/// * `state2` - 第二个文件的状态
/// * `writer` - 输出写入器
/// * `input` - 输入处理参数
/// * `repr` - 输出格式控制
fn finalize_join(
    state1: &mut JoinState,
    state2: &mut JoinState,
    writer: &mut BufWriter<std::io::StdoutLock>,
    input: &JoinInput,
    repr: &JoinRepr,
) -> CTResult<()> {
    // 处理第一个文件的剩余行
    if let Err(e) = state1.finalize(writer, input, repr) {
        writer.flush()?;
        return Err(e);
    }
    // 处理第二个文件的剩余行
    if let Err(e) = state2.finalize(writer, input, repr) {
        writer.flush()?;
        return Err(e);
    }

    // 刷新输出缓冲
    writer.flush()?;

    // 如果有排序错误，设置错误状态
    if state1.has_failed || state2.has_failed {
        eprintln!(
            "{}: input is not in sorted order",
            ctcore::ct_execute_phrase()
        );
        set_ct_exit_code(1);
    }
    Ok(())
}

/// 获取字段编号
///
/// 检查并解决多个字段指定可能的冲突
///
/// # 参数
/// * `keys` - 通过 -j 选项指定的字段
/// * `key` - 通过 -1 或 -2 选项指定的字段
///
/// # 返回值
/// 返回最终确定的字段编号
fn get_field_number(keys: Option<usize>, key: Option<usize>) -> CTResult<usize> {
    if let Some(keys) = keys {
        if let Some(key) = key {
            if keys != key {
                // Show zero-based field numbers as one-based.
                return Err(CtSimpleError::new(
                    1,
                    format!("incompatible join fields {}, {}", keys + 1, key + 1),
                ));
            }
        }

        return Ok(keys);
    }

    Ok(key.unwrap_or(0))
}

/// 解析字段编号
///
/// # 参数
/// * `value` - 字段编号字符串
///
/// # 返回值
/// 返回解析后的0基字段索引
///
/// # 错误
/// - 如果输入不是正整数
/// - 如果输入超出范围
fn parse_field_number(value: &str) -> CTResult<usize> {
    match value.parse::<usize>() {
        Ok(result) if result > 0 => Ok(result - 1),
        Err(e) if e.kind() == &IntErrorKind::PosOverflow => Ok(usize::MAX),
        _ => Err(CtSimpleError::new(
            1,
            format!("invalid field number: {}", value.quote()),
        )),
    }
}

fn parse_file_number(value: &str) -> CTResult<FileNum> {
    match value {
        "1" => Ok(FileNum::File1),
        "2" => Ok(FileNum::File2),
        value => Err(CtSimpleError::new(
            1,
            format!("invalid file number: {}", value.quote()),
        )),
    }
}

fn parse_field_number_option(value: Option<&str>) -> CTResult<Option<usize>> {
    match value {
        None => Ok(None),
        Some(val) => Ok(Some(parse_field_number(val)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_tool_implementation() {
        let tool = Join::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "join");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("join"));

        // 测试 execute 方法
        let args = vec![OsString::from("join"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }
    #[test]
    fn test_parse_settings_basic() {
        // 创建基本的命令行参数
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "file1.txt", "file2.txt"])
            .unwrap();

        // 测试默认设置
        let settings = JoinSettings::new(&matches).unwrap();
        assert_eq!(settings.key1, 0);
        assert_eq!(settings.key2, 0);
        assert!(!settings.is_print_unpaired1);
        assert!(!settings.is_print_unpaired2);
        assert!(settings.is_print_joined);
        assert!(!settings.is_ignore_case);
        assert_eq!(settings.line_ending, CtLineEnding::Newline);
        assert_eq!(settings.separator, Sep::Whitespaces);
        assert!(!settings.is_autoformat);
        assert!(settings.format.is_empty());
        assert!(settings.empty.is_empty());
        assert_eq!(settings.check_order, CheckOrder::Default);
        assert!(!settings.is_headers);
    }

    #[test]
    fn test_parse_settings_keys() {
        // 测试设置键字段
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "-1", "2", "-2", "3", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert_eq!(settings.key1, 1); // 2-1 因为是从1开始计数
        assert_eq!(settings.key2, 2); // 3-1
    }

    #[test]
    fn test_parse_settings_print_options() {
        // 测试打印选项
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "-a", "1", "-v", "2", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert!(settings.is_print_unpaired1);
        assert!(settings.is_print_unpaired2);
        assert!(!settings.is_print_joined); // -v 选项禁用连接行的打印
    }

    #[test]
    fn test_parse_settings_format() {
        // 测试格式化选项
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "-o", "1.1,2.2,0", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert!(!settings.is_autoformat);
        assert_eq!(settings.format.len(), 3);

        // 测试自动格式化
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "-o", "auto", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert!(settings.is_autoformat);
        assert!(settings.format.is_empty());
    }

    #[test]
    fn test_parse_settings_separator() {
        // 测试分隔符选项
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "-t", ",", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert!(matches!(settings.separator, Sep::Char(b',')));

        // 测试空分隔符
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "-t", "", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert!(matches!(settings.separator, Sep::Line));
    }

    #[test]
    fn test_parse_settings_order_check() {
        // 测试排序检查选项
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "--check-order", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert_eq!(settings.check_order, CheckOrder::Enabled);

        // 测试禁用排序检查
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "--nocheck-order", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert_eq!(settings.check_order, CheckOrder::Disabled);
    }

    #[test]
    fn test_parse_settings_headers() {
        // 测试标题选项
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "--header", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert!(settings.is_headers);
    }

    #[test]
    fn test_parse_settings_empty_field() {
        // 测试空字段替换选项
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "-e", "EMPTY", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert_eq!(settings.empty, b"EMPTY");
    }

    #[test]
    fn test_parse_settings_ignore_case() {
        // 测试忽略大小写选项
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "-i", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert!(settings.is_ignore_case);
    }

    #[test]
    fn test_parse_settings_zero_terminated() {
        // 测试零终止符选项
        let matches = ct_app()
            .try_get_matches_from(vec!["join", "-z", "file1.txt", "file2.txt"])
            .unwrap();

        let settings = JoinSettings::new(&matches).unwrap();
        assert_eq!(settings.line_ending, CtLineEnding::Nul);
    }

    use std::io::Cursor;

    #[test]
    fn test_print_field() {
        // 设置测试环境
        let empty = b"EMPTY";
        let repr = JoinRepr::new(CtLineEnding::Newline, b',', &[], empty);
        let mut output = Cursor::new(Vec::new());

        // 测试正常字段
        let field = b"test";
        repr.print_field(&mut output, Some(field)).unwrap();
        assert_eq!(output.get_ref(), b"test");

        // 测试空字段(使用empty替换)
        output = Cursor::new(Vec::new());
        repr.print_field(&mut output, None).unwrap();
        assert_eq!(output.get_ref(), b"EMPTY");

        // 测试空字符串字段
        output = Cursor::new(Vec::new());
        repr.print_field(&mut output, Some(b"")).unwrap();
        assert_eq!(output.get_ref(), b"");
    }

    #[test]
    fn test_print_fields() {
        // 创建测试行
        let line = Line::new(b"field1,field2,field3,field4".to_vec(), Sep::Char(b','), 4);

        // 设置测试环境
        let empty = b"EMPTY";
        let repr = JoinRepr::new(CtLineEnding::Newline, b',', &[], empty);
        let mut output = Cursor::new(Vec::new());

        // 测试跳过第一个字段
        repr.print_fields(&mut output, &line, 0).unwrap();

        // 添加调试输出
        println!("Skip first field test:");
        println!("Expected: {:?}", b",field2,field3,field4");
        println!("Got: {:?}", output.get_ref());
        assert_eq!(output.get_ref(), b",field2,field3,field4");

        // 其他测试...
    }

    #[test]
    fn test_print_format() {
        // 设置测试环境
        let empty = b"EMPTY";
        let format = vec![
            JoinSpec::Key,
            JoinSpec::Field(FileNum::File1, 1),
            JoinSpec::Field(FileNum::File2, 2),
        ];
        let repr = JoinRepr::new(CtLineEnding::Newline, b',', &format, empty);
        let mut output = Cursor::new(Vec::new());

        // 测试正常格式化
        let get_field = |spec: &JoinSpec| match spec {
            JoinSpec::Key => Some(b"key" as &[u8]),
            JoinSpec::Field(FileNum::File1, 1) => Some(b"field1" as &[u8]),
            JoinSpec::Field(FileNum::File2, 2) => Some(b"field2" as &[u8]),
            _ => None,
        };

        repr.print_format(&mut output, get_field).unwrap();

        // 添加调试输出
        println!("Normal format test:");
        println!("Expected: {:?}", b"key,field1,field2");
        println!("Got: {:?}", output.get_ref());
        assert_eq!(output.get_ref(), b"key,field1,field2");
    }

    mod join_exec_tests {
        use super::*;
        fn create_test_file(path: &std::path::Path, content: &str) -> std::io::Result<()> {
            let mut file = File::create(path)?;
            file.write_all(content.as_bytes())?;
            Ok(())
        }

        #[test]
        fn test_join_exec_basic() {
            let temp = tempdir().unwrap();

            // 创建测试文件
            let file1_path = temp.path().join("file1.txt");
            let file2_path = temp.path().join("file2.txt");

            create_test_file(&file1_path, "1 a\n2 b\n3 c\n").unwrap();
            create_test_file(&file2_path, "1 x\n2 y\n3 z\n").unwrap();

            // 基本连接测试
            let settings = JoinSettings {
                key1: 0,
                key2: 0,
                is_print_joined: true,
                separator: Sep::Whitespaces,
                ..Default::default()
            };

            let result = join_exec(
                file1_path.to_str().unwrap(),
                file2_path.to_str().unwrap(),
                settings,
            );
            assert!(result.is_ok());
        }

        #[test]
        fn test_join_exec_unmatched() {
            let temp = tempdir().unwrap();

            // 创建测试文件，包含不匹配的行
            let file1_path = temp.path().join("file1.txt");
            let file2_path = temp.path().join("file2.txt");

            create_test_file(&file1_path, "1 a\n2 b\n4 d\n").unwrap();
            create_test_file(&file2_path, "1 x\n3 z\n4 w\n").unwrap();

            // 测试打印不匹配行
            let settings = JoinSettings {
                key1: 0,
                key2: 0,
                is_print_joined: true,
                is_print_unpaired1: true,
                is_print_unpaired2: true,
                separator: Sep::Whitespaces,
                ..Default::default()
            };

            let result = join_exec(
                file1_path.to_str().unwrap(),
                file2_path.to_str().unwrap(),
                settings,
            );
            assert!(result.is_ok());
        }

        #[test]
        fn test_join_exec_custom_separator() {
            let temp = tempdir().unwrap();

            // 创建使用自定义分隔符的测试文件
            let file1_path = temp.path().join("file1.txt");
            let file2_path = temp.path().join("file2.txt");

            create_test_file(&file1_path, "1,a\n2,b\n3,c\n").unwrap();
            create_test_file(&file2_path, "1,x\n2,y\n3,z\n").unwrap();

            // 测试自定义分隔符
            let settings = JoinSettings {
                key1: 0,
                key2: 0,
                is_print_joined: true,
                separator: Sep::Char(b','),
                ..Default::default()
            };

            let result = join_exec(
                file1_path.to_str().unwrap(),
                file2_path.to_str().unwrap(),
                settings,
            );
            assert!(result.is_ok());
        }

        #[test]
        fn test_join_exec_with_headers() {
            let temp = tempdir().unwrap();

            // 创建带标题行的测试文件
            let file1_path = temp.path().join("file1.txt");
            let file2_path = temp.path().join("file2.txt");

            create_test_file(&file1_path, "id name\n1 a\n2 b\n").unwrap();
            create_test_file(&file2_path, "id value\n1 x\n2 y\n").unwrap();

            // 测试标题行处理
            let settings = JoinSettings {
                key1: 0,
                key2: 0,
                is_print_joined: true,
                is_headers: true,
                separator: Sep::Whitespaces,
                ..Default::default()
            };

            let result = join_exec(
                file1_path.to_str().unwrap(),
                file2_path.to_str().unwrap(),
                settings,
            );
            assert!(result.is_ok());
        }

        #[test]
        fn test_join_exec_empty_files() {
            let temp = tempdir().unwrap();

            // 创建空文件
            let file1_path = temp.path().join("file1.txt");
            let file2_path = temp.path().join("file2.txt");

            create_test_file(&file1_path, "").unwrap();
            create_test_file(&file2_path, "").unwrap();

            let settings = JoinSettings::default();

            let result = join_exec(
                file1_path.to_str().unwrap(),
                file2_path.to_str().unwrap(),
                settings,
            );
            assert!(result.is_ok());
        }

        #[test]
        fn test_join_exec_invalid_files() {
            let settings = JoinSettings::default();

            // 测试不存在的文件
            let result = join_exec("nonexistent1.txt", "nonexistent2.txt", settings);
            assert!(result.is_err());
        }

        #[test]
        fn test_join_exec_custom_format() {
            let temp = tempdir().unwrap();

            // 创建测试文件
            let file1_path = temp.path().join("file1.txt");
            let file2_path = temp.path().join("file2.txt");

            create_test_file(&file1_path, "1 a\n2 b\n").unwrap();
            create_test_file(&file2_path, "1 x\n2 y\n").unwrap();

            // 测试自定义输出格式
            let settings = JoinSettings {
                key1: 0,
                key2: 0,
                is_print_joined: true,
                format: vec![JoinSpec::Key, JoinSpec::Field(FileNum::File2, 1)],
                separator: Sep::Whitespaces,
                ..Default::default()
            };

            let result = join_exec(
                file1_path.to_str().unwrap(),
                file2_path.to_str().unwrap(),
                settings,
            );
            assert!(result.is_ok());
        }
    }
}
