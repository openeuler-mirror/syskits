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

// expand 命令在Linux和类UNIX系统中用于将文本文件中的制表符转换为一系列空格。它的主要作用是规范化文本，尤其是在代码编辑和排版时，确保不同系统和编辑器下显示的一致性。
// 以下是 expand 命令的一些关键作用和选项：
// 制表符转换：
//     默认情况下，expand 把每个制表符转换为8个空格。
//     用户可以通过 -t 或 --tabs 选项指定每个制表符应转换为的空格数。
// 标准输入和输出：
//     如果没有提供文件名，expand 会从标准输入读取数据并将其转换后输出到标准输出。
//     使用管道（|）可以将其他命令的输出传递给 expand 进行处理。
// 选项：
//     -i 或 --initial：不转换非空白字符之后的制表符。
//     -t NUMBER：指定每个制表符转换为 NUMBER 个空格。
//     -t LIST：定义一系列不同位置的制表符停靠点，用逗号分隔。
//     --help：显示命令的帮助信息。
//     --version：输出命令的版本信息。
// 应用场景：
//     在源代码控制或协作环境中，统一代码缩进风格。
//     当需要在不同配置的终端或编辑器中保持一致的显示效果时。
//     配合其他文本处理工具（如grep, sed, awk等）进行文本分析和转换。
// 与其他命令结合：
//     可以通过重定向（>）将转换后的输出保存到文件。
//     可以与其他命令组合，例如 cat file.txt | expand -t 4 | less 会显示制表符被转换为4个空格的文本内容。

extern crate rust_i18n;
use clap::Arg;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::ArgAction;
use clap::ArgMatches;
use clap::Command;
use clap::crate_version;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTError;
use ctcore::ct_error::CTResult;
use ctcore::ct_error::FromIo;
use ctcore::ct_error::set_ct_exit_code;

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::io::stdout;
use std::num::IntErrorKind;
use std::path::Path;
use std::str::from_utf8;
use sys_locale::get_locale;
use unicode_width::UnicodeWidthChar;

use ctcore::Tool;

pub mod opt_flags {
    pub static TABS: &str = "tabs";
    pub static INITIAL: &str = "initial";
    pub static NO_UTF8: &str = "no-utf8";
    pub static FILES: &str = "FILES";
}

static LONG_HELP: &str = "";

static DEFAULT_TABSTOP: usize = 8;
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// The mode to use when replacing tabs beyond the last one specified in
/// the `--tabs` argument.
#[derive(PartialEq, Debug)]
enum RemainingMode {
    None,
    Slash,
    Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpandTabstopMode {
    None,
    Slash,
    Plus,
}

#[derive(PartialEq, Eq, Debug)]
enum CharType {
    Backspace,
    Tab,
    Other,
}

#[derive(Debug, PartialEq, Eq)]
struct ExpandState {
    column: usize,
    is_init: bool,
    pending_utf8: Vec<u8>,
    line_had_tabs: Vec<bool>,
    current_line_has_tabs: bool,
    current_line_has_content: bool,
}

impl Default for ExpandState {
    fn default() -> Self {
        Self {
            column: 0,
            is_init: true,
            pending_utf8: Vec::new(),
            line_had_tabs: Vec::new(),
            current_line_has_tabs: false,
            current_line_has_content: false,
        }
    }
}

enum ExpandCharInfo {
    Parsed {
        c_type: CharType,
        c_width: usize,
        n_bytes: usize,
    },
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandRow {
    pub row_index: usize,
    pub line: String,
    pub had_tabs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandSemantic {
    pub tabstop_mode: ExpandTabstopMode,
    pub tabstops: Vec<usize>,
    pub initial_only: bool,
    pub assume_utf8: bool,
    pub rows: Vec<ExpandRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct ExpandRunOutcome {
    stderr_text: String,
    exit_code: i32,
    line_had_tabs: Vec<bool>,
}

/// Decide whether the character is either a space or a comma.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_space_or_comma(' '))
/// assert!(is_space_or_comma(','))
/// assert!(!is_space_or_comma('a'))
/// ```
fn is_space_or_comma(c: char) -> bool {
    c == ' ' || c == ','
}

/// Decide whether the character is either a digit or a comma.
fn is_digit_or_comma(c: char) -> bool {
    c.is_ascii_digit() || c == ','
}

/// Errors that can occur when parsing a `--tabs` argument.
#[derive(Debug, PartialEq)]
enum ExpandParseError {
    InvalidCharacter(String),
    SpecifierNotAtStartOfNumber(String, String),
    SpecifierOnlyAllowedWithLastValue(String),
    TabSizeCannotBeZero,
    TabSizeTooLarge(String),
    TabSizesMustBeAscending,
}

impl Error for ExpandParseError {}
impl CTError for ExpandParseError {}

impl fmt::Display for ExpandParseError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::InvalidCharacter(s) => {
                write!(fmt, "tab size contains invalid character(s): {}", s.quote())
            }
            Self::SpecifierNotAtStartOfNumber(specifier, s) => write!(
                fmt,
                "{} specifier not at start of number: {}",
                specifier.quote(),
                s.quote(),
            ),
            Self::SpecifierOnlyAllowedWithLastValue(specifier) => write!(
                fmt,
                "{} specifier only allowed with the last value",
                specifier.quote()
            ),
            Self::TabSizeCannotBeZero => write!(fmt, "tab size cannot be 0"),
            Self::TabSizeTooLarge(s) => write!(fmt, "tab stop is too large {}", s.quote()),
            Self::TabSizesMustBeAscending => write!(fmt, "tab sizes must be ascending"),
        }
    }
}

/// 表示应用程序的配置选项。
///
/// 此结构体包括输入文件、制表位位置以及制表符和空格扩展的选项等设置。
struct ExpandOptions {
    files: Vec<String>,   // 要处理的文件名列表
    tabstops: Vec<usize>, // 制表位应停止的位置
    tspaces: String,      // 用于制表符扩展的空格字符串
    iflag: bool,          // 标志，表示是否应用初始状态
    uflag: bool,          // 标志，表示是否假设UTF-8编码

