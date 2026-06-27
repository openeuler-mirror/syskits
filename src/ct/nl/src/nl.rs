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

//! nl - 向指定的各个 <文件> 添加行号，并写到标准输出。

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo, set_ct_exit_code};
use ctcore::{Args, ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show_error};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, stdin, stdout};
use std::path::Path;

const NL_ABOUT: &str = ct_help_about!("nl.md");
const NL_AFTER_HELP: &str = ct_help_section!("after help", "nl.md");
const NL_USAGE: &str = ct_help_usage!("nl.md");

/// 行号格式化工具的标志和选项
pub mod nl_flags {
    // 帮助信息标志
    pub const NL_HELP: &str = "help";
    // 输入文件路径
    pub const NL_FILE: &str = "file";
    // 正文部分的编号样式
    pub const NL_BODY_NUMBERING: &str = "body-numbering";
    // 分节符
    pub const NL_SECTION_DELIMITER: &str = "section-delimiter";
    // 页脚部分的编号样式
    pub const NL_FOOTER_NUMBERING: &str = "footer-numbering";
    // 页眉部分的编号样式
    pub const NL_HEADER_NUMBERING: &str = "header-numbering";
    // 行号增量
    pub const NL_LINE_INCREMENT: &str = "line-increment";
    // 合并空行的数量
    pub const NL_JOIN_BLANK_LINES: &str = "join-blank-lines";
    // 行号格式
    pub const NL_NUMBER_FORMAT: &str = "number-ct_format";
    // 是否重新编号
    pub const NL_NO_RENUMBER: &str = "no-renumber";
    // 行号分隔符
    pub const NL_NUMBER_SEPARATOR: &str = "number-separator";
    // 起始行号
    pub const NL_STARTING_LINE_NUMBER: &str = "starting-line-number";
    // 行号宽度
    pub const NL_NUMBER_WIDTH: &str = "number-width";
}

/// 行号格式化工具的配置结构
pub struct NlFlags {
    // 页眉、正文和页脚的编号样式
    header_numbering: NlNumberingStyle,
    body_numbering: NlNumberingStyle,
    footer_numbering: NlNumberingStyle,
    // 分节符
    section_delimiter: String,
    // 起始行号、增量、合并空行数和行号宽度
    starting_line_number: i64,
    line_increment: i64,
    join_blank_lines: u64,
    number_width: usize,
    // 行号格式和重新编号标志
    number_format: NlNumberFormat,
    is_renumber: bool,
    // 行号分隔符
    number_separator: String,
    // 要处理的文件列表
    files: Vec<String>,
    // 状态控制
    stats: NlStats,
}

impl Default for NlFlags {
    fn default() -> Self {
        Self {
            header_numbering: NlNumberingStyle::None,
            body_numbering: NlNumberingStyle::NonEmpty,
            footer_numbering: NlNumberingStyle::None,
            section_delimiter: String::from("\\:"),
            starting_line_number: 1,
            line_increment: 1,
            join_blank_lines: 1,
            number_width: 6,
            number_format: NlNumberFormat::Right,
            is_renumber: true,
            number_separator: String::from("\t"),
            files: vec!["-".to_string()],
            stats: NlStats::new(1),
        }
    }
}

// 判断是否存在"-v" 如果存在， 后需要跟一个 数字 ，这个数字是正数或者负数，如果存在将其合成一个
fn standardize_nl_args(args: impl ctcore::Args) -> impl ctcore::Args {
    let mut vec = Vec::<OsString>::new();
    let args_vec: Vec<OsString> = args.collect();
    let mut i = 0;

    while i < args_vec.len() {
        let arg = &args_vec[i];
        let arg_str = arg.to_string_lossy();

        if arg_str == "-v" || arg_str == "--starting-line-number" {
            // 如果"-v"后面跟一个数字，将-v 和数字合成一个参数格式为"-v=数字"
            if i + 1 < args_vec.len()
                && args_vec[i + 1].to_string_lossy().starts_with('-')
                && args_vec[i + 1]
                    .to_string_lossy()
                    .chars()
                    .nth(1)
                    .is_some_and(|c| c.is_ascii_digit())
            {
                // 将"-v"或者--starting-line-number和数字合并成一个Ostring
                let mut v: OsString = arg_str.to_string().into();
                v.push("=");
                v.push(args_vec[i + 1].clone());
                vec.push(v);
                i += 1;
            } else {
                vec.push(arg.clone());
            }
        } else {
            vec.push(arg.clone());
        }
        i += 1;
    }

    vec.into_iter()
}

