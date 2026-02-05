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
use ctcore::ct_show_error;

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::io::stdin;
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

/// The mode to use when replacing tabs beyond the last one specified in
/// the `--tabs` argument.
#[derive(PartialEq, Debug)]
enum RemainingMode {
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
        Ok(BufReader::new(Box::new(stdin()) as Box<dyn Read>))
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
#[allow(clippy::cognitive_complexity)]
fn expand_line(
    buffer: &mut Vec<u8>,
    output: &mut BufWriter<std::io::Stdout>,
    tabstops: &[usize],
    opts: &ExpandOptions,
) -> std::io::Result<()> {
    use self::CharType::*;

    // 初始化列数、字节位置和是否处于行起始的标志。
    let mut colum = 0;
    let mut byte = 0;
    let mut is_init = true;

    // 遍历缓冲区中的每个字符。
    while byte < buffer.len() {
        // 根据是否启用 Unicode 模式，确定字符类型、宽度和字节数。
        let (c_type, c_width, n_bytes) = if opts.uflag {
            let n_bytes = char::from(buffer[byte]).len_utf8();

            if byte + n_bytes > buffer.len() {
                // 处理由于无效 UTF-8 导致的缓冲区越界。
                (Other, 1, 1)
            } else if let Ok(t) = from_utf8(&buffer[byte..byte + n_bytes]) {
                match t.chars().next() {
                    Some('\t') => (Tab, 0, n_bytes),
                    Some('\x08') => (Backspace, 0, n_bytes),
                    Some(c) => (Other, UnicodeWidthChar::width(c).unwrap_or(0), n_bytes),
                    None => {
                        // 如果起始位置无效，则将该字节视为普通字符。
                        (Other, 1, 1)
                    }
                }
            } else {
                (Other, 1, 1) // 假设非 UTF-8 字符宽度为 1。
            }
        } else {
            (
                match buffer[byte] {
                    // 在严格 ASCII 模式下，每个字符均视为宽度为 1。
                    0x09 => Tab,
                    0x08 => Backspace,
                    _ => Other,
                },
                1,
                1,
            )
        };

        // 根据字符类型更新列数并输出相应字符。
        match c_type {
            Tab => {
                // 计算到下一个制表位需要多少空格。
                let nts = expand_next_tabstop(tabstops, colum, &opts.remaining_mode);
                colum += nts;

                // 根据选项扩展制表符为空格或保留制表符。
                if is_init || !opts.iflag {
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
                // 更新列数，处理退格符和非标准字符。
                colum = if c_type == Other {
                    colum + c_width
                } else if colum > 0 {
                    colum - 1
                } else {
                    0
                };

                // 如果当前字符不是空格，则标记行首空格处理完成。
                if buffer[byte] != 0x20 {
                    is_init = false;
                }

                output.write_all(&buffer[byte..byte + n_bytes])?;
            }
        }

        byte += n_bytes; // 移动到下一个字符。
    }

    // 刷新输出并清空缓冲区。
    output.flush()?;
    buffer.truncate(0); // 清空缓冲区。

    Ok(())
}

/**
 * 扩展给定选项中的文件内容。
 *
 * 此函数遍历`options.files`中指定的每个文件，对于每个文件，它读取内容并根据`options`中的设置进行扩展，
 * 然后将结果写入标准输出。
 *
 * @param options 一个包含文件列表和扩展设置的结构体引用。
 * @return CTResult<()>，如果成功则返回Ok(())，如果遇到错误则返回Err()。
 */
fn expand(options: &ExpandOptions) -> CTResult<()> {
    // 创建一个缓冲写入器，用于写入标准输出。
    let mut output = BufWriter::new(stdout());
    // 获取tabstops的引用，用于在行扩展过程中定位和处理制表符。
    let tabstops = options.tabstops.as_ref();
    // 创建一个缓冲区，用于临时存储从文件中读取的行。
    let mut buffer = Vec::new();

    // 遍历文件列表。
    for file in &options.files {
        // 检查文件是否为目录，如果是，则显示错误并继续处理下一个文件。
        if Path::new(file).is_dir() {
            ct_show_error!("{}: Is a directory", file);
            set_ct_exit_code(1);
            continue;
        }
        // 尝试打开文件。
        match expand_open(file) {
            Ok(mut fh) => {
                // 循环读取文件，直到没有更多内容可读。
                while match fh.read_until(b'\n', &mut buffer) {
                    Ok(s) => s > 0,
                    Err(_) => buffer.is_empty(),
                } {
                    // 对读取的每一行进行扩展，并将结果写入输出缓冲区。
                    expand_line(&mut buffer, &mut output, tabstops, options)
                        .map_err_context(|| "failed to write output".to_string())?;
                }
            }
            Err(e) => {
                // 如果打开文件时发生错误，显示错误并继续处理下一个文件。
                ct_show_error!("{}", e);
                set_ct_exit_code(1);
                continue;
            }
        }
    }
    // 如果成功处理所有文件，返回成功结果。
    Ok(())
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
            DEFAULT_TABSTOP, ExpandParseError, RemainingMode, expand_next_tabstop, expand_open,
            expand_shortcuts, expand_tabstops_parse,
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
    }
}