    /// 确定在超出指定 `tabstops` 的列中的制表符如何展开。
    remaining_mode: RemainingMode,
}

fn expand_tabstop_mode(remaining_mode: &RemainingMode) -> ExpandTabstopMode {
    match remaining_mode {
        RemainingMode::None => ExpandTabstopMode::None,
        RemainingMode::Slash => ExpandTabstopMode::Slash,
        RemainingMode::Plus => ExpandTabstopMode::Plus,
    }
}

fn finish_expand_line(state: &mut ExpandState) {
    state.line_had_tabs.push(state.current_line_has_tabs);
    state.current_line_has_tabs = false;
    state.current_line_has_content = false;
}

fn expand_rows_from_output(output: &str, line_had_tabs: &[bool]) -> Vec<ExpandRow> {
    output
        .split_terminator('\n')
        .enumerate()
        .map(|(index, line)| ExpandRow {
            row_index: index + 1,
            line: line.to_string(),
            had_tabs: line_had_tabs.get(index).copied().unwrap_or(false),
        })
        .collect()
}

impl ExpandOptions {
    /// 从命令行参数构建一个新的 `Options` 实例。
    ///
    /// 从提供的 `ArgMatches` 解析以提取选项，如制表位、输入文件和标志。它准备了根据指定选项处理文件所需的配置。
    ///
    /// - `matches`: 由命令行参数解析器解析的参数。
    ///
    /// 返回配置好的 `Options` 实例，或在参数解析失败时返回错误。
    fn new(args_match: &ArgMatches) -> Result<Self, ExpandParseError> {
        // 从命令行解析自定义制表位，或使用默认值。
        let (remaining_mode, tabstops) = match args_match.get_many::<String>(opt_flags::TABS) {
            Some(s) => expand_tabstops_parse(&s.map(|s| s.as_str()).collect::<Vec<_>>().join(","))?,
            None => (RemainingMode::None, vec![DEFAULT_TABSTOP]),
        };

        // 从命令行参数中提取初始和UTF-8标志。
        let is_iflag = args_match.get_flag(opt_flags::INITIAL);
        let is_uflag = !args_match.get_flag(opt_flags::NO_UTF8);

        // 预计算制表符扩展所需的最长空格数，以避免处理过程中重复分配。
        let nspaces = tabstops
            .iter()
            .scan(0, |pr, &it| {
                let ret = Some(it - *pr);
                *pr = it;
                ret
            })
            .max()
            .unwrap(); // 我们保证 `tabstops` 至少有一个元素。
        let tspaces = " ".repeat(nspaces);

        // 收集要处理的文件列表。如果没有指定文件，则默认使用标准输入。
        let files: Vec<String> = match args_match.get_many::<String>(opt_flags::FILES) {
            Some(s) => s.map(|v| v.to_string()).collect(),
            None => vec!["-".to_owned()],
        };

        Ok(Self {
            files,
            tabstops,
            tspaces,
            iflag: is_iflag,
            uflag: is_uflag,
            remaining_mode,
        })
    }
}

pub fn expand_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app().try_get_matches_from(expand_shortcuts(args.collect()))?;

    expand(&ExpandOptions::new(&args_match)?)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("expand.about");
    let usage_description = t!("expand.usage");