impl NlFlags {
    /// 从命令行参数创建新的NlFlags实例
    ///
    /// # 参数
    /// * `matches` - 命令行参数匹配结果
    ///
    /// # 返回值
    /// * `CTResult<Self>` - 配置实例或错误
    fn new(matches: ArgMatches) -> CTResult<Self> {
        let mut flags = Self::default();
        let mut errs: Vec<String> = vec![];

        // 提取文件路径参数
        flags.files = matches
            .get_many::<String>(nl_flags::NL_FILE)
            .map_or_else(|| vec!["-".to_string()], |v| v.cloned().collect());

        // 提取分隔符选项
        if let Some(delimiter) = matches.get_one::<String>(nl_flags::NL_SECTION_DELIMITER) {
            flags.section_delimiter = if delimiter.len() == 1 {
                format!("{delimiter}:")
            } else {
                delimiter.clone()
            };
        }

        // 提取行号分隔符
        if let Some(val) = matches.get_one::<String>(nl_flags::NL_NUMBER_SEPARATOR) {
            flags.number_separator.clone_from(val);
        }

        // 提取行号格式化选项
        flags.number_format = matches
            .get_one::<String>(nl_flags::NL_NUMBER_FORMAT)
            .map(Into::into)
            .unwrap_or_default();

        // 提取各种编号样式选项
        match matches
            .get_one::<String>(nl_flags::NL_HEADER_NUMBERING)
            .map(String::as_str)
            .map(TryInto::try_into)
        {
            None => {}
            Some(Ok(style)) => flags.header_numbering = style,
            Some(Err(message)) => errs.push(message),
        }

        match matches
            .get_one::<String>(nl_flags::NL_BODY_NUMBERING)
            .map(String::as_str)
            .map(TryInto::try_into)
        {
            None => {}
            Some(Ok(style)) => flags.body_numbering = style,
            Some(Err(message)) => errs.push(message),
        }

        match matches
            .get_one::<String>(nl_flags::NL_FOOTER_NUMBERING)
            .map(String::as_str)
            .map(TryInto::try_into)
        {
            None => {}
            Some(Ok(style)) => flags.footer_numbering = style,
            Some(Err(message)) => errs.push(message),
        }

        // 提取数值选项
        match matches.get_one::<usize>(nl_flags::NL_NUMBER_WIDTH) {
            None => {}
            Some(num) if *num > 0 => flags.number_width = *num,
            Some(_) => errs.push(String::from(
                "Invalid line number field width: '0': Numerical result out of range",
            )),
        }

        match matches.get_one::<u64>(nl_flags::NL_JOIN_BLANK_LINES) {
            None => {}
            Some(num) if *num > 0 => flags.join_blank_lines = *num,
            Some(_) => errs.push(String::from(
                "Invalid line number of blank lines: '0': Numerical result out of range",
            )),
        }

        if let Some(num) = matches.get_one::<i64>(nl_flags::NL_LINE_INCREMENT) {
            flags.line_increment = *num;
        }

        if let Some(num) = matches.get_one::<String>(nl_flags::NL_STARTING_LINE_NUMBER) {
            // 如果num 是数字，则转换为i64
            if let Ok(num) = num.parse::<i64>() {
                flags.starting_line_number = num;
            }
        }

        // 提取重新编号选项
        flags.is_renumber = matches.get_flag(nl_flags::NL_NO_RENUMBER);

        // 如果有错误，返回错误信息
        if !errs.is_empty() {
            return Err(CtSimpleError::new(
                1,
                format!("Invalid arguments supplied.\n{}", errs.join("\n")),
            ));
        }

        flags.stats = NlStats::new(flags.starting_line_number);
        Ok(flags)
    }
}

/// 行号格式化状态结构
struct NlStats {
    // 当前行号
    line_number: Option<i64>,
    // 连续空行计数
    consecutive_empty_lines: u64,
}

impl NlStats {
    /// 创建新的行号统计状态
    ///
    /// # 参数
    /// * `starting_line_number` - 起始行号
    ///
    /// # 返回值
    /// * `Self` - 统计状态实例
    fn new(starting_line_number: i64) -> Self {
        Self {
            line_number: Some(starting_line_number),
            consecutive_empty_lines: 0,
        }
    }
}

/// 行号编号样式枚举
#[derive(Clone, Debug)]
enum NlNumberingStyle {
    /// 对所有行编号
    All,
    /// 只对非空行编号
    NonEmpty,
    /// 不编号
    None,
    /// 使用正则表达式匹配的行编号
    Regex(Box<regex::Regex>),
}

impl Eq for NlNumberingStyle {}

impl PartialEq for NlNumberingStyle {
    fn eq(&self, other: &Self) -> bool {
        use NlNumberingStyle::*;
        match (self, other) {
            (All, All) | (NonEmpty, NonEmpty) | (None, None) => true,
            (Regex(re1), Regex(re2)) => re1.as_str() == re2.as_str(),
            _ => false,
        }
    }
}

impl TryFrom<&str> for NlNumberingStyle {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "a" => Ok(Self::All),
            "t" => Ok(Self::NonEmpty),
            "n" => Ok(Self::None),
            _ if s.starts_with('p') => match regex::Regex::new(&s[1..]) {
                Ok(re) => Ok(Self::Regex(Box::new(re))),
                Err(_) => Err(String::from("invalid regular expression")),
            },
            _ => Err(format!("invalid numbering style: '{s}'")),
        }
    }
}