    let args = vec![
        Arg::new(opt_flags::INITIAL)
            .long(opt_flags::INITIAL)
            .short('i')
            .help(t!("expand.clap.initial"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::TABS)
            .long(opt_flags::TABS)
            .short('t')
            .value_name("N, LIST")
            .action(ArgAction::Append)
            .help(
                "have tabs N characters apart, not 8 or use comma separated list \
                    of explicit tab positions",
            ),
        Arg::new(opt_flags::NO_UTF8)
            .long(opt_flags::NO_UTF8)
            .short('U')
            .help(t!("expand.clap.no_utf8"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::FILES)
            .action(ArgAction::Append)
            .hide(true)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .after_help(LONG_HELP)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args_override_self(true)
        .args(&args)
}

/// Preprocess command line arguments and expand shortcuts. For example, "-7" is expanded to
/// "--tabs=7" and "-1,3" to "--tabs=1 --tabs=3".
fn expand_shortcuts(args: Vec<OsString>) -> Vec<OsString> {
    let mut processed_args = Vec::with_capacity(args.len());

    for arg in args {
        if let Some(arg) = arg.to_str() {
            if arg.starts_with('-') && arg[1..].chars().all(is_digit_or_comma) {
                arg[1..]
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .for_each(|s| processed_args.push(OsString::from(format!("--tabs={s}"))));
                continue;
            }
        }
        processed_args.push(arg);
    }

    processed_args
}

/// Parse a list of tabstops from a `--tabs` argument.
///
/// This function returns both the vector of numbers appearing in the
/// comma- or space-separated list, and also an optional mode, specified
/// by either a "/" or a "+" character appearing before the final number
/// in the list. This mode defines the strategy to use for computing the
/// number of spaces to use for columns beyond the end of the tab stop
/// list specified here.
/**
 * 解析给定的字符串来设置制表位。
 *
 * 此函数接受一个字符串引用，解析它以确定制表位的设置。制表位可以以空格或逗号分隔的数字列表的形式指定，
 * 并且可以使用 "+" 或 "/" 修饰符来指定后续制表位的相对或绝对大小。
 *
 * @param s 字符串引用，包含要解析的制表位设置。
 * @return Result<(RemainingMode, Vec<usize>), ParseError> 解析成功时返回一个元组，包含剩余模式（None、Plus 或 Slash）和制表位位置的向量；
 * 错误时返回 ParseError。
 *
 * 解析过程忽略开头的空格和逗号。如果字符串仅包含空格和逗号，则使用默认的制表位设置。
 * 在解析数字时，会检查制表位大小是否为正，以及是否递增。此外，"+" 或 "/" 修饰符只能与列表中的最后一个数字一起使用。
 */
fn expand_tabstops_parse(s: &str) -> Result<(RemainingMode, Vec<usize>), ExpandParseError> {
    // 忽略开头的空格和逗号
    let str = s.trim_start_matches(is_space_or_comma);

    // 如果字符串为空，则使用默认制表位
    if str.is_empty() {
        return Ok((RemainingMode::None, vec![DEFAULT_TABSTOP]));
    }

    // 初始化制表位列表和剩余模式
    let mut numbers = vec![];
    let mut remaining_mode = RemainingMode::None;
    let mut is_specifier_already_used = false;

    // 解析每个由空格或逗号分隔的单词
    for word in str.split(is_space_or_comma) {
        let bytes = word.as_bytes();
        for index in 0..bytes.len() {
            match bytes[index] {
                b'+' => remaining_mode = RemainingMode::Plus,
                b'/' => remaining_mode = RemainingMode::Slash,
                _ => {
                    // 从字节序列解析数字
                    let s = from_utf8(&bytes[index..]).unwrap();
                    match s.parse::<usize>() {
                        Ok(num) => {
                            // 检查制表位大小是否为正，是否递增
                            if num == 0 {
                                return Err(ExpandParseError::TabSizeCannotBeZero);
                            }
                            if let Some(last_stop) = numbers.last() {
                                if *last_stop >= num {
                                    return Err(ExpandParseError::TabSizesMustBeAscending);
                                }
                            }

                            // 检查是否已使用修饰符，以及是否只能与最后一个值一起使用
                            if is_specifier_already_used {
                                let specifier = if remaining_mode == RemainingMode::Slash {
                                    "/".to_string()
                                } else {
                                    "+".to_string()
                                };
                                return Err(ExpandParseError::SpecifierOnlyAllowedWithLastValue(
                                    specifier,
                                ));
                            } else if remaining_mode != RemainingMode::None {
                                is_specifier_already_used = true;
                            }

                            // 将制表位添加到列表中
                            numbers.push(num);
                            break;
                        }
                        Err(e) => {
                            // 处理解析错误，如数值过大或字符非法
                            if *e.kind() == IntErrorKind::PosOverflow {
                                return Err(ExpandParseError::TabSizeTooLarge(s.to_string()));
                            }

                            let s = s.trim_start_matches(char::is_numeric);
                            if s.starts_with('/') || s.starts_with('+') {
                                return Err(ExpandParseError::SpecifierNotAtStartOfNumber(
                                    s[0..1].to_string(),
                                    s.to_string(),
                                ));
                            } else {
                                return Err(ExpandParseError::InvalidCharacter(s.to_string()));
                            }
                        }
                    }
                }
            }
        }
    }

    // 如果未解析任何数字，则使用默认制表位
    if numbers.is_empty() {
        numbers = vec![DEFAULT_TABSTOP];
    }

    // 如果制表位数量小于2，则重置剩余模式为 None
    if numbers.len() < 2 {
        remaining_mode = RemainingMode::None;
    }
    Ok((remaining_mode, numbers))
}

fn expand_open(file_path: &str) -> CTResult<BufReader<Box<dyn Read + 'static>>> {
    let file_buffer;
    if file_path == "-" {
        Ok(BufReader::new(ctcore::ct_io::stdin_reader_box()))
    } else {
        file_buffer = File::open(file_path).map_err_context(|| file_path.to_string())?;
        Ok(BufReader::new(Box::new(file_buffer) as Box<dyn Read>))
    }
}

/// Compute the number of spaces to the next tabstop.
///
/// `tabstops` is the sequence of tabstop locations.
///
/// `col` is the index of the current cursor in the line being written.
///
/// If `remaining_mode` is [`RemainingMode::Plus`], then the last entry
/// in the `tabstops` slice is interpreted as a relative number of
/// spaces, which this function will return for every input value of
/// `col` beyond the end of the second-to-last element of `tabstops`.
/**
 * 计算到达下一个制表位的字符数。
 *
 * 此函数根据提供的制表位集合、当前列位置以及剩余模式（如何处理到达最后一个制表位后的字符），
 * 来计算从当前列到下一个制表位的字符数。
 *
 * @param tabstops 制表位的位置集合，以字节为单位。
 * @param col 当前列的位置，以字节为单位。
 * @param remaining_mode 剩余模式，决定了如何处理到达最后一个制表位后的字符。
 * @return 返回从当前列到下一个制表位的字符数。
 */
fn expand_next_tabstop(tabstops: &[usize], colum: usize, remaining_mode: &RemainingMode) -> usize {
    let number_tabstops = tabstops.len();

    // 根据不同的剩余模式处理逻辑
    match remaining_mode {
        RemainingMode::Plus => {
            // 在当前列之后找到第一个制表位，计算距离；如果没有找到，则按照最后一个制表位的步长计算
            match tabstops[0..number_tabstops - 1]
                .iter()
                .find(|&&t| t > colum)
            {
                Some(t) => t - colum,
                None => {
                    let step_size = tabstops[number_tabstops - 1];
                    let last_fixed_tabstop = tabstops[number_tabstops - 2];
                    let characters_since_last_tabstop = colum - last_fixed_tabstop;

                    // 计算需要多少步到达下一个制表位，并计算对应的字符数
                    let steps_required = 1 + characters_since_last_tabstop / step_size;
                    steps_required * step_size - characters_since_last_tabstop
                }
            }
        }
        RemainingMode::Slash => {
            // 在当前列之后找到第一个制表位，计算距离；如果没有找到，则按照最后一个制表位的模运算来计算
            match tabstops[0..number_tabstops - 1]
                .iter()
                .find(|&&t| t > colum)
            {
                Some(t) => t - colum,
                None => tabstops[number_tabstops - 1] - colum % tabstops[number_tabstops - 1],
            }
        }
        RemainingMode::None => {
            // 如果只有一个制表位，直接按照该制表位计算；如果有多个，找到第一个大于当前列的制表位，或者返回1
            if number_tabstops == 1 {
                tabstops[0] - colum % tabstops[0]
            } else {
                match tabstops.iter().find(|&&t| t > colum) {
                    Some(t) => t - colum,
                    None => 1,
                }
            }
        }
    }
}

/// 扩展行
///
/// 此函数用于根据给定的设置和选项扩展或压缩缓冲区中的文本行。它处理制表符和退格符，并根据指定的选项进行扩展或保留。
///
/// # 参数
/// - `buf`: 指向需要处理的字节缓冲区的 mutable 引用。
/// - `output`: 指向 `BufWriter<std::io::Stdout>` 的 mutable 引用，用于输出处理后的文本。
/// - `tabstops`: 一个包含制表符停靠位置的 slice。
/// - `options`: 指向 `Options` 结构体的引用，包含各种处理选项，如是否扩展制表符、如何处理剩余字符等。
///
/// # 返回值
/// 返回一个 `std::io::Result<()>`，表示操作是否成功完成。如果遇到 I/O 错误，则返回相应的错误结果。
fn utf8_sequence_len(first_byte: u8) -> Option<usize> {
    match first_byte {
        0x00..=0x7F => Some(1),
        0xC2..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF4 => Some(4),
        _ => None,
    }
}

fn expand_char_info_from_char(ch: char) -> ExpandCharInfo {
    let n_bytes = ch.len_utf8();
    let c_type = if ch == '\t' {
        CharType::Tab
    } else if ch == '\x08' {
        CharType::Backspace
    } else {
        CharType::Other
    };
    let c_width = if matches!(c_type, CharType::Tab | CharType::Backspace) {
        0
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0)
    };
    ExpandCharInfo::Parsed {
        c_type,
        c_width,
        n_bytes,
    }
}

fn expand_next_char_info(is_u_flag: bool, buf: &[u8], byte: usize) -> ExpandCharInfo {
    if !is_u_flag {
        let c_type = match buf[byte] {
            0x09 => CharType::Tab,
            0x08 => CharType::Backspace,
            _ => CharType::Other,
        };
        return ExpandCharInfo::Parsed {
            c_type,
            c_width: 1,
            n_bytes: 1,
        };
    }

    let slice = &buf[byte..];
    match from_utf8(slice) {
        Ok(s) => match s.chars().next() {
            Some(ch) => expand_char_info_from_char(ch),
            None => ExpandCharInfo::Parsed {
                c_type: CharType::Other,
                c_width: 1,
                n_bytes: 1,
            },
        },
        Err(e) => {
            let valid = e.valid_up_to();
            if valid > 0 {
                let prefix = from_utf8(&slice[..valid]).unwrap_or_default();
                if let Some(ch) = prefix.chars().next() {
                    return expand_char_info_from_char(ch);
                }
            }

            if e.error_len().is_none()
                && utf8_sequence_len(slice[0]).is_some_and(|expected| expected > slice.len())
            {
                return ExpandCharInfo::Incomplete;
            }

            ExpandCharInfo::Parsed {
                c_type: CharType::Other,
                c_width: 1,
                n_bytes: 1,
            }
        }
    }
}

fn expand_raw_bytes(
    buffer: &[u8],
    output: &mut impl Write,
    state: &mut ExpandState,
) -> std::io::Result<()> {
    for &byte in buffer {
        if byte == b'\n' {
            state.column = 0;
            state.is_init = true;
            finish_expand_line(state);
        } else {
            state.column = state.column.saturating_add(1);
            state.current_line_has_content = true;
            if byte == b'\t' {
                state.current_line_has_tabs = true;
            }
            if byte != b' ' {
                state.is_init = false;
            }
        }
        output.write_all(&[byte])?;
    }
    Ok(())
}

#[allow(clippy::cognitive_complexity)]
fn expand_line(
    buffer: &[u8],
    output: &mut impl Write,
    tabstops: &[usize],
    opts: &ExpandOptions,
    state: &mut ExpandState,
) -> std::io::Result<()> {
    use self::CharType::*;

    let mut byte = 0;

    // 遍历缓冲区中的每个字符。
    while byte < buffer.len() {
        let (c_type, c_width, n_bytes) = match expand_next_char_info(opts.uflag, buffer, byte) {
            ExpandCharInfo::Parsed {
                c_type,
                c_width,
                n_bytes,
            } => (c_type, c_width, n_bytes),
            ExpandCharInfo::Incomplete => {
                state.pending_utf8.extend_from_slice(&buffer[byte..]);
                break;
            }
        };

        // 根据字符类型更新列数并输出相应字符。
        match c_type {
            Tab => {
                // 计算到下一个制表位需要多少空格。
                let nts = expand_next_tabstop(tabstops, state.column, &opts.remaining_mode);
                state.column += nts;
                state.current_line_has_tabs = true;
                state.current_line_has_content = true;

                // 根据选项扩展制表符为空格或保留制表符。
                if state.is_init || !opts.iflag {
                    if nts <= opts.tspaces.len() {
                        output.write_all(&opts.tspaces.as_bytes()[..nts])?;
                    } else {
                        output.write_all(" ".repeat(nts).as_bytes())?;
                    };
                } else {
                    output.write_all(&buffer[byte..byte + n_bytes])?;
                }
            }
            _ => {
                let byte_is_newline = buffer[byte] == b'\n';

                // 更新列数，处理退格符和非标准字符。
                state.column = if byte_is_newline {
                    0
                } else if c_type == Other {
                    state.column + c_width
                } else if state.column > 0 {
                    state.column - 1
                } else {
                    0
                };

                // 如果当前字符不是空格，则标记行首空格处理完成。
                if byte_is_newline {
                    state.is_init = true;
                    finish_expand_line(state);
                } else if buffer[byte] != 0x20 {
                    state.is_init = false;
                    state.current_line_has_content = true;
                } else {
                    state.current_line_has_content = true;
                }

                output.write_all(&buffer[byte..byte + n_bytes])?;
            }
        }

        byte += n_bytes; // 移动到下一个字符。
    }

    Ok(())
}

/**
 * 扩展给定选项中的文件内容。
 *
 * 此函数遍历`options.files`中指定的每个文件，对于每个文件，它读取内容并根据`options`中的设置进行扩展，
 * 然后将结果写入标准输出。
 *
 * @param options 一个包含文件列表和扩展设置的结构体引用。
 * @return CTResult<ExpandRunOutcome>，如果成功则返回执行结果，如果遇到致命错误则返回Err()。
 */
fn expand_to_writer<W: Write>(
    options: &ExpandOptions,
    output: &mut W,
) -> CTResult<ExpandRunOutcome> {
    let tabstops = options.tabstops.as_ref();
    let mut buffer = Vec::new();
    let mut state = ExpandState::default();
    let mut is_first_file = true;
    let mut first_file_has_bom = false;
    let mut stderr_text = String::new();
    let mut exit_code = 0;

    for file in &options.files {
        if Path::new(file).is_dir() {
            stderr_text.push_str(&format!("expand: {file}: Is a directory\n"));
            exit_code = 1;
            continue;
        }
        match expand_open(file) {
            Ok(mut fh) => {
                let mut is_first_chunk = true;
                loop {
                    buffer.clear();
                    // 通过 take 限制单次读取上限为 64KB。
                    // 避免读取 /dev/zero 等无换行符的无限流时造成死循环和 OOM。
                    let mut chunk_reader = (&mut fh).take(65536);
                    let n = match chunk_reader.read_until(b'\n', &mut buffer) {
                        Ok(size) => size,
                        Err(e) => {
                            stderr_text.push_str(&format!("expand: {e}\n"));
                            exit_code = 1;
                            break;
                        }
                    };

                    if n == 0 {
                        break;
                    }

                    if is_first_chunk {
                        if buffer.starts_with(&UTF8_BOM) {
                            if is_first_file && !first_file_has_bom {
                                output.write_all(&UTF8_BOM)?;
                                first_file_has_bom = true;
                            }
                            buffer.drain(..UTF8_BOM.len());
                        }
                        is_first_chunk = false;
                    }

                    if !state.pending_utf8.is_empty() {
                        let mut merged = std::mem::take(&mut state.pending_utf8);
                        merged.extend_from_slice(&buffer);
                        buffer = merged;
                    }

                    if buffer.is_empty() {
                        continue;
                    }

                    expand_line(&buffer, output, tabstops, options, &mut state)
                        .map_err(|e| ctcore::ct_error::CtSimpleError::new(1, e.to_string()))?;
                    output.flush()?;
                }
            }
            Err(e) => {
                stderr_text.push_str(&format!("expand: {e}\n"));
                exit_code = 1;
                continue;
            }
        }

        if !state.pending_utf8.is_empty() {
            let pending = std::mem::take(&mut state.pending_utf8);
            expand_raw_bytes(&pending, output, &mut state)?;
        }
        is_first_file = false;
    }

    if state.current_line_has_content {
        finish_expand_line(&mut state);
    }

    Ok(ExpandRunOutcome {
        stderr_text,
        exit_code,
        line_had_tabs: state.line_had_tabs,
    })
}

fn expand(options: &ExpandOptions) -> CTResult<()> {
    let mut output = BufWriter::new(stdout());
    let outcome = expand_to_writer(options, &mut output)?;
    output.flush()?;

    if !outcome.stderr_text.is_empty() {
        eprint!("{}", outcome.stderr_text);
    }
    if outcome.exit_code != 0 {
        set_ct_exit_code(outcome.exit_code);
    }

    Ok(())
}

pub fn expand_native_semantic(args: impl ctcore::Args) -> CTResult<ExpandSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app().try_get_matches_from(expand_shortcuts(args.collect()))?;
    let options = ExpandOptions::new(&args_match)?;
    let mut classic_output = Vec::new();
    let outcome = expand_to_writer(&options, &mut classic_output)?;
    let classic_text = String::from_utf8_lossy(&classic_output).into_owned();