/// 行号格式枚举
#[derive(Default, Clone, PartialEq, Eq, Debug)]
enum NlNumberFormat {
    /// 左对齐
    Left,
    /// 右对齐(默认)
    #[default]
    Right,
    /// 右对齐并用零填充
    RightZero,
}

impl<T: AsRef<str>> From<T> for NlNumberFormat {
    fn from(s: T) -> Self {
        match s.as_ref() {
            "ln" => Self::Left,
            "rn" => Self::Right,
            "rz" => Self::RightZero,
            _ => unreachable!("Should have been caught by clap"),
        }
    }
}

impl NlNumberFormat {
    /// 格式化行号
    ///
    /// # 参数
    /// * `number` - 要格式化的行号
    /// * `min_width` - 最小宽度
    ///
    /// # 返回值
    /// * `String` - 格式化后的行号字符串
    fn format(&self, number: i64, min_width: usize) -> String {
        match self {
            Self::Left => format!("{number:<min_width$}"),
            Self::Right => format!("{number:>min_width$}"),
            Self::RightZero if number < 0 => format!("-{0:0>1$}", number.abs(), min_width - 1),
            Self::RightZero => format!("{number:0>min_width$}"),
        }
    }
}

/// 分节符类型枚举
enum NlSectionDelimiter {
    /// 页眉分隔符
    Header,
    /// 正文分隔符
    Body,
    /// 页脚分隔符
    Footer,
}

impl NlSectionDelimiter {
    /// 解析分节符
    ///
    /// # 参数
    /// * `s` - 要解析的字符串
    /// * `pattern` - 分节符模式
    ///
    /// # 返回值
    /// * `Option<Self>` - 分节符类型或None
    fn parse(s: &str, pattern: &str) -> Option<Self> {
        if s.is_empty() || pattern.is_empty() {
            return None;
        }

        let pattern_count = s.matches(pattern).count();
        let is_length_ok = pattern_count * pattern.len() == s.len();

        match (pattern_count, is_length_ok) {
            (3, true) => Some(Self::Header),
            (2, true) => Some(Self::Body),
            (1, true) => Some(Self::Footer),
            _ => None,
        }
    }
}

/// 主程序入口函数
///
/// # 参数
/// * `writer` - 输出写入器
/// * `args` - 命令行参数
///
/// # 返回值
/// * `CTResult<()>` - 处理结果
pub fn nl_main<W>(writer: &mut W, args: impl Args) -> CTResult<()>
where
    W: std::io::Write,
{
    // 使用标准化参数处理函数预处理参数
    let processed_args = standardize_nl_args(args);

    // 解析处理后的参数
    let matches = ct_app().try_get_matches_from(processed_args)?;
    let mut flags = NlFlags::new(matches)?;
    let files = flags.files.clone();
    let mut had_errors = false;

    for file in &files {
        if file == "-" {
            let mut buffer = BufReader::new(stdin());
            match nl(writer, &mut buffer, &mut flags) {
                Ok(_) => {}
                Err(e) => {
                    ct_show_error!("{}", e);
                    had_errors = true;
                }
            }
        } else {
            let path = Path::new(file.as_str());
            if path.is_dir() {
                ct_show_error!("{}: Is a directory", path.display());
                had_errors = true;
            } else {
                match File::open(path) {
                    Ok(reader) => {
                        let mut buffer = BufReader::new(reader);
                        match nl(writer, &mut buffer, &mut flags) {
                            Ok(_) => {}
                            Err(e) => {
                                ct_show_error!("{}", e);
                                had_errors = true;
                            }
                        }
                    }
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::NotFound {
                            ct_show_error!("{}: No such file or directory", file);
                        } else {
                            ct_show_error!("{}: {}", file, e);
                        }
                        had_errors = true;
                    }
                }
            }
        }
    }

    if had_errors {
        set_ct_exit_code(1);
    }

    Ok(())
}

/// 主函数入口点
#[ctcore::main]
pub fn ctmain(args: impl Args) -> CTResult<()> {
    let mut stdout = stdout();
    nl_main(&mut stdout, args)
}

/// 主要的行号格式化处理函数
///
/// # 参数
/// * `writer` - 输出写入器
/// * `reader` - 输入读取器
/// * `flags` - 格式化配置
///
/// # 返回值
/// * `CTResult<()>` - 处理结果
fn nl<R, W>(writer: &mut W, reader: &mut BufReader<R>, flags: &mut NlFlags) -> CTResult<()>
where
    R: std::io::Read,
    W: std::io::Write,
{
    let mut current_numbering_style = &flags.body_numbering;

    for line in reader.lines() {
        let line = line.map_err_context(|| "could not read line".to_string())?;

        if line.is_empty() {
            flags.stats.consecutive_empty_lines += 1;
        } else {
            flags.stats.consecutive_empty_lines = 0;
        };

        let new_numbering_style = match NlSectionDelimiter::parse(&line, &flags.section_delimiter) {
            Some(NlSectionDelimiter::Header) => Some(&flags.header_numbering),
            Some(NlSectionDelimiter::Body) => Some(&flags.body_numbering),
            Some(NlSectionDelimiter::Footer) => Some(&flags.footer_numbering),
            None => None,
        };

        if let Some(new_style) = new_numbering_style {
            current_numbering_style = new_style;
            if flags.is_renumber {
                flags.stats.line_number = Some(flags.starting_line_number);
            }
            writeln!(writer)?;
            continue;
        }

        let is_line_numbered = match current_numbering_style {
            NlNumberingStyle::All => {
                if line.is_empty() {
                    if flags.join_blank_lines > 0 {
                        flags.stats.consecutive_empty_lines % flags.join_blank_lines == 0
                    } else {
                        true
                    }
                } else {
                    true
                }
            }
            NlNumberingStyle::NonEmpty => !line.is_empty(),
            NlNumberingStyle::None => false,
            NlNumberingStyle::Regex(re) => re.is_match(&line),
        };

        if is_line_numbered {
            let Some(line_number) = flags.stats.line_number else {
                return Err(CtSimpleError::new(1, "line number overflow"));
            };
            writeln!(
                writer,
                "{}{}{}",
                flags.number_format.format(line_number, flags.number_width),
                flags.number_separator,
                line
            )?;
            match line_number.checked_add(flags.line_increment) {
                Some(new_line_number) => flags.stats.line_number = Some(new_line_number),
                None => flags.stats.line_number = None,
            }
        } else {
            let spaces = " ".repeat(flags.number_width + 1);
            writeln!(writer, "{spaces}{line}")?;
        }
    }
    Ok(())
}