    Ok(ExpandSemantic {
        tabstop_mode: expand_tabstop_mode(&options.remaining_mode),
        tabstops: options.tabstops.clone(),
        initial_only: options.iflag,
        assume_utf8: options.uflag,
        rows: expand_rows_from_output(&classic_text, &outcome.line_had_tabs),
        classic_text,
        stderr_text: outcome.stderr_text,
        exit_code: outcome.exit_code,
    })
}

#[derive(Default)]
pub struct Expand;
impl Tool for Expand {
    fn name(&self) -> &'static str {
        "expand"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        expand_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Expand;

        // 测试 name 方法
        assert_eq!(tool.name(), "expand");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("expand"));

        // 测试 execute 方法
        let args = vec![OsString::from("expand"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    mod tests_echo_main {
        use crate::expand_main;

        use std::ffi::OsString;

        use std::fs;
        use std::fs::File;
        use std::io::Write;
        use tempfile::Builder;

        #[test]
        fn test_expand_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = expand_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_expand_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = expand_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_expand_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = expand_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_expand_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = expand_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }
        #[test]
        fn test_expand_main_initial() {
            let temp_dir = Builder::new()
                .prefix("test_expand_main_initial")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--initial", filename];
            let result = expand_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_expand_main_i() {
            let temp_dir = Builder::new()
                .prefix("test_expand_main_initial")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-i", filename];
            let result = expand_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_expand_main_tabs() {
            let temp_dir = Builder::new()
                .prefix("test_expand_main_initial")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--tabs", "4", filename];
            let result = expand_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_expand_main_t() {
            let temp_dir = Builder::new()
                .prefix("test_expand_main_initial")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-t", "4", filename];
            let result = expand_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_expand_main_no_utf8() {
            let temp_dir = Builder::new()
                .prefix("test_expand_main_initial")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--no-utf8", filename];
            let result = expand_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_expand_main_u() {
            let temp_dir = Builder::new()
                .prefix("test_expand_main_initial")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-U", filename];
            let result = expand_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
    }

    mod tests_ct_app {
        use crate::ct_app;

        use crate::opt_flags::{INITIAL, NO_UTF8};
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }
        #[test]
        fn test_ct_app_i() {
            let args = vec![ctcore::ct_util_name(), "-i", "file"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(INITIAL));
        }

        #[test]
        fn test_ct_app_initial() {
            let args = vec![ctcore::ct_util_name(), "--initial", "file"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(INITIAL));
        }

        #[test]
        fn test_ct_app_t() {
            let args = vec![ctcore::ct_util_name(), "-t", "4", "file"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_tabs() {
            let args = vec![ctcore::ct_util_name(), "--tabs", "4", "file"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u() {
            let args = vec![ctcore::ct_util_name(), "-U", "file"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(NO_UTF8));
        }

        #[test]
        fn test_ct_app_utf8() {
            let args = vec![ctcore::ct_util_name(), "--no-utf8", "file"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(NO_UTF8));
        }
    }

    mod tests_expand_functions {
        use crate::ExpandParseError::SpecifierNotAtStartOfNumber;
        use crate::{
            CharType, DEFAULT_TABSTOP, ExpandCharInfo, ExpandOptions, ExpandParseError, ExpandRow,
            ExpandState, ExpandTabstopMode, RemainingMode, UTF8_BOM, expand_line,
            expand_native_semantic, expand_next_char_info, expand_next_tabstop, expand_open,
            expand_shortcuts, expand_tabstops_parse, expand_to_writer,
        };

        use crate::is_digit_or_comma;

        #[test]
        fn test_next_tabstop_remaining_mode_none() {
            assert_eq!(expand_next_tabstop(&[1, 5], 0, &RemainingMode::None), 1);
            assert_eq!(expand_next_tabstop(&[1, 5], 3, &RemainingMode::None), 2);
            assert_eq!(expand_next_tabstop(&[1, 5], 6, &RemainingMode::None), 1);
        }

        #[test]
        fn test_next_tabstop_remaining_mode_plus() {
            assert_eq!(expand_next_tabstop(&[1, 5], 0, &RemainingMode::Plus), 1);
            assert_eq!(expand_next_tabstop(&[1, 5], 3, &RemainingMode::Plus), 3);
            assert_eq!(expand_next_tabstop(&[1, 5], 6, &RemainingMode::Plus), 5);
        }

        #[test]
        fn test_next_tabstop_remaining_mode_slash() {
            assert_eq!(expand_next_tabstop(&[1, 5], 0, &RemainingMode::Slash), 1);
            assert_eq!(expand_next_tabstop(&[1, 5], 3, &RemainingMode::Slash), 2);
            assert_eq!(expand_next_tabstop(&[1, 5], 6, &RemainingMode::Slash), 4);
        }

        #[test]
        fn test_is_digit_or_comma() {
            assert!(is_digit_or_comma('1'));
            assert!(is_digit_or_comma(','));
            assert!(!is_digit_or_comma('a'));
        }

        #[test]
        fn test_expand_tabstops_parse_empty_string() {
            let result = expand_tabstops_parse("");
            assert_eq!(result, Ok((RemainingMode::None, vec![DEFAULT_TABSTOP])));
        }

        #[test]
        fn test_expand_tabstops_parse_default_tabstop() {
            let result = expand_tabstops_parse("    ,   ,     ");
            assert_eq!(result, Ok((RemainingMode::None, vec![DEFAULT_TABSTOP])));
        }

        #[test]
        fn test_expand_tabstops_parse_valid_input() {
            let result = expand_tabstops_parse("4,8,12+16,20/");
            assert_eq!(
                result,
                Err(SpecifierNotAtStartOfNumber(
                    "+".to_string(),
                    "+16".to_string()
                ))
            );
        }

        #[test]
        fn test_expand_tabstops_parse_invalid_tabsize_zero() {
            let result = expand_tabstops_parse("0");
            assert_eq!(result, Err(ExpandParseError::TabSizeCannotBeZero));
        }

        #[test]
        fn test_expand_tabstops_parse_invalid_tabsizes_not_ascending() {
            let result = expand_tabstops_parse("8,4");
            assert_eq!(result, Err(ExpandParseError::TabSizesMustBeAscending));
        }

        #[test]
        fn test_expand_tabstops_parse_invalid_specifier_already_used() {
            let result = expand_tabstops_parse("4+8/12");
            assert_eq!(
                result,
                Err(SpecifierNotAtStartOfNumber(
                    "+".to_string(),
                    "+8/12".to_string()
                ))
            );
        }

        #[test]
        fn test_expand_tabstops_parse_invalid_tabsize_too_large() {
            let result = expand_tabstops_parse("9999999999999999999");
            assert_eq!(
                result.unwrap(),
                (RemainingMode::None, vec![9999999999999999999])
            );
        }

        #[test]
        fn test_expand_tabstops_parse_invalid_specifier_not_at_start_of_number() {
            let result = expand_tabstops_parse("4+8a");
            assert_eq!(
                result,
                Err(ExpandParseError::SpecifierNotAtStartOfNumber(
                    "+".to_string(),
                    "+8a".to_string()
                ))
            );
        }

        #[test]
        fn test_expand_tabstops_parse_invalid_character() {
            let result = expand_tabstops_parse("a");
            assert_eq!(
                result,
                Err(ExpandParseError::InvalidCharacter("a".to_string()))
            );
        }

        use std::ffi::OsString;
        use std::fs::File;
        use std::io::{Read, Write};

        #[test]
        fn test_expand_shortcuts() {
            let args = vec![
                OsString::from("-1,2,3"),
                OsString::from("file1.txt"),
                OsString::from("-4,5,6"),
                OsString::from("file2.txt"),
            ];
            let expected = vec![
                OsString::from("--tabs=1"),
                OsString::from("--tabs=2"),
                OsString::from("--tabs=3"),
                OsString::from("file1.txt"),
                OsString::from("--tabs=4"),
                OsString::from("--tabs=5"),
                OsString::from("--tabs=6"),
                OsString::from("file2.txt"),
            ];

            let result = expand_shortcuts(args);

            assert_eq!(result, expected);
        }

        #[test]
        fn test_expand_shortcuts_empty_args() {
            let args = Vec::new();
            let expected: Vec<OsString> = Vec::new();

            let result = expand_shortcuts(args);

            assert_eq!(result, expected);
        }

        #[test]
        fn test_expand_shortcuts_no_shortcuts() {
            let args = vec![
                OsString::from("file1.txt"),
                OsString::from("file2.txt"),
                OsString::from("file3.txt"),
            ];
            let expected = vec![
                OsString::from("file1.txt"),
                OsString::from("file2.txt"),
                OsString::from("file3.txt"),
            ];

            let result = expand_shortcuts(args);

            assert_eq!(result, expected);
        }

        #[test]
        fn test_expand_shortcuts_non_digit_or_comma() {
            let args = vec![OsString::from("-abc,def")];
            let expected = vec![OsString::from("-abc,def")];

            let result = expand_shortcuts(args);

            assert_eq!(result, expected);
        }

        // #[test]
        // fn test_expand_open_with_standard_input() {
        //     let input = "test input";
        //     let mut reader = expand_open("-").unwrap();
        //     let mut output = String::new();
        //     reader.read_to_string(&mut output).unwrap();
        //     assert_eq!(output, input);
        // }

        #[test]
        fn test_expand_open_with_file_path() {
            let file_path = "test_file.txt"; // Replace with the actual file path
            let mut file = File::create(file_path).unwrap();
            let content = "test content";
            file.write_all(content.as_bytes()).unwrap();

            let mut reader = expand_open(file_path).unwrap();
            let mut output = String::new();
            reader.read_to_string(&mut output).unwrap();
            assert_eq!(output, content);

            std::fs::remove_file(file_path).unwrap();
        }

        fn test_options(tabstop: usize) -> ExpandOptions {
            test_options_with_utf8(tabstop, false)
        }

        fn test_options_with_utf8(tabstop: usize, uflag: bool) -> ExpandOptions {
            ExpandOptions {
                files: vec![],
                tabstops: vec![tabstop],
                tspaces: " ".repeat(tabstop),
                iflag: false,
                uflag,
                remaining_mode: RemainingMode::None,
            }
        }

        #[test]
        fn test_expand_line_keeps_column_state_across_chunks() {
            let opts = test_options(3);
            let mut output = Vec::new();
            let mut state = ExpandState::default();

            let first = vec![b'a'; 65_536];
            expand_line(&first, &mut output, &opts.tabstops, &opts, &mut state).unwrap();
            expand_line(b"\tX\n", &mut output, &opts.tabstops, &opts, &mut state).unwrap();

            let expected = [vec![b'a'; 65_536], b"  X\n".to_vec()].concat();
            assert_eq!(output, expected);
        }

        #[test]
        fn test_expand_line_resets_state_after_newline() {
            let opts = test_options(3);
            let mut output = Vec::new();
            let mut state = ExpandState::default();

            expand_line(b"a\t\n", &mut output, &opts.tabstops, &opts, &mut state).unwrap();
            expand_line(b"\tX\n", &mut output, &opts.tabstops, &opts, &mut state).unwrap();

            assert_eq!(output, b"a  \n   X\n");
            assert_eq!(
                state,
                ExpandState {
                    line_had_tabs: vec![true, true],
                    ..ExpandState::default()
                }
            );
        }

        #[test]
        fn test_expand_next_char_info_handles_wide_utf8_characters() {
            let (c_type, c_width, n_bytes) = match expand_next_char_info(true, "　".as_bytes(), 0)
            {
                ExpandCharInfo::Parsed {
                    c_type,
                    c_width,
                    n_bytes,
                } => (c_type, c_width, n_bytes),
                ExpandCharInfo::Incomplete => panic!("unexpected incomplete sequence"),
            };

            assert_eq!(c_type, CharType::Other);
            assert_eq!(c_width, 2);
            assert_eq!(n_bytes, "　".len());
        }

        #[test]
        fn test_expand_line_carries_partial_utf8_between_chunks() {
            let opts = test_options_with_utf8(3, true);
            let mut output = Vec::new();
            let mut state = ExpandState::default();

            let mut first = vec![b'a'; 65_535];
            first.push("　".as_bytes()[0]);
            expand_line(&first, &mut output, &opts.tabstops, &opts, &mut state).unwrap();

            let mut second = "　".as_bytes()[1..].to_vec();
            second.extend_from_slice(b"\tX\n");
            let mut merged = std::mem::take(&mut state.pending_utf8);
            merged.extend_from_slice(&second);
            expand_line(&merged, &mut output, &opts.tabstops, &opts, &mut state).unwrap();

            let expected = [
                vec![b'a'; 65_535],
                "　".as_bytes().to_vec(),
                b" X\n".to_vec(),
            ]
            .concat();
            assert_eq!(output, expected);
            assert_eq!(
                state,
                ExpandState {
                    line_had_tabs: vec![true],
                    ..ExpandState::default()
                }
            );
        }

        #[test]
        fn test_expand_to_writer_skips_bom_after_first_file() {
            let mut first = tempfile::NamedTempFile::new().unwrap();
            let mut second = tempfile::NamedTempFile::new().unwrap();
            first.write_all(&UTF8_BOM).unwrap();
            first.write_all(b"a\tb\n").unwrap();
            second.write_all(&UTF8_BOM).unwrap();
            second.write_all(b"c\td\n").unwrap();

            let options = ExpandOptions {
                files: vec![
                    first.path().to_string_lossy().to_string(),
                    second.path().to_string_lossy().to_string(),
                ],
                tabstops: vec![8],
                tspaces: " ".repeat(8),
                iflag: false,
                uflag: true,
                remaining_mode: RemainingMode::None,
            };

            let mut output = Vec::new();
            expand_to_writer(&options, &mut output).unwrap();

            assert_eq!(output, b"\xEF\xBB\xBFa       b\nc       d\n");
        }

        #[test]
        fn test_expand_native_semantic_collects_rows_and_metadata() {
            let mut file = tempfile::NamedTempFile::new().unwrap();
            file.write_all(b"a\tb\n\tc\n").unwrap();

            let semantic = expand_native_semantic(
                vec![
                    OsString::from("expand"),
                    OsString::from("-t"),
                    OsString::from("4"),
                    file.path().as_os_str().to_os_string(),
                ]
                .into_iter(),
            )
            .unwrap();

            assert_eq!(semantic.tabstop_mode, ExpandTabstopMode::None);
            assert_eq!(semantic.tabstops, vec![4]);
            assert!(!semantic.initial_only);
            assert!(semantic.assume_utf8);
            assert_eq!(semantic.classic_text, "a   b\n    c\n");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
            assert_eq!(
                semantic.rows,
                vec![
                    ExpandRow {
                        row_index: 1,
                        line: "a   b".into(),
                        had_tabs: true,
                    },
                    ExpandRow {
                        row_index: 2,
                        line: "    c".into(),
                        had_tabs: true,
                    },
                ]
            );
        }

        #[test]
        fn test_expand_native_semantic_preserves_directory_error() {
            let temp_dir = tempfile::tempdir().unwrap();
            let input_dir = temp_dir.path().join("input");
            std::fs::create_dir(&input_dir).unwrap();

            let semantic = expand_native_semantic(
                vec![OsString::from("expand"), input_dir.as_os_str().into()].into_iter(),
            )
            .unwrap();

            assert!(semantic.rows.is_empty());
            assert_eq!(semantic.classic_text, "");
            assert_eq!(
                semantic.stderr_text,
                format!("expand: {}: Is a directory\n", input_dir.display())
            );
            assert_eq!(semantic.exit_code, 1);
        }
    }
}