/// 命令行应用程序配置函数
///
/// # 返回值
/// * `Command` - clap命令行配置
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = NL_USAGE;
    let usage_description = ct_format_usage(NL_ABOUT);
    let args = vec![
        Arg::new(nl_flags::NL_HELP)
            .long(nl_flags::NL_HELP)
            .help("Print help information.")
            .action(ArgAction::Help),
        Arg::new(nl_flags::NL_FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(nl_flags::NL_BODY_NUMBERING)
            .short('b')
            .long(nl_flags::NL_BODY_NUMBERING)
            .help("use STYLE for numbering body lines")
            .value_name("STYLE"),
        Arg::new(nl_flags::NL_SECTION_DELIMITER)
            .short('d')
            .long(nl_flags::NL_SECTION_DELIMITER)
            .help("use CC for separating logical pages")
            .value_name("CC"),
        Arg::new(nl_flags::NL_FOOTER_NUMBERING)
            .short('f')
            .long(nl_flags::NL_FOOTER_NUMBERING)
            .help("use STYLE for numbering footer lines")
            .value_name("STYLE"),
        Arg::new(nl_flags::NL_HEADER_NUMBERING)
            .short('h')
            .long(nl_flags::NL_HEADER_NUMBERING)
            .help("use STYLE for numbering header lines")
            .value_name("STYLE"),
        Arg::new(nl_flags::NL_LINE_INCREMENT)
            .short('i')
            .long(nl_flags::NL_LINE_INCREMENT)
            .help("line number increment at each line")
            .value_name("NUMBER")
            .value_parser(clap::value_parser!(i64)),
        Arg::new(nl_flags::NL_JOIN_BLANK_LINES)
            .short('l')
            .long(nl_flags::NL_JOIN_BLANK_LINES)
            .help("group of NUMBER empty lines counted as one")
            .value_name("NUMBER")
            .value_parser(clap::value_parser!(u64)),
        Arg::new(nl_flags::NL_NUMBER_FORMAT)
            .short('n')
            .long(nl_flags::NL_NUMBER_FORMAT)
            .help("insert line numbers according to FORMAT")
            .value_name("FORMAT")
            .value_parser(["ln", "rn", "rz"]),
        Arg::new(nl_flags::NL_NO_RENUMBER)
            .short('p')
            .long(nl_flags::NL_NO_RENUMBER)
            .help("do not reset line numbers at logical pages")
            .action(ArgAction::SetFalse),
        Arg::new(nl_flags::NL_NUMBER_SEPARATOR)
            .short('s')
            .long(nl_flags::NL_NUMBER_SEPARATOR)
            .help("add STRING after (possible) line number")
            .value_name("STRING"),
        Arg::new(nl_flags::NL_STARTING_LINE_NUMBER)
            .short('v')
            .long(nl_flags::NL_STARTING_LINE_NUMBER)
            .help("first line number on each logical page")
            .value_name("STRING")
            .num_args(1),
        Arg::new(nl_flags::NL_NUMBER_WIDTH)
            .short('w')
            .long(nl_flags::NL_NUMBER_WIDTH)
            .help("use NUMBER columns for line numbers")
            .value_name("NUMBER")
            .value_parser(clap::value_parser!(usize)),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .after_help(NL_AFTER_HELP)
        .disable_help_flag(true)
        .args(&args)
}

#[derive(Default)]
pub struct Nl;
impl Tool for Nl {
    fn name(&self) -> &'static str {
        "nl"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let mut stdout = stdout();
        nl_main(&mut stdout, args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::io::Cursor;

    #[test]
    fn test_tool_implementation() {
        let tool = Nl::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "nl");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("nl"));

        // 测试 execute 方法 - 帮助命令应该返回错误，但不会崩溃
        let args = vec![OsString::from("nl"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    /// 测试参数标准化函数
    #[test]
    fn test_standardize_nl_args() {
        // 测试 -v 后面跟负数的情况
        let args = vec![
            OsString::from("nl"),
            OsString::from("-v"),
            OsString::from("-10"),
            OsString::from("test.txt"),
        ];

        let processed: Vec<OsString> = standardize_nl_args(args.into_iter()).collect();
        assert_eq!(processed.len(), 3);
        assert_eq!(processed[0].to_string_lossy(), "nl");
        assert_eq!(processed[1].to_string_lossy(), "-v=-10");
        assert_eq!(processed[2].to_string_lossy(), "test.txt");

        // 测试 -v 后面跟负数的情况（有额外参数）
        let args = vec![
            OsString::from("nl"),
            OsString::from("-b"),
            OsString::from("a"),
            OsString::from("-v"),
            OsString::from("-10"),
            OsString::from("test.txt"),
        ];

        let processed: Vec<OsString> = standardize_nl_args(args.into_iter()).collect();
        assert_eq!(processed.len(), 5);
        assert_eq!(processed[0].to_string_lossy(), "nl");
        assert_eq!(processed[1].to_string_lossy(), "-b");
        assert_eq!(processed[2].to_string_lossy(), "a");
        assert_eq!(processed[3].to_string_lossy(), "-v=-10");
        assert_eq!(processed[4].to_string_lossy(), "test.txt");

        // 测试 --starting-line-number 后面跟负数的情况
        let args = vec![
            OsString::from("nl"),
            OsString::from("--starting-line-number"),
            OsString::from("-10"),
            OsString::from("test.txt"),
        ];

        let processed: Vec<OsString> = standardize_nl_args(args.into_iter()).collect();
        assert_eq!(processed.len(), 3);
        assert_eq!(processed[0].to_string_lossy(), "nl");
        assert_eq!(processed[1].to_string_lossy(), "--starting-line-number=-10");
        assert_eq!(processed[2].to_string_lossy(), "test.txt");

        // 测试已经带等号的情况
        let args = vec![
            OsString::from("nl"),
            OsString::from("-v=-10"),
            OsString::from("test.txt"),
        ];

        let processed: Vec<OsString> = standardize_nl_args(args.into_iter()).collect();
        assert_eq!(processed.len(), 3);
        assert_eq!(processed[0].to_string_lossy(), "nl");
        assert_eq!(processed[1].to_string_lossy(), "-v=-10");
        assert_eq!(processed[2].to_string_lossy(), "test.txt");

        // 测试 -v 后面跟非负数的情况
        let args = vec![
            OsString::from("nl"),
            OsString::from("-v"),
            OsString::from("10"),
            OsString::from("test.txt"),
        ];

        let processed: Vec<OsString> = standardize_nl_args(args.into_iter()).collect();
        assert_eq!(processed.len(), 4);
        assert_eq!(processed[0].to_string_lossy(), "nl");
        assert_eq!(processed[1].to_string_lossy(), "-v");
        assert_eq!(processed[2].to_string_lossy(), "10");
        assert_eq!(processed[3].to_string_lossy(), "test.txt");

        // 测试-v后面跟一个以-开头但不是数字的参数
        let args = vec![
            OsString::from("nl"),
            OsString::from("-v"),
            OsString::from("-abc"),
            OsString::from("test.txt"),
        ];

        let processed: Vec<OsString> = standardize_nl_args(args.into_iter()).collect();
        assert_eq!(processed.len(), 4);
        assert_eq!(processed[0].to_string_lossy(), "nl");
        assert_eq!(processed[1].to_string_lossy(), "-v");
        assert_eq!(processed[2].to_string_lossy(), "-abc");
        assert_eq!(processed[3].to_string_lossy(), "test.txt");

        // 测试普通参数不受影响
        let args = vec![
            OsString::from("nl"),
            OsString::from("-ft"),
            OsString::from("test.txt"),
        ];

        let processed: Vec<OsString> = standardize_nl_args(args.into_iter()).collect();

        assert_eq!(processed.len(), 3);
        assert_eq!(processed[0].to_string_lossy(), "nl");
        assert_eq!(processed[1].to_string_lossy(), "-ft");
        assert_eq!(processed[2].to_string_lossy(), "test.txt");
    }

    /// 测试NlFlags相关功能
    mod nl_flags_tests {
        use super::*;

        /// 创建测试用的命令行参数匹配
        fn create_test_matches(args: &[&str]) -> ArgMatches {
            ct_app().try_get_matches_from(args).unwrap()
        }

        /// 测试编号样式的解析
        #[test]
        fn test_flags_numbering_styles() {
            let test_cases = [
                ("a", Ok(NlNumberingStyle::All)),
                ("t", Ok(NlNumberingStyle::NonEmpty)),
                ("n", Ok(NlNumberingStyle::None)),
                (
                    "p[0-9]+",
                    Ok(NlNumberingStyle::Regex(Box::new(
                        regex::Regex::new("[0-9]+").unwrap(),
                    ))),
                ),
                (
                    "invalid",
                    Err("invalid numbering style: 'invalid'".to_string()),
                ),
            ];

            for (input, expected_result) in test_cases {
                let result = NlNumberingStyle::try_from(input);
                match (result, expected_result) {
                    (Ok(style), Ok(expected)) => {
                        assert_eq!(style, expected, "Failed for input: {}", input);
                    }
                    (Err(e), Err(expected)) => {
                        assert_eq!(e, expected, "Failed for input: {}", input);
                    }
                    (Ok(_), Err(_)) => {
                        panic!("Expected error but got success for input: {}", input);
                    }
                    (Err(_), Ok(_)) => {
                        panic!("Expected success but got error for input: {}", input);
                    }
                }
            }
        }

        /// 测试基本标志设置
        #[test]
        fn test_flags_basic() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "test.txt"]);
            let flags = NlFlags::new(matches).unwrap();

            assert_eq!(flags.line_increment, 1);
            assert_eq!(flags.join_blank_lines, 1);
            assert_eq!(flags.starting_line_number, 1);
            assert_eq!(flags.number_width, 6);
            assert_eq!(flags.files, vec!["test.txt"]);
            assert_eq!(flags.section_delimiter, "\\:");
            assert_eq!(flags.number_separator, "\t");
            assert!(matches!(flags.number_format, NlNumberFormat::Right));
            assert!(matches!(flags.header_numbering, NlNumberingStyle::None));
            assert!(matches!(flags.body_numbering, NlNumberingStyle::NonEmpty));
            assert!(matches!(flags.footer_numbering, NlNumberingStyle::None));
            assert!(flags.is_renumber);
        }

        /// 测试行号格式设置
        #[test]
        fn test_flags_number_format() {
            let test_cases = [
                ("ln", NlNumberFormat::Left),
                ("rn", NlNumberFormat::Right),
                ("rz", NlNumberFormat::RightZero),
            ];

            for (input, expected_format) in test_cases {
                let matches =
                    create_test_matches(&[ctcore::ct_util_name(), "-n", input, "test.txt"]);
                let flags = NlFlags::new(matches).unwrap();
                assert_eq!(
                    &flags.number_format, &expected_format,
                    "Failed for input: {}",
                    input
                );
            }
        }

        /// 测试分节符设置
        #[test]
        fn test_flags_section_delimiter() {
            let test_cases = [
                ("\\", "\\:"),    // 单字符自动添加 :
                ("\\\\", "\\\\"), // 多字符保持原样
                ("%%", "%%"),
            ];

            for (input, expected) in test_cases {
                let matches =
                    create_test_matches(&[ctcore::ct_util_name(), "-d", input, "test.txt"]);
                let flags = NlFlags::new(matches).unwrap();
                assert_eq!(flags.section_delimiter, expected);
            }
        }

        /// 测试数值选项设置
        #[test]
        fn test_flags_numeric_options() {
            let matches = create_test_matches(&[
                ctcore::ct_util_name(),
                "-i",
                "2", // line increment
                "-l",
                "3", // join blank lines
                "-v",
                "100", // starting line number
                "-w",
                "10", // number width
                "test.txt",
            ]);
            let flags = NlFlags::new(matches).unwrap();

            assert_eq!(flags.line_increment, 2);
            assert_eq!(flags.join_blank_lines, 3);
            assert_eq!(flags.starting_line_number, 100);
            assert_eq!(flags.number_width, 10);
        }

        /// 测试无效数值选项处理
        #[test]
        fn test_flags_invalid_numeric_options() {
            // 测试无效的数值选项
            let matches = create_test_matches(&[
                ctcore::ct_util_name(),
                "-w",
                "0", // invalid width
                "test.txt",
            ]);
            assert!(NlFlags::new(matches).is_err());

            let matches = create_test_matches(&[
                ctcore::ct_util_name(),
                "-l",
                "0", // invalid join blank lines
                "test.txt",
            ]);
            assert!(NlFlags::new(matches).is_err());
        }

        /// 测试多文件处理
        #[test]
        fn test_flags_multiple_files() {
            let matches = create_test_matches(&[
                ctcore::ct_util_name(),
                "file1.txt",
                "file2.txt",
                "file3.txt",
            ]);
            let flags = NlFlags::new(matches).unwrap();
            assert_eq!(flags.files, vec!["file1.txt", "file2.txt", "file3.txt"]);
        }

        /// 测试标准输入处理
        #[test]
        fn test_flags_default_stdin() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "-"]);
            let flags = NlFlags::new(matches).unwrap();
            assert_eq!(flags.files, vec!["-"]);
        }

        /// 测试编号样式选项设置
        #[test]
        fn test_flags_numbering_style_options() {
            let matches = create_test_matches(&[
                ctcore::ct_util_name(),
                "-h",
                "a", // header: all
                "-b",
                "t", // body: non-empty
                "-f",
                "n", // footer: none
                "test.txt",
            ]);
            let flags = NlFlags::new(matches).unwrap();

            assert!(matches!(flags.header_numbering, NlNumberingStyle::All));
            assert!(matches!(flags.body_numbering, NlNumberingStyle::NonEmpty));
            assert!(matches!(flags.footer_numbering, NlNumberingStyle::None));
        }

        /// 测试无效编号样式处理
        #[test]
        fn test_flags_invalid_numbering_style() {
            let matches =
                create_test_matches(&[ctcore::ct_util_name(), "-h", "invalid", "test.txt"]);
            assert!(NlFlags::new(matches).is_err());
        }

        /// 测试负数起始行号设置
        #[test]
        fn test_flags_negative_starting_line_number() {
            let processed_args = standardize_nl_args(
                [ctcore::ct_util_name(), "-v", "-10", "test.txt"]
                    .into_iter()
                    .map(|s| OsString::from(s)),
            );

            let matches = ct_app().try_get_matches_from(processed_args).unwrap();
            let flags = NlFlags::new(matches).unwrap();

            assert_eq!(flags.starting_line_number, -10);
        }

        /// 测试带等号的负数起始行号
        #[test]
        fn test_flags_negative_starting_line_number_with_equals() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "-v=-10", "test.txt"]);
            let flags = NlFlags::new(matches).unwrap();

            assert_eq!(flags.starting_line_number, -10);
        }

        /// 测试长选项名的负数起始行号
        #[test]
        fn test_flags_negative_starting_line_number_long_option() {
            let processed_args = standardize_nl_args(
                [
                    ctcore::ct_util_name(),
                    "--starting-line-number",
                    "-10",
                    "test.txt",
                ]
                .into_iter()
                .map(|s| OsString::from(s)),
            );

            let matches = ct_app().try_get_matches_from(processed_args).unwrap();
            let flags = NlFlags::new(matches).unwrap();

            assert_eq!(flags.starting_line_number, -10);
        }

        /// 测试极端负数值
        #[test]
        fn test_flags_extreme_negative_starting_line_number() {
            let processed_args = standardize_nl_args(
                [
                    ctcore::ct_util_name(),
                    "-v",
                    "-2147483648", // min value for i32
                    "test.txt",
                ]
                .into_iter()
                .map(|s| OsString::from(s)),
            );

            let matches = ct_app().try_get_matches_from(processed_args).unwrap();
            let flags = NlFlags::new(matches).unwrap();

            assert_eq!(flags.starting_line_number, -2147483648);
        }
    }

    /// 测试nl函数的核心功能
    mod nl_function_tests {
        use super::*;

        /// 测试基本行号功能
        #[test]
        fn test_basic_numbering() {
            let input = "Line 1\nLine 2\nLine 3\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1\tLine 1\n     2\tLine 2\n     3\tLine 3\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试空行处理
        #[test]
        fn test_empty_lines() {
            let input = "Line 1\n\n\nLine 4\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1\tLine 1\n       \n       \n     2\tLine 4\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试分节符处理
        #[test]
        fn test_section_delimiter() {
            let input = "\\:\nLine 1\n\\:\\:\nLine 2\n\\:\\:\\:\nLine 3\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "\n       Line 1\n\n     1\tLine 2\n\n       Line 3\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试行号格式化
        #[test]
        fn test_number_format() {
            let input = "Line 1\nLine 2\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.number_format = NlNumberFormat::RightZero;
            flags.number_width = 3;

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "001\tLine 1\n002\tLine 2\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试空行合并功能
        #[test]
        fn test_join_blank_lines() {
            let input = "Line 1\n\n\n\nLine 5\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.join_blank_lines = 2;
            flags.body_numbering = NlNumberingStyle::All;

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1\tLine 1\n       \n     2\t\n       \n     3\tLine 5\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试行号增量功能
        #[test]
        fn test_line_increment() {
            let input = "Line 1\nLine 2\nLine 3\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.line_increment = 2;

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1\tLine 1\n     3\tLine 2\n     5\tLine 3\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试起始行号设置
        #[test]
        fn test_starting_line_number() {
            let input = "Line 1\nLine 2\nLine 3\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = 100;

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1\tLine 1\n     2\tLine 2\n     3\tLine 3\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试行号分隔符
        #[test]
        fn test_number_separator() {
            let input = "Line 1\nLine 2\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.number_separator = String::from(" | ");

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1 | Line 1\n     2 | Line 2\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试正则表达式编号功能
        #[test]
        fn test_regex_numbering() {
            let input = "123\nabc\n456\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.body_numbering =
                NlNumberingStyle::Regex(Box::new(regex::Regex::new("[0-9]+").unwrap()));

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1\t123\n       abc\n     2\t456\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试全部编号样式
        #[test]
        fn test_all_numbering_style() {
            let input = "Line 1\n\nLine 3\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.body_numbering = NlNumberingStyle::All;

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1\tLine 1\n     2\t\n     3\tLine 3\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试无编号样式
        #[test]
        fn test_no_numbering_style() {
            let input = "Line 1\nLine 2\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.body_numbering = NlNumberingStyle::None;

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "       Line 1\n       Line 2\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试负数起始行号
        #[test]
        fn test_negative_starting_line_number() {
            let input = "Line 1\nLine 2\nLine 3\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = -10;
            flags.stats = NlStats::new(flags.starting_line_number);
            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "   -10\tLine 1\n    -9\tLine 2\n    -8\tLine 3\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试负数起始行号与行号增量
        #[test]
        fn test_negative_starting_line_number_with_increment() {
            let input = "Line 1\nLine 2\nLine 3\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = -10;
            flags.line_increment = 2;
            flags.section_delimiter = " ".to_string();
            flags.body_numbering = NlNumberingStyle::All;
            flags.stats = NlStats::new(flags.starting_line_number);
            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "   -10\tLine 1\n    -8\tLine 2\n    -6\tLine 3\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试负数起始行号与格式化
        #[test]
        fn test_negative_starting_line_number_with_formatting() {
            let input = "Line 1\nLine 2\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = -10;
            flags.number_format = NlNumberFormat::RightZero;
            flags.number_width = 3;

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "001\tLine 1\n002\tLine 2\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试负数起始行号跨越0边界
        #[test]
        fn test_negative_starting_line_number_crossing_zero() {
            let input = "Line 1\nLine 2\nLine 3\nLine 4\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = -2;
            flags.stats = NlStats::new(flags.starting_line_number);
            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "    -2\tLine 1\n    -1\tLine 2\n     0\tLine 3\n     1\tLine 4\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试负数行号左对齐
        #[test]
        fn test_negative_line_number_left_aligned() {
            let input = "Line 1\nLine 2\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = -10;
            flags.number_format = NlNumberFormat::Left;
            flags.number_width = 5;
            flags.stats = NlStats::new(flags.starting_line_number);
            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "-10  \tLine 1\n-9   \tLine 2\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }
    }

    /// 测试特殊边界情况
    mod nl_edge_cases {
        use super::*;

        /// 测试空输入
        #[test]
        fn test_empty_input_with_negative_starting_line() {
            let input = "";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = -10;

            nl(&mut output, &mut reader, &mut flags).unwrap();

            // 空输入不应该输出任何内容
            let expected = "";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试只有空行的输入
        #[test]
        fn test_only_empty_lines() {
            let input = "\n\n\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.body_numbering = NlNumberingStyle::All; // 对所有行编号

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1\t\n     2\t\n     3\t\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试负数起始行号与大的行号增量
        #[test]
        fn test_negative_starting_line_with_large_increment() {
            let input = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = -100;
            flags.line_increment = 50;
            flags.stats = NlStats::new(flags.starting_line_number);
            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected =
                "  -100\tLine 1\n   -50\tLine 2\n     0\tLine 3\n    50\tLine 4\n   100\tLine 5\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试负数起始行号与较窄宽度
        #[test]
        fn test_negative_starting_line_with_narrow_width() {
            let input = "Line 1\nLine 2\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = -100;
            flags.number_width = 3; // 宽度比行号短
            flags.stats = NlStats::new(flags.starting_line_number);
            nl(&mut output, &mut reader, &mut flags).unwrap();

            // 行号应该超出设定的宽度
            let expected = "-100\tLine 1\n-99\tLine 2\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试负行号增量
        #[test]
        fn test_with_negative_line_increment() {
            let input = "Line 1\nLine 2\nLine 3\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = 5;
            flags.line_increment = -1; // 负数增量

            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "     1\tLine 1\n     0\tLine 2\n    -1\tLine 3\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }

        /// 测试负起始行号与负行号增量
        #[test]
        fn test_negative_starting_line_with_negative_increment() {
            let input = "Line 1\nLine 2\nLine 3\n";
            let mut output = Vec::new();
            let mut reader = BufReader::new(Cursor::new(input));
            let mut flags = NlFlags::default();
            flags.starting_line_number = -5;
            flags.line_increment = -1; // 负数增量
            flags.stats = NlStats::new(flags.starting_line_number);
            nl(&mut output, &mut reader, &mut flags).unwrap();

            let expected = "    -5\tLine 1\n    -6\tLine 2\n    -7\tLine 3\n";
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }
    }
}
