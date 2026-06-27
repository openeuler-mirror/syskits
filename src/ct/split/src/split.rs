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

mod filenames;
mod number;
mod platform;
mod strategy;

use crate::filenames::{FilenameIterator, FilenameSuffix, FilenameSuffixError};
use crate::strategy::{Strategy, StrategyError, StrategyNumberType};
use clap::{Arg, ArgAction, ArgMatches, Command, ValueHint, crate_version, parser::ValueSource};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTIoError, CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_parse_size::parse_size_u64;
use ctcore::uio_error;
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs::{File, metadata};
use std::io;
use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Read, Seek, SeekFrom, Write, stdin};
use std::path::Path;

static OPT_BYTES: &str = "bytes";
static OPT_LINE_BYTES: &str = "line-bytes";
static OPT_LINES: &str = "lines";
static OPT_ADDITIONAL_SUFFIX: &str = "additional-suffix";
static OPT_FILTER: &str = "filter";
static OPT_NUMBER: &str = "number";
static OPT_NUMERIC_SUFFIXES: &str = "numeric-suffixes";
static OPT_NUMERIC_SUFFIXES_SHORT: &str = "-d";
static OPT_HEX_SUFFIXES: &str = "hex-suffixes";
static OPT_HEX_SUFFIXES_SHORT: &str = "-x";
static OPT_SUFFIX_LENGTH: &str = "suffix-length";
static OPT_VERBOSE: &str = "verbose";
static OPT_SEPARATOR: &str = "separator";
static OPT_ELIDE_EMPTY_FILES: &str = "elide-empty-files";
static OPT_IO_BLKSIZE: &str = "-io-blksize";

static ARG_INPUT: &str = "input";
static ARG_PREFIX: &str = "prefix";

const SPLIT_ABOUT: &str = ct_help_about!("split.md");
const SPLIT_USAGE: &str = ct_help_usage!("split.md");
const AFTER_HELP: &str = ct_help_section!("after help", "split.md");

#[derive(Default)]
pub struct Split;
impl Tool for Split {
    fn name(&self) -> &'static str {
        "split"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        split_main(args.iter().cloned()).map(|_| ())
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    split_main(args).map(|_| ())
}

pub fn split_main(args: impl ctcore::Args) -> CTResult<()> {
    let (args, obs_lines) = split_handle_obsolete(args);
    let args_match = ct_app().try_get_matches_from(args)?;

    match SpliceSettings::from(&args_match, &obs_lines) {
        Ok(settings) => split(&settings),
        Err(e) if e.splice_requires_usage() => Err(CTsageError::new(1, format!("{e}"))),
        Err(e) => Err(CtSimpleError::new(1, format!("{e}"))),
    }
}

/// Extract obsolete shorthand (if any) for specifying lines in following scenarios (and similar)
/// `split -22 file` would mean `split -l 22 file`
/// `split -2de file` would mean `split -l 2 -d -e file`
/// `split -x300e file` would mean `split -x -l 300 -e file`
/// `split -x300e -22 file` would mean `split -x -e -l 22 file` (last obsolete lines option wins)
/// following GNU `split` behavior
/// 分割并处理命令行参数。
fn split_handle_obsolete(args: impl ctcore::Args) -> (Vec<OsString>, Option<String>) {
    // 初始化用于存储已弃用参数行的可选项，以及标记是否紧随了需要值的长或短选项。
    let mut obs_lines = None;
    let mut preceding_long_opt_req_value = false;
    let mut preceding_short_opt_req_value = false;

    // 过滤并映射输入参数，忽略已弃用的参数行并处理需要值的长/短选项。
    let filtered_args = args
        .filter_map(|os_slice| {
            split_filter_args(
                os_slice,
                &mut obs_lines,
                &mut preceding_long_opt_req_value,
                &mut preceding_short_opt_req_value,
            )
        })
        .collect();

    (filtered_args, obs_lines)
}

/// Helper function to [`split_handle_obsolete`]
/// Filters out obsolete lines option from args
/**
 * 过滤命令行参数，并根据特定条件处理和返回这些参数。
 *
 * 此函数主要处理与命令行选项相关的逻辑，包括识别是否需要提取废弃行信息（如果存在），
 * 以及根据参数特征更新全局状态。
 *
 * @param os_slice 原始操作系统字符串形式的命令行参数，这可能是非UTF-8编码的。
 * @param obs_lines 可选字符串的引用，用于存储可能被提取的废弃行信息。
 * @param preceding_long_opt_req_value 长选项需要值的前置状态标志的可变引用。
 * @param preceding_short_opt_req_value 短选项需要值的前置状态标志的可变引用。
 * @return 返回一个选项，可能包含经过处理或未处理的原始OsString。
 */
fn split_filter_args(
    os_slice: OsString,
    obs_lines: &mut Option<String>,
    is_preceding_long_opt_req_value: &mut bool,
    is_preceding_short_opt_req_value: &mut bool,
) -> Option<OsString> {
    // 根据os_slice的UTF-8转换状态和特定逻辑来决定过滤结果
    let opt_filter: Option<OsString>;
    if let Some(slice) = os_slice.to_str() {
        // 判断是否应该提取废弃行信息
        if split_should_extract_obs_lines(
            slice,
            is_preceding_long_opt_req_value,
            is_preceding_short_opt_req_value,
        ) {
            // 处理并尝试提取废弃行信息
            opt_filter = splice_handle_extract_obs_lines(slice, obs_lines);
        } else {
            // 直接使用该参数，不涉及废弃行信息
            opt_filter = Some(OsString::from(slice));
        }
        // 更新前置选项状态
        splice_handle_preceding_options(
            slice,
            is_preceding_long_opt_req_value,
            is_preceding_short_opt_req_value,
        );
    } else {
        // 对于无法转换为UTF-8的os_slice，直接返回，不进行处理
        opt_filter = Some(os_slice);
    }
    opt_filter
}

/// Helper function to [`split_filter_args`]
/// Checks if the slice is a true short option (and not hyphen prefixed value of an option)
/// and if so, a short option that can contain obsolete lines value
/**
 * 判断给定的字符串切片是否满足特定的条件。
 *
 * 此函数设计用于处理命令行参数的分割字符串，判断某个分割出来的字符串
 * 是否应该被当作一个独立的观察行（obs line）提取出来。
 *
 * @param split_slice 需要进行判断的字符串切片。
 * @param is_preceding_long_opt_req_value 指向前一个长选项是否需要值的布尔引用。
 * @param is_preceding_short_opt_req_value 指向前一个短选项是否需要值的布尔引用。
 * @return 返回一个布尔值，如果满足特定条件，则为true，否则为false。
 */
fn split_should_extract_obs_lines(
    split_slice: &str,
    is_preceding_long_opt_req_value: &bool,
    is_preceding_short_opt_req_value: &bool,
) -> bool {
    // 检查split_slice是否以单个'-'开头但不是"--"，并且不跟在特定选项之后
    // 同时前一个长选项或短选项不需要值
    split_slice.starts_with('-')
        && !split_slice.starts_with("--")
        && !is_preceding_long_opt_req_value
        && !is_preceding_short_opt_req_value
        && !split_slice.starts_with("-a")
        && !split_slice.starts_with("-b")
        && !split_slice.starts_with("-C")
        && !split_slice.starts_with("-l")
        && !split_slice.starts_with("-n")
        && !split_slice.starts_with("-t")
}

/// # 参数
/// - `splice_slice`: 输入的原始字符串，可能包含需要提取的过时行信息。
/// - `obs_lines`: 指向可选字符串的引用，用于存储提取出的过时行信息。如果提取成功，此处将更新为包含过时行信息的字符串。
///
/// # 返回值
/// - 如果成功提取出过时行信息，则返回一个包含过滤后字符串的 `Option<OsString>`，这个字符串不包含过时行信息。
/// - 如果没有提取出过时行信息，则返回原始输入字符串的 `Option<OsString>`。
fn splice_handle_extract_obs_lines(
    splice_slice: &str,
    obs_lines: &mut Option<String>,
) -> Option<OsString> {
    // 初始化用于存储提取出的过时行信息的向量。
    let mut is_obs_lines_extracted: Vec<char> = vec![];
    // 用于标记是否已经达到了过时行信息的末尾。
    let mut is_obs_lines_end_reached = false;

    // 过滤 `splice_slice`，提取出非数字字符或在遇到非数字字符后停止提取。
    let splice_filtered_slice: Vec<char> = splice_slice
        .chars()
        .filter(|c| {
            if c.is_ascii_digit() && !is_obs_lines_end_reached {
                is_obs_lines_extracted.push(*c);
                false // 遇到数字但尚未到达过时行信息末尾时，不包含该字符。
            } else {
                if !is_obs_lines_extracted.is_empty() {
                    is_obs_lines_end_reached = true;
                }
                true // 遇到非数字字符或已提取完过时行信息时，包含该字符。
            }
        })
        .collect();

    if is_obs_lines_extracted.is_empty() {
        // 如果没有提取到过时行信息，返回原始字符串。
        Some(OsString::from(splice_slice))
    } else {
        // 如果提取到了过时行信息，更新 `obs_lines` 并根据 `splice_filtered_slice` 的状态返回处理后的字符串或 `None`。
        let extracted_string: String = is_obs_lines_extracted.iter().collect();
        *obs_lines = Some(extracted_string);

        if splice_filtered_slice.get(1).is_some() {
            // 如果 `splice_filtered_slice` 中还有其他字符，则返回过滤后的字符串。
            let filtered_slice: String = splice_filtered_slice.iter().collect();
            Some(OsString::from(filtered_slice))
        } else {
            // 如果 `splice_filtered_slice` 仅包含过时行信息，则返回 `None`。
            None
        }
    }
}

/// Helper function to [`splice_handle_extract_obs_lines`]
/// Captures if current slice is a preceding option
/// that requires value
// 处理前置选项的函数。
// 此函数用于分析命令行参数中的选项和值，尤其关注那些需要值但未使用'='来直接赋值的长选项和短选项。
//
// # 参数
// - `splice_slice`: &str - 当前正在分析的命令行参数切片。
// - `is_preceding_long_opt_req_value`: &mut bool - 指向一个布尔值的可变引用，用于指示是否遇到了需要值的前置长选项。
// - `is_preceding_short_opt_req_value`: &mut bool - 指向一个布尔值的可变引用，用于指示是否遇到了需要值的前置短选项。
fn splice_handle_preceding_options(
    splice_slice: &str,
    is_preceding_long_opt_req_value: &mut bool,
    is_preceding_short_opt_req_value: &mut bool,
) {
    // 检查当前切片是否为需要值的前置长选项且未使用'='赋值
    if splice_slice.starts_with("--") {
        *is_preceding_long_opt_req_value = &splice_slice[2..] == OPT_BYTES
            || &splice_slice[2..] == OPT_LINE_BYTES
            || &splice_slice[2..] == OPT_LINES
            || &splice_slice[2..] == OPT_ADDITIONAL_SUFFIX
            || &splice_slice[2..] == OPT_FILTER
            || &splice_slice[2..] == OPT_NUMBER
            || &splice_slice[2..] == OPT_SUFFIX_LENGTH
            || &splice_slice[2..] == OPT_SEPARATOR;
    }
    // 检查当前切片是否为需要值的前置短选项（值通过空格分隔）
    *is_preceding_short_opt_req_value = splice_slice == "-b"
        || splice_slice == "-C"
        || splice_slice == "-l"
        || splice_slice == "-n"
        || splice_slice == "-a"
        || splice_slice == "-t";
    // 如果当前切片不是以'-'开头，则认为它是一个值，并重置前置选项标志
    if !splice_slice.starts_with('-') {
        *is_preceding_short_opt_req_value = false;
        *is_preceding_long_opt_req_value = false;
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = SPLIT_ABOUT;
    let usage_description = ct_format_usage(SPLIT_USAGE);

    let args = splice_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .after_help(AFTER_HELP)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn splice_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(OPT_BYTES)
            .short('b')
            .long(OPT_BYTES)
            .allow_hyphen_values(true)
            .value_name("SIZE")
            .help("put SIZE bytes per output file"),
        Arg::new(OPT_LINE_BYTES)
            .short('C')
            .long(OPT_LINE_BYTES)
            .allow_hyphen_values(true)
            .value_name("SIZE")
            .help("put at most SIZE bytes of lines per output file"),
        Arg::new(OPT_LINES)
            .short('l')
            .long(OPT_LINES)
            .allow_hyphen_values(true)
            .value_name("NUMBER")
            .default_value("1000")
            .help("put NUMBER lines/records per output file"),
        Arg::new(OPT_NUMBER)
            .short('n')
            .long(OPT_NUMBER)
            .allow_hyphen_values(true)
            .value_name("CHUNKS")
            .help("generate CHUNKS output files; see explanation below"),
        Arg::new(OPT_ADDITIONAL_SUFFIX)
            .long(OPT_ADDITIONAL_SUFFIX)
            .allow_hyphen_values(true)
            .value_name("SUFFIX")
            .default_value("")
            .help("additional SUFFIX to append to output file names"),
        Arg::new(OPT_FILTER)
            .long(OPT_FILTER)
            .allow_hyphen_values(true)
            .value_name("COMMAND")
            .value_hint(ValueHint::CommandName)
            .help(
                "write to shell COMMAND; file name is $FILE (Currently not implemented for Windows)",
            ),
        Arg::new(OPT_ELIDE_EMPTY_FILES)
            .long(OPT_ELIDE_EMPTY_FILES)
            .short('e')
            .help("do not generate empty output files with '-n'")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_NUMERIC_SUFFIXES_SHORT)
            .short('d')
            .action(ArgAction::SetTrue)
            .overrides_with_all([
                OPT_NUMERIC_SUFFIXES,
                OPT_NUMERIC_SUFFIXES_SHORT,
                OPT_HEX_SUFFIXES,
                OPT_HEX_SUFFIXES_SHORT
            ])
            .help("use numeric suffixes starting at 0, not alphabetic"),
        Arg::new(OPT_NUMERIC_SUFFIXES)
            .long(OPT_NUMERIC_SUFFIXES)
            .require_equals(true)
            .num_args(0..=1)
            .overrides_with_all([
                OPT_NUMERIC_SUFFIXES,
                OPT_NUMERIC_SUFFIXES_SHORT,
                OPT_HEX_SUFFIXES,
                OPT_HEX_SUFFIXES_SHORT
            ])
            .value_name("FROM")
            .help("same as -d, but allow setting the start value"),
        Arg::new(OPT_HEX_SUFFIXES_SHORT)
            .short('x')
            .action(ArgAction::SetTrue)
            .overrides_with_all([
                OPT_NUMERIC_SUFFIXES,
                OPT_NUMERIC_SUFFIXES_SHORT,
                OPT_HEX_SUFFIXES,
                OPT_HEX_SUFFIXES_SHORT
            ])
            .help("use hex suffixes starting at 0, not alphabetic"),
        Arg::new(OPT_HEX_SUFFIXES)
            .long(OPT_HEX_SUFFIXES)
            .require_equals(true)
            .num_args(0..=1)
            .overrides_with_all([
                OPT_NUMERIC_SUFFIXES,
                OPT_NUMERIC_SUFFIXES_SHORT,
                OPT_HEX_SUFFIXES,
                OPT_HEX_SUFFIXES_SHORT
            ])
            .value_name("FROM")
            .help("same as -x, but allow setting the start value"),
        Arg::new(OPT_SUFFIX_LENGTH)
            .short('a')
            .long(OPT_SUFFIX_LENGTH)
            .allow_hyphen_values(true)
            .value_name("N")
            .help("generate suffixes of length N (default 2)"),
        Arg::new(OPT_VERBOSE)
            .long(OPT_VERBOSE)
            .help("print a diagnostic just before each output file is opened")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_SEPARATOR)
            .short('t')
            .long(OPT_SEPARATOR)
            .allow_hyphen_values(true)
            .value_name("SEP")
            .action(ArgAction::Append)
            .help("use SEP instead of newline as the record separator; '\\0' (zero) specifies the NUL character"),
        Arg::new(OPT_IO_BLKSIZE)
            .long("io-blksize")
            .alias(OPT_IO_BLKSIZE)
            .hide(true),
        Arg::new(ARG_INPUT)
            .default_value("-")
            .value_hint(ValueHint::FilePath),
        Arg::new(ARG_PREFIX)
            .default_value("x")
    ];
    args
}

/// Parameters that control how a file gets split.
///
/// You can convert an [`ArgMatches`] instance into a [`SpliceSettings`]
/// instance by calling [`SpliceSettings::from`].
struct SpliceSettings {
    prefix: String,
    suffix: FilenameSuffix,
    input: String,
    /// When supplied, a shell command to output to instead of xaa, xab …
    filter: Option<String>,
    strategy: Strategy,
    verbose: bool,
    separator: u8,

    /// Whether to *not* produce empty files when using `-n`.
    ///
    /// The `-n` command-line argument gives a specific number of
    /// chunks into which the input files will be split. If the number
    /// of chunks is greater than the number of bytes, and this is
    /// `false`, then empty files will be created for the excess
    /// chunks. If this is `false`, then empty files will not be
    /// created.
    elide_empty_files: bool,
    io_blksize: Option<u64>,
}

/// An error when parsing settings from command-line arguments.
#[derive(Debug)]
enum SpliceSettingsError {
    /// Invalid chunking strategy.
    Strategy(StrategyError),

    /// Invalid suffix length parameter.
    Suffix(FilenameSuffixError),

    /// Multi-character (Invalid) separator
    MultiCharacterSeparator(String),

    /// Multiple different separator characters
    MultipleSeparatorCharacters,

    /// Using `--filter` with `--number` option sub-strategies that print Kth chunk out of N chunks to stdout
    /// K/N
    /// l/K/N
    /// r/K/N
    FilterWithKthChunkNumber,

    /// Invalid IO block size
    InvalidIOBlockSize(String),

    /// The `--filter` option is not supported on Windows.
    #[cfg(windows)]
    NotSupported,
}

impl SpliceSettingsError {
    /// Whether the error demands a usage message.
    fn splice_requires_usage(&self) -> bool {
        matches!(
            self,
            Self::Strategy(StrategyError::MultipleWays)
                | Self::Suffix(FilenameSuffixError::ContainsSeparator(_))
        )
    }
}

impl fmt::Display for SpliceSettingsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Strategy(e) => e.fmt(f),
            Self::Suffix(e) => e.fmt(f),
            Self::MultiCharacterSeparator(s) => {
                write!(f, "multi-character separator {}", s.quote())
            }
            Self::MultipleSeparatorCharacters => {
                write!(f, "multiple separator characters specified")
            }
            Self::FilterWithKthChunkNumber => {
                write!(f, "--filter does not process a chunk extracted to stdout")
            }
            Self::InvalidIOBlockSize(s) => write!(f, "invalid IO block size: {}", s.quote()),
            #[cfg(windows)]
            Self::NotSupported => write!(
                f,
                "{OPT_FILTER} is currently not supported in this platform"
            ),
        }
    }
}

impl SpliceSettings {
    /// Parse a strategy from the command-line arguments.
    fn from(
        args_match: &ArgMatches,
        obs_lines: &Option<String>,
    ) -> Result<Self, SpliceSettingsError> {
        let strategy =
            Strategy::from(args_match, obs_lines).map_err(SpliceSettingsError::Strategy)?;
        let suffix =
            FilenameSuffix::from(args_match, &strategy).map_err(SpliceSettingsError::Suffix)?;

        // 尝试从命令行参数中获取分隔符字符串
        let args_separator = match args_match.get_many::<String>(OPT_SEPARATOR) {
            Some(mut sep_values) => {
                // 获取并解析第一个分隔符值
                let first = sep_values.next().unwrap();
                // 检查是否所有分隔符值都相同
                if !sep_values.all(|s| s == first) {
                    // 如果存在不同的分隔符值，则返回错误
                    return Err(SpliceSettingsError::MultipleSeparatorCharacters);
                }
                // 根据分隔符值处理分隔符
                match first.as_str() {
                    "\\0" => b'\0',
                    s if s.len() == 1 => s.as_bytes()[0],
                    // 如果分隔符不是单个字符，则返回错误
                    s => return Err(SpliceSettingsError::MultiCharacterSeparator(s.to_string())),
                }
            }
            None => b'\n', // 如果没有指定分隔符，则默认使用换行符
        };
        let io_blksize: Option<u64> = if let Some(s) = args_match.get_one::<String>(OPT_IO_BLKSIZE)
        {
            match parse_size_u64(s) {
                Ok(0) => return Err(SpliceSettingsError::InvalidIOBlockSize(s.to_string())),
                Ok(n) if n <= ctcore::ct_fs::sane_blksize::MAX => Some(n),
                _ => return Err(SpliceSettingsError::InvalidIOBlockSize(s.to_string())),
            }
        } else {
            None
        };

        let result = Self {
            prefix: args_match.get_one::<String>(ARG_PREFIX).unwrap().clone(),
            suffix,
            input: args_match.get_one::<String>(ARG_INPUT).unwrap().clone(),
            filter: args_match.get_one::<String>(OPT_FILTER).cloned(),
            strategy,
            verbose: args_match.value_source(OPT_VERBOSE) == Some(ValueSource::CommandLine),
            separator: args_separator,
            elide_empty_files: args_match.get_flag(OPT_ELIDE_EMPTY_FILES),
            io_blksize,
        };

        #[cfg(windows)]
        if result.filter.is_some() {
            // see https://github.com/rust-lang/rust/issues/29494
            return Err(SpliceSettingsError::NotSupported);
        }

        // 如果--filter选项与--number选项的任一Kth块子策略一起使用，则返回错误。
        // 因为这些子策略将数据写入split命令的stdout，而无法写入过滤命令子进程。
        let kth_chunk = matches!(
            result.strategy,
            Strategy::Number(StrategyNumberType::KthBytes(_, _))
                | Strategy::Number(StrategyNumberType::KthLines(_, _))
                | Strategy::Number(StrategyNumberType::KthRoundRobin(_, _))
        );
        if kth_chunk && result.filter.is_some() {
            return Err(SpliceSettingsError::FilterWithKthChunkNumber);
        }

        Ok(result)
    }

    fn splice_instantiate_current_writer(
        &self,
        file_name: &str,
        new: bool,
    ) -> io::Result<BufWriter<Box<dyn Write>>> {
        if platform::paths_refer_to_same_file(&self.input, file_name) {
            return Err(io::Error::new(
                ErrorKind::Other,
                format!("'{file_name}' would overwrite input; aborting"),
            ));
        }

        platform::instantiate_current_writer(&self.filter, file_name, new)
    }
}

/// When using `--filter` option, writing to child command process stdin
/// could fail with BrokenPipe error
/// It can be safely ignored
fn split_ignorable_io_error(error: &std::io::Error, splice_settings: &SpliceSettings) -> bool {
    error.kind() == ErrorKind::BrokenPipe && splice_settings.filter.is_some()
}

/// Custom wrapper for `write()` method
/// Follows similar approach to GNU implementation
/// If ignorable io error occurs, return number of bytes as if all bytes written
/// Should not be used for Kth chunk number sub-strategies
/// as those do not work with `--filter` option
fn splice_custom_write<T: Write>(
    splice_bytes: &[u8],
    splice_writer: &mut T,
    splice_settings: &SpliceSettings,
) -> std::io::Result<usize> {
    match splice_writer.write(splice_bytes) {
        Ok(n) => Ok(n),
        Err(e) if split_ignorable_io_error(&e, splice_settings) => Ok(splice_bytes.len()),
        Err(e) => Err(e),
    }
}

/// Custom wrapper for `write_all()` method
/// Similar to [`splice_custom_write`], but returns true or false
/// depending on if `--filter` stdin is still open (no BrokenPipe error)
/// Should not be used for Kth chunk number sub-strategies
/// as those do not work with `--filter` option
fn splice_custom_write_all<T: Write>(
    splice_bytes: &[u8],
    splice_writer: &mut T,
    splice_settings: &SpliceSettings,
) -> std::io::Result<bool> {
    match splice_writer.write_all(splice_bytes) {
        Ok(()) => Ok(true),
        Err(e) if split_ignorable_io_error(&e, splice_settings) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Get the size of the input file in bytes
/// Used only for subset of `--number=CHUNKS` strategy, as there is a need
/// to determine input file size upfront in order to estimate the chunk size
/// to be written into each of N files/chunks:
/// * N       split into N files based on size of input
/// * K/N     output Kth of N to stdout
/// * l/N     split into N files without splitting lines/records
/// * l/K/N   output Kth of N to stdout without splitting lines/records
///
/// For most files the size will be determined by either reading entire file content into a buffer
/// or by `len()` function of [`std::fs::metadata`].
///
/// However, for some files which report filesystem metadata size that does not match
/// their actual content size, we will need to attempt to find the end of file
/// with direct `seek()` on [`std::fs::File`].
///
/// For STDIN stream - read into a buffer up to a limit
/// If input stream does not EOF before that - return an error
/// (i.e. "infinite" input as in `cat /dev/zero | split ...`, `yes | split ...` etc.).
///
/// Note: The `buf` might end up with either partial or entire input content.
// 尝试确定输入数据的大小。
//
// 此函数用于从指定的输入源（文件或标准输入）中读取数据，并确定可以读取的数据量或输入源的总大小。
//
// # 参数
// - `splice_input`: 指定的输入源路径，可以是文件路径或"-"表示标准输入。
// - `splice_reader`: 一个实现了`BufRead`的读取器，用于从输入源读取数据。
// - `bufffer`: 用于存储读取数据的缓冲区。
// - `splice_io_blksize`: 一个可选参数，指定读取数据时的块大小。如果未指定，则会尝试从文件系统获取一个合适的块大小。
//
// # 返回值
// 返回一个`std::io::Result<u64>`，其中`u64`表示读取的数据量或输入源的大小（以字节为单位）。如果无法确定大小，将返回错误。
fn splice_get_input_size<R>(
    splice_input: &String,
    splice_reader: &mut R,
    bufffer: &mut Vec<u8>,
    splice_io_blksize: &Option<u64>,
) -> std::io::Result<u64>
where
    R: BufRead,
{
    // 设置读取限制为指定的io_blksize，如果未指定，则尝试从文件系统获取一个默认值
    let read_splice_limit: u64 = if let Some(custom_blksize) = splice_io_blksize {
        *custom_blksize
    } else {
        ctcore::ct_fs::sane_blksize::sane_blksize_from_path(Path::new(splice_input))
    };

    // 尝试在限制范围内读取数据到缓冲区
    let number_bytes = splice_reader
        .by_ref()
        .take(read_splice_limit)
        .read_to_end(bufffer)
        .map(|n| n as u64)?;

    if number_bytes < read_splice_limit {
        // 如果读取的字节数小于限制，说明输入源可能是一个小文件或空输入流
        Ok(number_bytes)
    } else if splice_input == "-" {
        // 如果输入源是标准输入，且未读取到所有内容，说明输入流可能是一个无限的流
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("{}: cannot determine input size", splice_input),
        ));
    } else {
        // 如果文件大小超过了读取限制，尝试从文件元数据中获取文件大小
        let input_metadata = metadata(splice_input)?;
        let input_metadata_size = input_metadata.len();
        if number_bytes <= input_metadata_size {
            Ok(input_metadata_size)
        } else {
            // 如果文件元数据大小与实际读取的大小不符，尝试直接寻求文件末尾以确定文件大小
            let mut tmp_file = File::open(Path::new(splice_input))?;
            let file_end = tmp_file.seek(SeekFrom::End(0))?;
            if file_end > 0 {
                Ok(file_end)
            } else {
                // 如果无法确定文件大小，返回错误
                return Err(io::Error::new(
                    ErrorKind::Other,
                    format!("{}: cannot determine file size", splice_input),
                ));
            }
        }
    }
}

/// Write a certain number of bytes to one file, then move on to another one.
///
/// This struct maintains an underlying writer representing the
/// current chunk of the output. If a call to [`write`] would cause
/// the underlying writer to write more than the allowed number of
/// bytes, a new writer is created and the excess bytes are written to
/// that one instead. As many new underlying writers are created as
/// needed to write all the bytes in the input buffer.
struct SpliceByteChunkWriter<'a> {
    /// Parameters for creating the underlying writer for each new chunk.
    settings: &'a SpliceSettings,

    /// The maximum number of bytes allowed for a single chunk of output.
    chunk_size: u64,

    /// Running total of number of chunks that have been completed.
    num_chunks_written: u64,

    /// Remaining capacity in number of bytes in the current chunk.
    ///
    /// This number starts at `chunk_size` and decreases as bytes are
    /// written. Once it reaches zero, a writer for a new chunk is
    /// initialized and this number gets reset to `chunk_size`.
    num_bytes_remaining_in_current_chunk: u64,

    /// The underlying writer for the current chunk.
    ///
    /// Once the number of bytes written to this writer exceeds
    /// `chunk_size`, a new writer is initialized and assigned to this
    /// field.
    inner: BufWriter<Box<dyn Write>>,

    /// Iterator that yields filenames for each chunk.
    filename_iterator: FilenameIterator<'a>,
}

impl<'a> SpliceByteChunkWriter<'a> {
    fn new(
        splice_chunk_size: u64,
        splice_settings: &'a SpliceSettings,
    ) -> CTResult<SpliceByteChunkWriter<'a>> {
        let mut file_iterator =
            FilenameIterator::new(&splice_settings.prefix, &splice_settings.suffix)?;
        let file_name = file_iterator
            .next()
            .ok_or_else(|| CtSimpleError::new(1, "output file suffixes exhausted"))?;
        if splice_settings.verbose {
            println!("creating file {}", file_name.quote());
        }
        let splice_inner = splice_settings.splice_instantiate_current_writer(&file_name, true)?;
        Ok(SpliceByteChunkWriter {
            settings: splice_settings,
            chunk_size: splice_chunk_size,
            num_bytes_remaining_in_current_chunk: splice_chunk_size,
            num_chunks_written: 0,
            inner: splice_inner,
            filename_iterator: file_iterator,
        })
    }
}

impl Write for SpliceByteChunkWriter<'_> {
    /// Implements `--bytes=SIZE`
    /**
     * 将字节切片写入当前的分块中。如果当前分块没有剩余空间，则会创建新的分块并继续写入。
     * 这个函数设计用于需要将大数据分块写入多个文件场景。
     *
     */
    fn write(&mut self, mut buf: &[u8]) -> std::io::Result<usize> {
        // 循环，直到没有更多的数据需要写入
        let mut carryover_bytes_written: usize = 0;
        loop {
            // 如果缓冲区为空，则完成写入，返回已经写入的字节数
            if buf.is_empty() {
                return Ok(carryover_bytes_written);
            }

            // 如果当前分块没有剩余空间，准备写入下一个分块
            if self.num_bytes_remaining_in_current_chunk == 0 {
                // 更新分块信息并创建新的分块文件
                self.num_chunks_written += 1;
                self.num_bytes_remaining_in_current_chunk = self.chunk_size;

                // 根据文件名生成器获取下一个文件名，并创建该文件
                let file_name = self.filename_iterator.next().ok_or_else(|| {
                    std::io::Error::new(ErrorKind::Other, "output file suffixes exhausted")
                })?;
                if self.settings.verbose {
                    println!("creating file {}", file_name.quote());
                }
                self.inner = self
                    .settings
                    .splice_instantiate_current_writer(&file_name, true)?;
            }

            // 决定本次写入的字节数
            let buffer_len = buf.len();
            if (buffer_len as u64) < self.num_bytes_remaining_in_current_chunk {
                // 如果当前分块剩余空间大于缓冲区中的字节数，则写入全部缓冲区中的字节
                let num_bytes_written = splice_custom_write(buf, &mut self.inner, self.settings)?;
                self.num_bytes_remaining_in_current_chunk -= num_bytes_written as u64;
                return Ok(carryover_bytes_written + num_bytes_written);
            } else {
                // 如果当前分块剩余空间小于或等于缓冲区中的字节数，则只写入足够填满当前分块的字节
                let size = self.num_bytes_remaining_in_current_chunk as usize;
                let num_bytes_written =
                    splice_custom_write(&buf[..size], &mut self.inner, self.settings)?;
                self.num_bytes_remaining_in_current_chunk -= num_bytes_written as u64;

                // 如果底层写入器未能写入所有字节，则返回已写入的字节数
                if num_bytes_written < size {
                    return Ok(carryover_bytes_written + num_bytes_written);
                } else {
                    // 更新缓冲区，只考虑剩余未写入的字节
                    buf = &buf[size..];

                    // 更新累计已写入的字节数
                    carryover_bytes_written += num_bytes_written;
                }
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Write a certain number of lines to one file, then move on to another one.
///
/// This struct maintains an underlying writer representing the
/// current chunk of the output. If a call to [`write`] would cause
/// the underlying writer to write more than the allowed number of
/// lines, a new writer is created and the excess lines are written to
/// that one instead. As many new underlying writers are created as
/// needed to write all the lines in the input buffer.
struct SpliceLineChunkWriter<'a> {
    /// Parameters for creating the underlying writer for each new chunk.
    settings: &'a SpliceSettings,

    /// The maximum number of lines allowed for a single chunk of output.
    chunk_size: u64,

    /// Running total of number of chunks that have been completed.
    num_chunks_written: u64,

    /// Remaining capacity in number of lines in the current chunk.
    ///
    /// This number starts at `chunk_size` and decreases as lines are
    /// written. Once it reaches zero, a writer for a new chunk is
    /// initialized and this number gets reset to `chunk_size`.
    num_lines_remaining_in_current_chunk: u64,

    /// The underlying writer for the current chunk.
    ///
    /// Once the number of lines written to this writer exceeds
    /// `chunk_size`, a new writer is initialized and assigned to this
    /// field.
    inner: BufWriter<Box<dyn Write>>,

    /// Iterator that yields filenames for each chunk.
    filename_iterator: FilenameIterator<'a>,
}

impl<'a> SpliceLineChunkWriter<'a> {
    fn new(
        chunk_size: u64,
        splice_settings: &'a SpliceSettings,
    ) -> CTResult<SpliceLineChunkWriter<'a>> {
        let mut file_iterator =
            FilenameIterator::new(&splice_settings.prefix, &splice_settings.suffix)?;
        let file_name = file_iterator
            .next()
            .ok_or_else(|| CtSimpleError::new(1, "output file suffixes exhausted"))?;
        if splice_settings.verbose {
            println!("creating file {}", file_name.quote());
        }
        let buf_inner = splice_settings.splice_instantiate_current_writer(&file_name, true)?;
        Ok(SpliceLineChunkWriter {
            settings: splice_settings,
            chunk_size,
            num_lines_remaining_in_current_chunk: chunk_size,
            num_chunks_written: 0,
            inner: buf_inner,
            filename_iterator: file_iterator,
        })
    }
}

impl Write for SpliceLineChunkWriter<'_> {
    /// Implements `--lines=NUMBER`
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // 当`buf`中的行数超过当前分块剩余行数时，需要写入多个底层写入器。
        // 此循环中，每次迭代均向对应当前分块编号的底层写入器写入数据。
        let mut prev_size = 0;
        let mut total_bytes_written_size = 0;
        let separator = self.settings.separator;

        // 使用`memchr`库查找所有分隔符位置，遍历这些位置
        for size in memchr::memchr_iter(separator, buf) {
            // 若已达到当前分块的行数限制，创建新的分块及对应的底层写入器
            if self.num_lines_remaining_in_current_chunk == 0 {
                self.num_chunks_written += 1;

                // 获取新文件名，若已耗尽则返回错误
                let filename = self
                    .filename_iterator
                    .next()
                    .ok_or_else(|| std::io::Error::new(ErrorKind::Other, "输出文件后缀用尽"))?;

                // 开启详细日志时，打印创建文件信息
                if self.settings.verbose {
                    println!("创建文件 {}", filename.quote());
                }

                // 实例化当前分块对应的底层写入器
                self.inner = self
                    .settings
                    .splice_instantiate_current_writer(&filename, true)?;

                // 重置当前分块剩余行数
                self.num_lines_remaining_in_current_chunk = self.chunk_size;
            }

            // 从上一个分隔符后的第一个字符开始，到当前分隔符前的最后一个字符结束，写入一行数据
            let num_bytes_written_size =
                splice_custom_write(&buf[prev_size..=size], &mut self.inner, self.settings)?;
            total_bytes_written_size += num_bytes_written_size;
            prev_size = size + 1;
            self.num_lines_remaining_in_current_chunk -= 1;
        }

        // 写入剩余未处理部分（可能包含最后一行）
        let num_bytes_written_size =
            splice_custom_write(&buf[prev_size..buf.len()], &mut self.inner, self.settings)?;
        total_bytes_written_size += num_bytes_written_size;

        // 返回已成功写入的总字节数
        Ok(total_bytes_written_size)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Write lines to each sequential output files, limited by bytes.
///
/// This struct maintains an underlying writer representing the
/// current chunk of the output. On each call to [`write`], it writes
/// as many lines as possible to the current chunk without exceeding
/// the specified byte limit. If a single line has more bytes than the
/// limit, then fill an entire single chunk with those bytes and
/// handle the remainder of the line as if it were its own distinct
/// line. As many new underlying writers are created as needed to
/// write all the data in the input buffer.
struct SplitLineBytesChunkWriter<'a> {
    /// Parameters for creating the underlying writer for each new chunk.
    settings: &'a SpliceSettings,

    /// The maximum number of bytes allowed for a single chunk of output.
    chunk_size: u64,

    /// Running total of number of chunks that have been completed.
    num_chunks_written: usize,

    /// Remaining capacity in number of bytes in the current chunk.
    ///
    /// This number starts at `chunk_size` and decreases as lines are
    /// written. Once it reaches zero, a writer for a new chunk is
    /// initialized and this number gets reset to `chunk_size`.
    num_bytes_remaining_in_current_chunk: usize,

    /// The underlying writer for the current chunk.
    ///
    /// Once the number of bytes written to this writer exceeds
    /// `chunk_size`, a new writer is initialized and assigned to this
    /// field.
    inner: BufWriter<Box<dyn Write>>,

    /// Iterator that yields filenames for each chunk.
    filename_iterator: FilenameIterator<'a>,
}

impl<'a> SplitLineBytesChunkWriter<'a> {
    fn new(
        chunk_size: u64,
        splice_settings: &'a SpliceSettings,
    ) -> CTResult<SplitLineBytesChunkWriter<'a>> {
        let mut file_iterator =
            FilenameIterator::new(&splice_settings.prefix, &splice_settings.suffix)?;
        let file_name = file_iterator
            .next()
            .ok_or_else(|| CtSimpleError::new(1, "output file suffixes exhausted"))?;
        if splice_settings.verbose {
            println!("creating file {}", file_name.quote());
        }
        let buf_inner = splice_settings.splice_instantiate_current_writer(&file_name, true)?;
        Ok(SplitLineBytesChunkWriter {
            settings: splice_settings,
            chunk_size,
            num_bytes_remaining_in_current_chunk: usize::try_from(chunk_size).unwrap(),
            num_chunks_written: 0,
            inner: buf_inner,
            filename_iterator: file_iterator,
        })
    }
}

impl Write for SplitLineBytesChunkWriter<'_> {
    /// Write as many lines to a chunk as possible without
    /// exceeding the byte limit. If a single line has more bytes
    /// than the limit, then fill an entire single chunk with those
    /// bytes and handle the remainder of the line as if it were
    /// its own distinct line.
    ///
    /// For example: if the `chunk_size` is 8 and the input is:
    ///
    /// ```text
    /// aaaaaaaaa\nbbbb\ncccc\ndd\nee\n
    /// ```
    ///
    /// then the output gets broken into chunks like this:
    ///
    /// ```text
    /// chunk 0    chunk 1    chunk 2    chunk 3
    ///
    /// 0            1             2
    /// 01234567  89 01234   56789 012   345 6
    /// |------|  |-------|  |--------|  |---|
    /// aaaaaaaa  a\nbbbb\n  cccc\ndd\n  ee\n
    /// ```
    ///
    /// Implements `--line-bytes=SIZE`
    fn write(&mut self, mut buffer: &[u8]) -> std::io::Result<usize> {
        // 已写 mut total_bytes_written_size = 0入总字节数
        let mut total_bytes_written_size = 0;

        // 循环写入直到缓冲区为空（或发生I/O错误）
        loop {
            // 缓冲区为空，写入完成，返回已写入总字节数
            if buffer.is_empty() {
                return Ok(total_bytes_written_size);
            }

            // 当前分块已满，分配新分块并初始化对应写入器
            if self.num_bytes_remaining_in_current_chunk == 0 {
                self.num_chunks_written += 1;
                let filename = self.filename_iterator.next().ok_or_else(|| {
                    std::io::Error::new(ErrorKind::Other, "output file suffixes exhausted")
                })?;
                if self.settings.verbose {
                    println!("creating file {}", filename.quote());
                }
                self.inner = self
                    .settings
                    .splice_instantiate_current_writer(&filename, true)?;
                self.num_bytes_remaining_in_current_chunk = self.chunk_size.try_into().unwrap();
            }

            // 查找分隔符
            let separator = self.settings.separator;
            match memchr::memchr(separator, buffer) {
                // 无分隔符且缓冲区非空，尽可能多地写入字节，并在必要时切换至新分块
                None => {
                    let end_size = self.num_bytes_remaining_in_current_chunk;

                    // 这段代码虽然不太美观，但为了匹配GNU的行为而保留。如果输入数据末尾不含分隔符，
                    // 为了处理倒数第二个分块，我们假装它存在。参见line-bytes.sh。
                    if end_size == buffer.len()
                        && self.num_bytes_remaining_in_current_chunk
                            < self.chunk_size.try_into().unwrap_or(usize::MAX)
                        && buffer[buffer.len() - 1] != separator
                    {
                        self.num_bytes_remaining_in_current_chunk = 0;
                    } else {
                        let num_bytes_written = splice_custom_write(
                            &buffer[..end_size.min(buffer.len())],
                            &mut self.inner,
                            self.settings,
                        )?;
                        self.num_bytes_remaining_in_current_chunk -= num_bytes_written;
                        total_bytes_written_size += num_bytes_written;
                        buffer = &buffer[num_bytes_written..];
                    }
                }

                // 有分隔符，根据分隔符位置、当前分块剩余空间以及已写入其他行的情况，决定如何处理
                Some(i) if i < self.num_bytes_remaining_in_current_chunk => {
                    let num_bytes_written =
                        splice_custom_write(&buffer[..=i], &mut self.inner, self.settings)?;
                    self.num_bytes_remaining_in_current_chunk -= num_bytes_written;
                    total_bytes_written_size += num_bytes_written;
                    buffer = &buffer[num_bytes_written..];
                }

                // 若存在分隔符字符，且当前行（包括分隔符字符）无法放入当前分块中，
                // 同时当前分块尚未写入其他行，则尽可能多地写入字节并进入下一次迭代。
                // （参考上述示例注释中的第0个分块）
                Some(_)
                    if self.num_bytes_remaining_in_current_chunk
                        == self.chunk_size.try_into().unwrap_or(usize::MAX) =>
                {
                    let end = self.num_bytes_remaining_in_current_chunk;
                    let num_bytes_written =
                        splice_custom_write(&buffer[..end], &mut self.inner, self.settings)?;
                    self.num_bytes_remaining_in_current_chunk -= num_bytes_written;
                    total_bytes_written_size += num_bytes_written;
                    buffer = &buffer[num_bytes_written..];
                }

                // 如果存在分隔符字符，且当前行（包括分隔符字符）无法放入当前分块中，且当前分块中已至少写入过一行，则向下次迭代传递信号，
                // 表示需要创建新分块，并继续循环以尝试在新分块中写入该行。
                Some(_) => {
                    self.num_bytes_remaining_in_current_chunk = 0;
                }
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Output file parameters
struct SplitOutFile {
    filename: String,
    maybe_writer: Option<BufWriter<Box<dyn Write>>>,
    is_new: bool,
}

/// A set of output files
/// Used in [`splice_n_chunks_by_byte`], [`splice_n_chunks_by_line`]
/// and [`splice_n_chunks_by_line_round_robin`] functions.
type OutFiles = Vec<SplitOutFile>;
trait SplitManageOutFiles {
    fn instantiate_writer(
        &mut self,
        index: usize,
        split_settings: &SpliceSettings,
    ) -> CTResult<&mut BufWriter<Box<dyn Write>>>;
    /// Initialize a new set of output files
    /// Each OutFile is generated with filename, while the writer for it could be
    /// optional, to be instantiated later by the calling function as needed.
    /// Optional writers could happen in the following situations:
    /// * in [`splice_n_chunks_by_line`] and [`splice_n_chunks_by_line_round_robin`] if `elide_empty_files` parameter is set to `true`
    /// * if the number of files is greater than system limit for open files
    fn init(
        num_files: u64,
        split_settings: &SpliceSettings,
        is_writer_optional: bool,
    ) -> CTResult<Self>
    where
        Self: Sized;
    /// Get the writer for the output file by index.
    /// If system limit of open files has been reached
    /// it will try to close one of previously instantiated writers
    /// to free up resources and re-try instantiating current writer,
    /// except for `--filter` mode.
    /// The writers that get closed to free up resources for the current writer
    /// are flagged as `is_new=false`, so they can be re-opened for appending
    /// instead of created anew if we need to keep writing into them later,
    /// i.e. in case of round robin distribution as in [`splice_n_chunks_by_line_round_robin`]
    fn get_writer(
        &mut self,
        index: usize,
        split_settings: &SpliceSettings,
    ) -> CTResult<&mut BufWriter<Box<dyn Write>>>;
}

impl SplitManageOutFiles for OutFiles {
    /**
     * 初始化函数，用于创建和初始化一个输出文件集合。
     *
     * @param number_files 指定要创建的文件数量。
     * @param split_settings 包含分割设置的引用，如前缀和后缀。
     * @param writer_optional 指示是否每个文件都需要一个写入器。如果为`true`，则文件可以不关联写入器。
     * @return 返回一个包含初始化好的输出文件集合的`CTResult`。如果操作成功，`CTResult`包含初始化的实例；如果失败，包含错误信息。
     */
    fn init(
        number_files: u64,
        split_settings: &SpliceSettings,
        writer_optional: bool,
    ) -> CTResult<Self> {
        // 创建文件名迭代器，用于生成每个分割文件的名称。
        let mut file_iterator: FilenameIterator<'_> =
            FilenameIterator::new(&split_settings.prefix, &split_settings.suffix)
                .map_err(|e| io::Error::new(ErrorKind::Other, format!("{e}")))?;
        let mut output_files: Self = Self::new();
        for _ in 0..number_files {
            // 获取下一个文件名。如果文件名序列耗尽，则视为错误。
            let file_name = file_iterator
                .next()
                .ok_or_else(|| CtSimpleError::new(1, "output file suffixes exhausted"))?;
            // 根据`writer_optional`标志，决定是否为当前文件创建一个写入器。
            let maybe_writer = if writer_optional {
                None
            } else {
                // 尝试为文件实例化一个写入器。如果因系统限制而失败，并且当前不是为`--filter`子进程创建写入器，则记录为`None`。
                let instantiated =
                    split_settings.splice_instantiate_current_writer(file_name.as_str(), true);
                match instantiated {
                    Ok(writer) => Some(writer),
                    Err(e) if split_settings.filter.is_some() => {
                        return Err(e.into());
                    }
                    Err(_) => None,
                }
            };
            // 将文件名和可能的写入器添加到输出文件集合中。
            output_files.push(SplitOutFile {
                filename: file_name,
                maybe_writer,
                is_new: true,
            });
        }
        // 初始化完成，返回输出文件集合。
        Ok(output_files)
    }

    /**
     * 尝试为指定索引的文件创建一个新的写入器。
     *
     * 此函数会在系统达到文件描述符限制时，尝试关闭其他已打开的文件描述符，以腾出资源。
     * 如果 `splice_settings` 中设置了过滤器，则在创建写入器失败时会直接返回错误。
     *
     * @param &mut self 对结构体的可变引用，该结构体包含文件信息和已打开的写入器。
     * @param index 要为其创建写入器的文件索引。
     * @param splice_settings 包含拼接操作设置的引用，例如如何处理文件名和是否使用过滤器。
     * @return CTResult<&mut BufWriter<Box<dyn Write>>> 成功时返回对写入器的可变引用，失败时返回错误。
     */
    fn instantiate_writer(
        &mut self,
        index: usize,
        splice_settings: &SpliceSettings,
    ) -> CTResult<&mut BufWriter<Box<dyn Write>>> {
        let mut count = 0;
        // 尝试多次关闭文件描述符以应对系统限制，特别是当有其他进程可能占用已释放的文件描述符时
        'loop1: loop {
            let file_to_open = self[index].filename.as_str();
            let file_to_open_is_new = self[index].is_new;
            let maybe_writer = splice_settings
                .splice_instantiate_current_writer(file_to_open, file_to_open_is_new);
            if let Ok(writer) = maybe_writer {
                self[index].maybe_writer = Some(writer);
                return Ok(self[index].maybe_writer.as_mut().unwrap());
            }

            if splice_settings.filter.is_some() {
                // 在过滤器模式下，直接返回错误
                return Err(maybe_writer.err().unwrap().into());
            }

            // 如果达到系统文件描述符限制，尝试关闭其他已打开的写入器
            for (i, out_file) in self.iter_mut().enumerate() {
                if i != index && out_file.maybe_writer.is_some() {
                    out_file.maybe_writer.as_mut().unwrap().flush()?;
                    out_file.maybe_writer = None;
                    out_file.is_new = false;
                    count += 1;

                    // 再次尝试创建写入器
                    continue 'loop1;
                }
            }

            // 如果无法创建写入器且无其他文件描述符可关闭，则放弃并返回错误
            ctcore::ct_show_error!(
                "at file descriptor limit, but no file descriptor left to close. Closed {count} writers before."
            );
            return Err(maybe_writer.err().unwrap().into());
        }
    }

    /**
     * 获取一个指定索引位置的写入器（如果已存在则直接返回，否则创建新的）。
     *
     * @param &mut self 对象的可变引用，用于访问和修改对象内部状态。
     * @param index 想要获取写入器的索引位置。
     * @param splice_settings 用于创建写入器的拼接设置。
     * @return CTResult<&mut BufWriter<Box<dyn Write>>> 如果成功，返回指定索引位置的写入器的可变引用；
     *         如果失败，返回错误信息。
     */
    fn get_writer(
        &mut self,
        index: usize,
        splice_settings: &SpliceSettings,
    ) -> CTResult<&mut BufWriter<Box<dyn Write>>> {
        if self[index].maybe_writer.is_some() {
            // 如果指定索引处的写入器已存在，则直接返回这个写入器的可变引用。
            Ok(self[index].maybe_writer.as_mut().unwrap())
        } else {
            // 如果指定索引处的写入器不存在，则创建一个新的写入器，并记录下来供未来使用。
            self.instantiate_writer(index, splice_settings)
        }
    }
}

/// Split a file or STDIN into a specific number of chunks by byte.
///
/// When file size cannot be evenly divided into the number of chunks of the same size,
/// the first X chunks are 1 byte longer than the rest,
/// where X is a modulus reminder of (file size % number of chunks)
///
/// In Kth chunk of N mode - writes to STDOUT the contents of the chunk identified by `kth_chunk`
///
/// In N chunks mode - this function always creates one output file for each chunk, even
/// if there is an error reading or writing one of the chunks or if
/// the input file is truncated. However, if the `--filter` option is
/// being used, then files will only be created if `$FILE` variable was used
/// in filter command,
/// i.e. `split -n 10 --filter='head -c1 > $FILE' in`
///
/// # Errors
///
/// This function returns an error if there is a problem reading from
/// `reader` or writing to one of the output files or stdout.
///
/// # See also
///
/// * [`splice_n_chunks_by_line`], which splits its input into a specific number of chunks by line.
///
/// Implements `--number=CHUNKS`
/// Where CHUNKS
/// * N
/// * K/N
/**
 *   根据指定的切分设置，将输入数据拆分为多个块。
 *
 * @param splice_settings 包含切分操作所需全部设置的结构体引用。
 * @param reader 输入数据的缓冲读取器引用。
 * @param num_chunks 请求拆分的块数量。
 * @param opt_kth_chunk 选项，指定需要提取的第 k 个块。如果未指定，则按 N 块模式处理。
 * @return CTResult<()>，成功返回 ()，错误返回包含错误信息的 CtResult 错误。
 *
 * 此函数首先尝试获取输入数据的总字节数，然后根据请求的切分模式和设置，将数据拆分为多个块，
 * 并将这些块写入标准输出或指定的文件中。
 */
fn splice_n_chunks_by_byte<R>(
    splice_settings: &SpliceSettings,
    reader: &mut R,
    num_chunks: u64,
    opt_kth_chunk: Option<u64>,
) -> CTResult<()>
where
    R: BufRead,
{
    // 尝试获取输入的总字节数
    let initial_buffer = &mut Vec::new();
    let mut num_bytes = splice_get_input_size(
        &splice_settings.input,
        reader,
        initial_buffer,
        &splice_settings.io_blksize,
    )?;
    let mut reader = initial_buffer.chain(reader);

    // 如果输入文件为空，并且我们无法在 Kth 块 of N 块模式中确定第 K 块，那么立即终止
    if opt_kth_chunk.is_some() && num_bytes == 0 {
        return Ok(());
    }

    // 如果请求的块数量超过了输入中的字节数量，则根据设置调整块数量
    let num_chunks =
        if opt_kth_chunk.is_none() && splice_settings.elide_empty_files && num_chunks > num_bytes {
            num_bytes
        } else {
            num_chunks
        };

    // 如果我们将写入零个输出块，则立即终止
    if num_chunks == 0 {
        return Ok(());
    }

    // 准备输出：如果在 Kth 块 of N 块模式，则写入标准输出；否则，为每个块创建一个写入器
    let mut splice_stdout_writer = std::io::stdout().lock();
    let mut output_files: OutFiles = OutFiles::new();

    // 计算每个块的基础大小和余数，用于之后计算块大小
    let chunk_size_base = num_bytes / num_chunks;
    let chunk_size_reminder = num_bytes % num_chunks;

    // 如果在 N 块模式，为每个块创建一个写入器
    if opt_kth_chunk.is_none() {
        output_files = OutFiles::init(num_chunks, splice_settings, false)?;
    }

    // 遍历每个块，从读取器中读取数据并写入相应的输出
    for size in 1_u64..=num_chunks {
        let chunk_size = chunk_size_base + (chunk_size_reminder > size - 1) as u64;
        let buf = &mut Vec::new();
        if num_bytes > 0 {
            // 读取 `chunk_size` 字节到 `buf`，除了最后一个块。
            // 最后一个块会接收所有剩余字节，确保我们不会留下任何字节。
            let limit_size = {
                if size == num_chunks {
                    num_bytes
                } else {
                    chunk_size
                }
            };

            let read_size = reader.by_ref().take(limit_size).read_to_end(buf);

            match read_size {
                Ok(n_bytes) => {
                    num_bytes -= n_bytes as u64;
                }
                Err(error) => {
                    return Err(CtSimpleError::new(
                        1,
                        format!(
                            "{}: cannot read from input : {}",
                            splice_settings.input, error
                        ),
                    ));
                }
            }

            // 根据是否指定了第 K 个块，将数据写入标准输出或文件
            match opt_kth_chunk {
                Some(chunk_number) => {
                    if size == chunk_number {
                        splice_stdout_writer.write_all(buf)?;
                        break;
                    }
                }
                None => {
                    let idx = (size - 1) as usize;
                    let writer = output_files.get_writer(idx, splice_settings)?;
                    writer.write_all(buf)?;
                }
            }
        } else {
            break;
        }
    }
    Ok(())
}

/// Split a file or STDIN into a specific number of chunks by line.
///
/// It is most likely that input cannot be evenly divided into the number of chunks
/// of the same size in bytes or number of lines, since we cannot break lines.
/// It is also likely that there could be empty files (having `elide_empty_files` is disabled)
/// when a long line overlaps one or more chunks.
///
/// In Kth chunk of N mode - writes to STDOUT the contents of the chunk identified by `kth_chunk`
/// Note: the `elide_empty_files` flag is ignored in this mode
///
/// In N chunks mode - this function always creates one output file for each chunk, even
/// if there is an error reading or writing one of the chunks or if
/// the input file is truncated. However, if the `--filter` option is
/// being used, then files will only be created if `$FILE` variable was used
/// in filter command,
/// i.e. `split -n l/10 --filter='head -c1 > $FILE' in`
///
/// # Errors
///
/// This function returns an error if there is a problem reading from
/// `reader` or writing to one of the output files.
///
/// # See also
///
/// * [`splice_n_chunks_by_byte`], which splits its input into a specific number of chunks by byte.
///
/// Implements `--number=CHUNKS`
/// Where CHUNKS
/// * l/N
/// * l/K/N
///   根据指定的切分设置，将输入数据切分为多个块，并根据情况将这些块写入标准输出或多个文件。
///
/// # 参数
/// * `splice_settings` - 包含切分操作所需各种设置的结构体引用。
/// * `splice_reader` - 一个可缓冲的读取器，用于读取输入数据。
/// * `number_chunks` - 预计要切分的块的数量。
/// * `kth_chunk` - 一个可选值，指明需要提取的特定块的索引（从1开始）。如果提供，只处理该特定块。
///
/// # 返回值
/// * `CTResult<()>` - 如果操作成功，返回`Ok(())`；如果遇到错误，返回错误信息。
fn splice_n_chunks_by_line<R>(
    splice_settings: &SpliceSettings,
    splice_reader: &mut R,
    number_chunks: u64,
    kth_chunk: Option<u64>,
) -> CTResult<()>
where
    R: BufRead,
{
    // 初始化，计算每块的字节数
    let initial_buffer = &mut Vec::new();
    let number_bytes = splice_get_input_size(
        &splice_settings.input,
        splice_reader,
        initial_buffer,
        &splice_settings.io_blksize,
    )?;
    let reader_buffer = initial_buffer.chain(splice_reader);

    // 处理输入为空的情况
    if number_bytes == 0 && (kth_chunk.is_some() || splice_settings.elide_empty_files) {
        return Ok(());
    }

    // 准备输出：确定是写入标准输出还是多个文件
    let mut stdout_writer = std::io::stdout().lock();
    let mut output_files: OutFiles = OutFiles::new();

    // 计算基本块大小和余数，用于确定应写入的字节数
    let chunk_size_base = number_bytes / number_chunks;
    let chunk_size_reminder = number_bytes % number_chunks;

    // 初始化文件输出，如果启用，则创建文件或管道
    if kth_chunk.is_none() {
        output_files = OutFiles::init(
            number_chunks,
            splice_settings,
            splice_settings.elide_empty_files,
        )?;
    }

    // 主循环：切分并写入数据
    let mut chunk_number = 1;
    let separator = splice_settings.separator;
    let mut number_bytes_should_be_written = chunk_size_base + (chunk_size_reminder > 0) as u64;
    let mut number_bytes_written = 0;

    for line_result in reader_buffer.split(separator) {
        let mut line = line_result?;
        // 检查是否需要在行尾添加分隔符
        if (number_bytes_written + line.len() as u64) < number_bytes {
            line.push(separator);
        }
        let size = line.as_slice();

        // 根据是否指定`kth_chunk`，将数据写入标准输出或文件
        match kth_chunk {
            Some(kth) => {
                if chunk_number == kth {
                    stdout_writer.write_all(size)?;
                }
            }
            None => {
                let idx = (chunk_number - 1) as usize;
                let writer = output_files.get_writer(idx, splice_settings)?;
                splice_custom_write_all(size, writer, splice_settings)?;
            }
        }

        // 更新已写入的字节数，并根据需要前进到下一个块
        let number_line_bytes = size.len() as u64;
        number_bytes_written += number_line_bytes;
        let mut skipped = -1;
        while number_bytes_should_be_written <= number_bytes_written {
            number_bytes_should_be_written +=
                chunk_size_base + (chunk_size_reminder > chunk_number) as u64;
            chunk_number += 1;
            skipped += 1;
        }

        // 如果因为长行而跳文件，则调整块编号以保持文件名过了块，并且启用了省略空的连续性
        if splice_settings.elide_empty_files && skipped > 0 && kth_chunk.is_none() {
            chunk_number -= skipped as u64;
        }

        // 如果已经处理完指定的块，则终止循环
        if let Some(kth) = kth_chunk {
            if chunk_number > kth {
                break;
            }
        }
    }
    Ok(())
}

/// Split a file or STDIN into a specific number of chunks by line, but
/// assign lines via round-robin.
/// Note: There is no need to know the size of the input upfront for this method,
/// since the lines are assigned to chunks randomly and the size of each chunk
/// does not need to be estimated. As a result, "infinite" inputs are supported
/// for this method, i.e. `yes | split -n r/10` or `yes | split -n r/3/11`
///
/// In Kth chunk of N mode - writes to stdout the contents of the chunk identified by `kth_chunk`
///
/// In N chunks mode - this function always creates one output file for each chunk, even
/// if there is an error reading or writing one of the chunks or if
/// the input file is truncated. However, if the `--filter` option is
/// being used, then files will only be created if `$FILE` variable was used
/// in filter command,
/// i.e. `split -n r/10 --filter='head -c1 > $FILE' in`
///
/// # Errors
///
/// This function returns an error if there is a problem reading from
/// `reader` or writing to one of the output files.
///
/// # See also
///
/// * [`splice_n_chunks_by_line`], which splits its input into a specific number of chunks by line.
///
/// Implements `--number=CHUNKS`
/// Where CHUNKS
/// * r/N
/// * r/K/N
/**
 *   将输入数据拆分并按照指定的块数或特定块写入不同的输出目标。
 *
 * 此函数主要用于根据提供的拆分设置和读取器，将数据拆分成多个部分，然后将这些部分写入不同的文件或标准输出。
 * 可以在两种模式下运行：N块模式和Kth块模式。
 * - 在N块模式下，数据将被平均分割，并写入指定数量的文件中。
 * - 在Kth块模式下，只有指定的块（Kth块）会被写入标准输出。
 *
 * @param splice_settings 拆分设置，包含拆分相关的配置信息，如分隔符等。
 * @param splice_reader 一个可缓冲的读取器，用于读取待拆分的数据。
 * @param number_chunks 指定要拆分的块数。
 * @param kth_chunk 指定的块号，用于Kth块模式。如果为None，则运行在N块模式下。
 * @return 返回一个执行结果，如果成功则为()`，否则为错误信息。
 */
fn splice_n_chunks_by_line_round_robin<R>(
    splice_settings: &SpliceSettings,
    splice_reader: &mut R,
    number_chunks: u64,
    kth_chunk: Option<u64>,
) -> CTResult<()>
where
    R: BufRead,
{
    // 初始化输出目标。如果在N块模式下，将创建多个文件作为输出；如果在Kth块模式下，输出将直接写入标准输出。
    let mut stdout_writer = std::io::stdout().lock();
    let mut output_files: OutFiles = OutFiles::new();

    // 在N块模式下初始化输出文件。
    if kth_chunk.is_none() {
        output_files = OutFiles::init(
            number_chunks,
            splice_settings,
            splice_settings.elide_empty_files,
        )?;
    }

    let num_chunks: usize = number_chunks.try_into().unwrap();
    let separator = splice_settings.separator;
    let mut closed_writers_size = 0;

    let mut size = 0;
    loop {
        let line = &mut Vec::new();
        let number_bytes_read = splice_reader.by_ref().read_until(separator, line)?;

        // 如果没有更多的数据可读，则退出循环。
        if number_bytes_read == 0 {
            break;
        };

        let bytes = line.as_slice();
        match kth_chunk {
            Some(chunk_number) => {
                // 在Kth块模式下，根据当前块的编号决定是否将数据写入标准输出。
                if (size % num_chunks) == (chunk_number - 1) as usize {
                    stdout_writer.write_all(bytes)?;
                }
            }
            None => {
                // 在N块模式下，根据当前块的编号选择对应的文件写入数据。
                let writer = output_files.get_writer(size % num_chunks, splice_settings)?;
                let writer_stdin_open = splice_custom_write_all(bytes, writer, splice_settings)?;
                if !writer_stdin_open {
                    closed_writers_size += 1;
                }
            }
        }
        size += 1;
        if closed_writers_size == num_chunks {
            // 如果所有写入器都已关闭，则停止读取数据。
            break;
        }
    }
    Ok(())
}

#[allow(clippy::cognitive_complexity)]
/**
 * 根据给定的分割设置将输入数据分割成多个部分。
 *
 * # 参数
 * `splice_settings`: &SpliceSettings - 包含分割操作所需全部设置的引用，如输入源、分割策略等。
 *
 * # 返回值
 * `CTResult<()>` - 表示操作成功或失败的结果。成功时返回`()`，失败时返回包含错误信息的`Err`。
 */
fn split(splice_settings: &SpliceSettings) -> CTResult<()> {
    // 根据输入源创建一个读取器
    let read_box = if splice_settings.input == "-" {
        Box::new(stdin()) as Box<dyn Read>
    } else {
        let r = File::open(Path::new(&splice_settings.input)).map_err_context(|| {
            format!("cannot open {} for reading", splice_settings.input.quote())
        })?;
        Box::new(r) as Box<dyn Read>
    };

    // 根据是否指定了IO块大小，创建一个具有相应缓冲区的读取器
    let mut reader = if let Some(c) = splice_settings.io_blksize {
        BufReader::with_capacity(c.try_into().unwrap(), read_box)
    } else {
        BufReader::new(read_box)
    };

    // 根据分割策略执行相应的分割逻辑
    match splice_settings.strategy {
        Strategy::Number(StrategyNumberType::Bytes(num_chunks)) => {
            // 按字节分割成指定数量的块
            splice_n_chunks_by_byte(splice_settings, &mut reader, num_chunks, None)
        }
        Strategy::Number(StrategyNumberType::KthBytes(chunk_number, num_chunks)) => {
            // 按字节分割，并保留指定的第K个块
            splice_n_chunks_by_byte(splice_settings, &mut reader, num_chunks, Some(chunk_number))
        }
        Strategy::Number(StrategyNumberType::Lines(num_chunks)) => {
            // 按行分割成指定数量的块
            splice_n_chunks_by_line(splice_settings, &mut reader, num_chunks, None)
        }
        Strategy::Number(StrategyNumberType::KthLines(chunk_number, num_chunks)) => {
            // 按行分割，并保留指定的第K个块
            splice_n_chunks_by_line(splice_settings, &mut reader, num_chunks, Some(chunk_number))
        }
        Strategy::Number(StrategyNumberType::RoundRobin(num_chunks)) => {
            // 使用轮询方式按行分割成指定数量的块
            splice_n_chunks_by_line_round_robin(splice_settings, &mut reader, num_chunks, None)
        }
        Strategy::Number(StrategyNumberType::KthRoundRobin(chunk_number, num_chunks)) => {
            // 使用轮询方式按行分割，并保留指定的第K个块
            splice_n_chunks_by_line_round_robin(
                splice_settings,
                &mut reader,
                num_chunks,
                Some(chunk_number),
            )
        }
        Strategy::Lines(chunk_size) => {
            // 按指定行大小进行分割
            let mut splice_writer = SpliceLineChunkWriter::new(chunk_size, splice_settings)?;
            match std::io::copy(&mut reader, &mut splice_writer) {
                Ok(_) => Ok(()),
                Err(e) => match e.kind() {
                    // 处理复制过程中出现的错误
                    ErrorKind::Other => Err(CtSimpleError::new(1, format!("{e}"))),
                    _ => Err(uio_error!(e, "input/output error")),
                },
            }
        }
        Strategy::Bytes(chunk_size) => {
            // 按指定字节大小进行分割
            let mut splice_writer = SpliceByteChunkWriter::new(chunk_size, splice_settings)?;
            match std::io::copy(&mut reader, &mut splice_writer) {
                Ok(_) => Ok(()),
                Err(e) => match e.kind() {
                    // 处理复制过程中出现的错误
                    ErrorKind::Other => Err(CtSimpleError::new(1, format!("{e}"))),
                    _ => Err(uio_error!(e, "input/output error")),
                },
            }
        }
        Strategy::LineBytes(chunk_size) => {
            // 在行边界上按指定字节大小进行分割
            let mut splice_writer = SplitLineBytesChunkWriter::new(chunk_size, splice_settings)?;
            match std::io::copy(&mut reader, &mut splice_writer) {
                Ok(_) => Ok(()),
                Err(e) => match e.kind() {
                    // 处理复制过程中出现的错误
                    ErrorKind::Other => Err(CtSimpleError::new(1, format!("{e}"))),
                    _ => Err(uio_error!(e, "input/output error")),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod tests_handle_preceding_options {
        use super::*;

        use std::fs;
        use tempfile::Builder;

        #[test]
        fn test_handle_options_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));

            assert_eq!(args[1], OsString::from("--version"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));

            assert_eq!(args[1], OsString::from("-V"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));

            assert_eq!(args[1], OsString::from("--help"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));

            assert_eq!(args[1], OsString::from("-h"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_b() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-b"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("-b"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));

            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10K"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10K"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10M"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10M"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10G"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10G"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10T"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10T"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10P"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10P"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10E"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10E"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10Z"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10Z"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10Y"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10Y"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10r() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10R"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10R"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_bytes_10q() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--bytes", "10Q"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--bytes"));
            assert_eq!(args[3], OsString::from("10Q"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_c() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-C"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("-C"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_line_bytes_10k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10K"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10K"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_bytes_10m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10M"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10M"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_bytes_10g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10G"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10G"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_bytes_10t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10T"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10T"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_bytes_10p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10P"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10P"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_bytes_10e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10E"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10E"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_bytes_10z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10Z"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10Z"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_bytes_10y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10Y"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10Y"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_bytes_10r() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10R"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10R"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_line_bytes_10q() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--line-bytes", "10Q"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--line-bytes"));
            assert_eq!(args[3], OsString::from("10Q"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_lines() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--lines"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--lines"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_l() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-l"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("-l"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_lines_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--lines", "10"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--lines"));
            assert_eq!(args[3], OsString::from("10"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_lines_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--lines", "100"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--lines"));
            assert_eq!(args[3], OsString::from("100"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_lines_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--lines", "1000"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--lines"));
            assert_eq!(args[3], OsString::from("1000"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--number"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_n() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-n"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("-n"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--number", "10"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("10"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--number", "100"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("100"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--number", "1000"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("1000"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--additional-suffix"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--additional-suffix"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_additional_suffix_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--additional-suffix",
                "10",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--additional-suffix"));
            assert_eq!(args[3], OsString::from("10"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_additional_suffix_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--additional-suffix",
                "100",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--additional-suffix"));
            assert_eq!(args[3], OsString::from("100"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_additional_suffix_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--additional-suffix",
                "1000",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--additional-suffix"));
            assert_eq!(args[3], OsString::from("1000"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--filter"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--filter"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--filter", "ls"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--filter"));
            assert_eq!(args[3], OsString::from("ls"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_filter_cat() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--filter", "cat"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--filter"));
            assert_eq!(args[3], OsString::from("cat"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_filter_tail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--filter", "tail"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--filter"));
            assert_eq!(args[3], OsString::from("tail"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_10_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--number",
                "10",
                "--additional-suffix",
                ".txt",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("10"));
            assert_eq!(args[4], OsString::from("--additional-suffix"));
            assert_eq!(args[5], OsString::from(".txt"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_100_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--number",
                "100",
                "--additional-suffix",
                ".txt",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("100"));
            assert_eq!(args[4], OsString::from("--additional-suffix"));
            assert_eq!(args[5], OsString::from(".txt"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_1000_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--number",
                "1000",
                "--additional-suffix",
                ".txt",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("1000"));
            assert_eq!(args[4], OsString::from("--additional-suffix"));
            assert_eq!(args[5], OsString::from(".txt"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_10_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--number",
                "10",
                "--filter",
                "ls",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("10"));
            assert_eq!(args[4], OsString::from("--filter"));
            assert_eq!(args[5], OsString::from("ls"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_100_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--number",
                "100",
                "--filter",
                "ls",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("100"));
            assert_eq!(args[4], OsString::from("--filter"));
            assert_eq!(args[5], OsString::from("ls"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_1000_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--number",
                "1000",
                "--filter",
                "ls",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("1000"));
            assert_eq!(args[4], OsString::from("--filter"));
            assert_eq!(args[5], OsString::from("ls"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_additional_suffix_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--additional-suffix",
                ".txt",
                "--filter",
                "ls",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--additional-suffix"));
            assert_eq!(args[3], OsString::from(".txt"));
            assert_eq!(args[4], OsString::from("--filter"));
            assert_eq!(args[5], OsString::from("ls"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_additional_suffix_filter_cat() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--additional-suffix",
                ".txt",
                "--filter",
                "cat",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--additional-suffix"));
            assert_eq!(args[3], OsString::from(".txt"));
            assert_eq!(args[4], OsString::from("--filter"));
            assert_eq!(args[5], OsString::from("cat"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_additional_suffix_filter_cd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--additional-suffix",
                ".txt",
                "--filter",
                "cd",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--additional-suffix"));
            assert_eq!(args[3], OsString::from(".txt"));
            assert_eq!(args[4], OsString::from("--filter"));
            assert_eq!(args[5], OsString::from("cd"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_additional_suffix_filter_tail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--additional-suffix",
                ".txt",
                "--filter",
                "tail",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--additional-suffix"));
            assert_eq!(args[3], OsString::from(".txt"));
            assert_eq!(args[4], OsString::from("--filter"));
            assert_eq!(args[5], OsString::from("tail"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_number_additional_suffix_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--number",
                "3",
                "--additional-suffix",
                ".txt",
                "--filter",
                "ls",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 8);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--number"));
            assert_eq!(args[3], OsString::from("3"));
            assert_eq!(args[4], OsString::from("--additional-suffix"));
            assert_eq!(args[5], OsString::from(".txt"));
            assert_eq!(args[6], OsString::from("--filter"));
            assert_eq!(args[7], OsString::from("ls"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-e"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("-e"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_elide_empty_files() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--elide-empty-files"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            assert_eq!(args[2], OsString::from("--elide-empty-files"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_d() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-d"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            //
            assert_eq!(args[2], OsString::from("-d"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_elide_numeric_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--numeric-suffixes"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            //
            assert_eq!(args[2], OsString::from("--numeric-suffixes"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_elide_numeric_suffixes_2() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--numeric-suffixes", "2"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            //
            assert_eq!(args[2], OsString::from("--numeric-suffixes"));
            assert_eq!(args[3], OsString::from("2"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_elide_numeric_suffixes_3() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--numeric-suffixes", "3"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            //
            assert_eq!(args[2], OsString::from("--numeric-suffixes"));
            assert_eq!(args[3], OsString::from("3"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_x() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-x"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));
            //
            assert_eq!(args[2], OsString::from("-x"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_hex_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--hex-suffixes"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--hex-suffixes"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_hex_suffixes_2() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--hex-suffixes", "2"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--hex-suffixes"));
            assert_eq!(args[3], OsString::from("2"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_hex_suffixes_3() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--hex-suffixes", "3"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--hex-suffixes"));
            assert_eq!(args[3], OsString::from("3"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_a() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-a"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("-a"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--suffix-length"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--suffix-length"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_suffix_length_2() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--suffix-length", "2"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--suffix-length"));
            assert_eq!(args[3], OsString::from("2"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_suffix_length_3() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--suffix-length", "3"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--suffix-length"));
            assert_eq!(args[3], OsString::from("3"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--verbose"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--verbose"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_verbose_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--verbose",
                "--suffix-length",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--verbose"));
            assert_eq!(args[3], OsString::from("--suffix-length"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_verbose_suffix_length_2() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--verbose",
                "--suffix-length",
                "2",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 5);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--verbose"));
            assert_eq!(args[3], OsString::from("--suffix-length"));
            assert_eq!(args[4], OsString::from("2"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_verbose_suffix_length_3() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "--verbose",
                "--suffix-length",
                "3",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 5);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--verbose"));
            assert_eq!(args[3], OsString::from("--suffix-length"));
            assert_eq!(args[4], OsString::from("3"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_a_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-a", "--verbose"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("-a"));
            assert_eq!(args[3], OsString::from("--verbose"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_a_verbose_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "-a",
                "--verbose",
                "--suffix-length",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 5);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("-a"));
            assert_eq!(args[3], OsString::from("--verbose"));
            assert_eq!(args[4], OsString::from("--suffix-length"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_a_verbose_suffix_length_2() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "-a",
                "--verbose",
                "--suffix-length",
                "2",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("-a"));
            assert_eq!(args[3], OsString::from("--verbose"));
            assert_eq!(args[4], OsString::from("--suffix-length"));
            assert_eq!(args[5], OsString::from("2"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_a_verbose_suffix_length_3() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename,
                "-a",
                "--verbose",
                "--suffix-length",
                "3",
            ];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("-a"));
            assert_eq!(args[3], OsString::from("--verbose"));
            assert_eq!(args[4], OsString::from("--suffix-length"));
            assert_eq!(args[5], OsString::from("3"));
            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "-t", "'\0'"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("-t"));
            assert_eq!(args[3], OsString::from("'\0'"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_separator_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--separator", "'\0'"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--separator"));
            assert_eq!(args[3], OsString::from("'\0'"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_separator_n() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--separator", "'\n'"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--separator"));
            assert_eq!(args[3], OsString::from("'\n'"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_separator_r() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--separator", "'\r'"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--separator"));
            assert_eq!(args[3], OsString::from("'\r'"));

            assert_eq!(obs_lines, None);
        }

        #[test]
        fn test_handle_options_separator_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename, "--separator", "'\t'"];
            let (args, obs_lines) = split_handle_obsolete(args.iter().map(|s| OsString::from(s)));
            assert_eq!(args.len(), 4);
            assert_eq!(args[0], OsString::from(ctcore::ct_util_name()));
            assert_eq!(args[1], OsString::from(filename));

            assert_eq!(args[2], OsString::from("--separator"));
            assert_eq!(args[3], OsString::from("'\t'"));

            assert_eq!(obs_lines, None);
        }
    }

    mod tests_ct_app {
        use super::*;

        use clap::error::ErrorKind;
        use std::fs;
        use tempfile::Builder;

        #[test]
        fn test_ct_app_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind(),
                clap::error::ErrorKind::DisplayVersion
            );
        }

        #[test]
        fn test_ct_app_v() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_h() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_b() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-b", "5"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(matches.get_one::<String>(OPT_BYTES), Some(&"5".to_string()));
        }

        #[test]
        fn test_ct_app_bytes_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "100"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"100".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "1000"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"1000".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10K"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10K".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10M"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10M".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10G"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10G".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10T"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10T".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10P"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10P".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10E"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10E".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Z"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10Z".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Y"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10Y".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10r() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10R"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10R".to_string())
            );
        }

        #[test]
        fn test_ct_app_bytes_10q() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Q"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_BYTES),
                Some(&"10Q".to_string())
            );
        }

        #[test]
        fn test_ct_app_c() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-C", "5"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_LINE_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_LINE_BYTES),
                Some(&"5".to_string())
            );
        }

        #[test]
        fn test_ct_app_lines_bytes_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "10"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_LINE_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_LINE_BYTES),
                Some(&"10".to_string())
            );
        }

        #[test]
        fn test_ct_app_lines_bytes_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "100"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_LINE_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_LINE_BYTES),
                Some(&"100".to_string())
            );
        }

        #[test]
        fn test_ct_app_lines_bytes_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "1000"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_LINE_BYTES));
            assert_eq!(
                matches.get_one::<String>(OPT_LINE_BYTES),
                Some(&"1000".to_string())
            );
        }

        #[test]
        fn test_ct_app_l() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-l", "5"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_LINES));
            assert_eq!(matches.get_one::<String>(OPT_LINES), Some(&"5".to_string()));
        }

        #[test]
        fn test_ct_app_lines_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "10"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_LINES));
            assert_eq!(
                matches.get_one::<String>(OPT_LINES),
                Some(&"10".to_string())
            );
        }

        #[test]
        fn test_ct_app_lines_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "100"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_LINES));
            assert_eq!(
                matches.get_one::<String>(OPT_LINES),
                Some(&"100".to_string())
            );
        }

        #[test]
        fn test_ct_app_lines_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "1000"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_LINES));
            assert_eq!(
                matches.get_one::<String>(OPT_LINES),
                Some(&"1000".to_string())
            );
        }

        #[test]
        fn test_ct_app_n() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-n", "5"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_NUMBER));
            assert_eq!(
                matches.get_one::<String>(OPT_NUMBER),
                Some(&"5".to_string())
            );
        }

        #[test]
        fn test_ct_app_number_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--number", "10"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_NUMBER));
            assert_eq!(
                matches.get_one::<String>(OPT_NUMBER),
                Some(&"10".to_string())
            );
        }

        #[test]
        fn test_ct_app_number_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--number", "100"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_NUMBER));
            assert_eq!(
                matches.get_one::<String>(OPT_NUMBER),
                Some(&"100".to_string())
            );
        }

        #[test]
        fn test_ct_app_number_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--number", "1000"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_NUMBER));
            assert_eq!(
                matches.get_one::<String>(OPT_NUMBER),
                Some(&"1000".to_string())
            );
        }

        #[test]
        fn test_ct_app_additional_suffix_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "10",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_ADDITIONAL_SUFFIX));
            assert_eq!(
                matches.get_one::<String>(OPT_ADDITIONAL_SUFFIX),
                Some(&"10".to_string())
            );
        }

        #[test]
        fn test_ct_app_additional_suffix_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "100",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_ADDITIONAL_SUFFIX));
            assert_eq!(
                matches.get_one::<String>(OPT_ADDITIONAL_SUFFIX),
                Some(&"100".to_string())
            );
        }

        #[test]
        fn test_ct_app_additional_suffix_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "1000",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_ADDITIONAL_SUFFIX));
            assert_eq!(
                matches.get_one::<String>(OPT_ADDITIONAL_SUFFIX),
                Some(&"1000".to_string())
            );
        }

        #[test]
        fn test_ct_app_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "ls"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_FILTER));
            assert_eq!(
                matches.get_one::<String>(OPT_FILTER),
                Some(&"ls".to_string())
            );
        }

        #[test]
        fn test_ct_app_filter_cat() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cat"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_FILTER));
            assert_eq!(
                matches.get_one::<String>(OPT_FILTER),
                Some(&"cat".to_string())
            );
        }

        #[test]
        fn test_ct_app_filter_cd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cd"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_FILTER));
            assert_eq!(
                matches.get_one::<String>(OPT_FILTER),
                Some(&"cd".to_string())
            );
        }

        #[test]
        fn test_ct_app_filter_tail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "tail"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_FILTER));
            assert_eq!(
                matches.get_one::<String>(OPT_FILTER),
                Some(&"tail".to_string())
            );
        }

        #[test]
        fn test_ct_app_number_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_NUMBER));
            assert_eq!(
                matches.get_one::<String>(OPT_NUMBER),
                Some(&"10".to_string())
            );
            assert!(matches.contains_id(OPT_FILTER));
            assert_eq!(
                matches.get_one::<String>(OPT_FILTER),
                Some(&"ls".to_string())
            );
        }

        #[test]
        fn test_ct_app_n_number_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-n",
                "--number",
                "10",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.contains_id(OPT_NUMBER));
            assert_eq!(
                matches.get_one::<String>(OPT_NUMBER),
                Some(&"--number".to_string())
            );
            assert!(matches.contains_id(OPT_FILTER));
            assert_eq!(
                matches.get_one::<String>(OPT_FILTER),
                Some(&"ls".to_string())
            );
        }

        #[test]
        fn test_ct_app_number_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--additional-suffix",
                ".txt",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_NUMBER));
            assert_eq!(
                matches.get_one::<String>(OPT_NUMBER),
                Some(&"10".to_string())
            );

            assert!(matches.contains_id(OPT_ADDITIONAL_SUFFIX));
            assert_eq!(
                matches.get_one::<String>(OPT_ADDITIONAL_SUFFIX),
                Some(&".txt".to_string())
            );
        }

        #[test]
        fn test_ct_app_filter_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--filter",
                "ls",
                "--additional-suffix",
                ".txt",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_ADDITIONAL_SUFFIX));
            assert_eq!(
                matches.get_one::<String>(OPT_ADDITIONAL_SUFFIX),
                Some(&".txt".to_string())
            );

            assert!(matches.contains_id(OPT_FILTER));
            assert_eq!(
                matches.get_one::<String>(OPT_FILTER),
                Some(&"ls".to_string())
            );
        }

        #[test]
        fn test_ct_app_number_additional_suffix_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--additional-suffix",
                ".txt",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_NUMBER));
            assert_eq!(
                matches.get_one::<String>(OPT_NUMBER),
                Some(&"10".to_string())
            );

            assert!(matches.contains_id(OPT_ADDITIONAL_SUFFIX));
            assert_eq!(
                matches.get_one::<String>(OPT_ADDITIONAL_SUFFIX),
                Some(&".txt".to_string())
            );

            assert!(matches.contains_id(OPT_FILTER));
            assert_eq!(
                matches.get_one::<String>(OPT_FILTER),
                Some(&"ls".to_string())
            );
        }

        #[test]
        fn test_ct_app_elide_empty_files() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--elide-empty-files"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_ELIDE_EMPTY_FILES));
            // println!("matches: {:?}", matches.get_one::<bool>(OPT_ELIDE_EMPTY_FILES));
            assert_eq!(
                matches.get_one::<bool>(OPT_ELIDE_EMPTY_FILES),
                Some(true).as_ref()
            );
        }

        #[test]
        fn test_ct_app_e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-e"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_ELIDE_EMPTY_FILES));
            // println!("matches: {:?}", matches.get_one::<bool>(OPT_ELIDE_EMPTY_FILES));
            assert_eq!(
                matches.get_one::<bool>(OPT_ELIDE_EMPTY_FILES),
                Some(true).as_ref()
            );
        }

        #[test]
        fn test_ct_app_d() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-d", "txt"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(matches.get_one::<String>(OPT_NUMERIC_SUFFIXES), None);
        }

        #[test]
        fn test_ct_app_numeric_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--numeric-suffixes=.txt"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_NUMERIC_SUFFIXES));

            assert_eq!(
                matches.get_one::<String>(OPT_NUMERIC_SUFFIXES),
                Some(&".txt".to_string())
            );
        }

        #[test]
        fn test_ct_app_d_numeric_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--numeric-suffixes=.txt",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_NUMERIC_SUFFIXES));

            assert_eq!(
                matches.get_one::<String>(OPT_NUMERIC_SUFFIXES),
                Some(&".txt".to_string())
            );
        }

        // #[test]
        #[test]
        fn test_ct_app_x() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-x", "111"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(matches.get_one::<String>(OPT_HEX_SUFFIXES), None);
        }

        #[test]
        fn test_ct_app_hex_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--hex-suffixes=11"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_HEX_SUFFIXES));

            assert_eq!(
                matches.get_one::<String>(OPT_HEX_SUFFIXES),
                Some(&"11".to_string())
            );
        }

        #[test]
        fn test_ct_app_d_hex_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-d", "--hex-suffixes=11"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_HEX_SUFFIXES));

            assert_eq!(
                matches.get_one::<String>(OPT_HEX_SUFFIXES),
                Some(&"11".to_string())
            );
        }

        #[test]
        fn test_ct_app_a() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-a", "11"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(OPT_SUFFIX_LENGTH),
                Some(&"11".to_string())
            );
        }

        #[test]
        fn test_ct_app_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--suffix-length=11"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_SUFFIX_LENGTH));

            assert_eq!(
                matches.get_one::<String>(OPT_SUFFIX_LENGTH),
                Some(&"11".to_string())
            );
        }

        #[test]
        fn test_ct_app_d_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--suffix-length=11",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_SUFFIX_LENGTH));

            assert_eq!(
                matches.get_one::<String>(OPT_SUFFIX_LENGTH),
                Some(&"11".to_string())
            );
        }

        #[test]
        fn test_ct_app_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--verbose"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_VERBOSE));

            assert_eq!(matches.get_one::<bool>(OPT_VERBOSE), Some(true).as_ref());
        }

        #[test]
        fn test_ct_app_a_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-a", "111", "--verbose"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(OPT_SUFFIX_LENGTH),
                Some(&"111".to_string())
            );

            assert!(matches.contains_id(OPT_VERBOSE));

            assert_eq!(matches.get_one::<bool>(OPT_VERBOSE), Some(true).as_ref());
        }

        #[test]
        fn test_ct_app_suffix_length_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--suffix-length=11",
                "--verbose",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_SUFFIX_LENGTH));

            assert_eq!(
                matches.get_one::<String>(OPT_SUFFIX_LENGTH),
                Some(&"11".to_string())
            );

            assert!(matches.contains_id(OPT_VERBOSE));

            assert_eq!(matches.get_one::<bool>(OPT_VERBOSE), Some(true).as_ref());
        }

        #[test]
        fn test_ct_app_d_suffix_length_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--suffix-length=11",
                "--verbose",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.contains_id(OPT_SUFFIX_LENGTH));

            assert_eq!(
                matches.get_one::<String>(OPT_SUFFIX_LENGTH),
                Some(&"11".to_string())
            );

            assert!(matches.contains_id(OPT_VERBOSE));

            assert_eq!(matches.get_one::<bool>(OPT_VERBOSE), Some(true).as_ref());
        }

        #[test]
        fn test_ct_app_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "-t", "'\0'"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(OPT_SEPARATOR),
                Some(&"'\0'".to_string())
            );
        }

        #[test]
        fn test_ct_app_separator_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "'\0'"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(OPT_SEPARATOR),
                Some(&"'\0'".to_string())
            );
        }

        #[test]
        fn test_ct_app_separator_n() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "'\n'"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(OPT_SEPARATOR),
                Some(&"'\n'".to_string())
            );
        }

        #[test]
        fn test_ct_app_separator_r() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "'\r'"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(OPT_SEPARATOR),
                Some(&"'\r'".to_string())
            );
        }

        #[test]
        fn test_ct_app_separator_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "'\t'"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(OPT_SEPARATOR),
                Some(&"'\t'".to_string())
            );
        }
    }

    mod tests_ct_main {
        use super::*;

        use std::fs;
        use tempfile::Builder;

        #[test]
        fn test_ct_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_b() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-b", "5"];

            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "100"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "1000"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10K"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10M"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10G"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10T"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10P"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10E"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Z"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Y"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10r() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10R"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes_10q() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Q"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_c() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-C", "5"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_lines_bytes_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "10"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_lines_bytes_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "100"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_lines_bytes_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "1000"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_l() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-l", "5"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_lines_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "10"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_lines_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "100"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_lines_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "1000"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_n() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-n", "5"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_number_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--number", "10"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_number_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--number", "100"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_number_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--number", "1000"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_additional_suffix_10() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "10",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_additional_suffix_100() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "100",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_additional_suffix_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "1000",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "ls"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_filter_cat() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cat"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_filter_cd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cd"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_filter_tail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "tail"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_number_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--filter",
                "ls",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_n_number_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-n",
                "--number",
                "10",
                "--filter",
                "ls",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_number_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--additional-suffix",
                ".txt",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_filter_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--filter",
                "ls",
                "--additional-suffix",
                ".txt",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_number_additional_suffix_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--additional-suffix",
                ".txt",
                "--filter",
                "ls",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_elide_empty_files() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--elide-empty-files"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-e"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-d", "txt"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with("txt") && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_numeric_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--numeric-suffixes=11"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_numeric_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--numeric-suffixes=11",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        // #[test]
        #[test]
        fn test_ct_main_x() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-x", "111"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('1') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_hex_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--hex-suffixes=11"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_hex_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-d", "--hex-suffixes=11"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_a() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-a", "11"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--suffix-length=11"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--suffix-length=11",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--verbose"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_a_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-a", "111", "--verbose"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_suffix_length_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--suffix-length=11",
                "--verbose",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_suffix_length_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--suffix-length=11",
                "--verbose",
            ];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }
        ////////////////////////
        #[test]
        fn test_ct_main_t_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-t", "\0"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_separator_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\0"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_separator_n() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\n"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_separator_r() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\r"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_separator_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\t"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_t_fail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-t", "'\0'"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }

        #[test]
        fn test_ct_main_separator_zero_fail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "'\0'"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }

        #[test]
        fn test_ct_main_separator_n_fail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "'\n'"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }

        #[test]
        fn test_ct_main_separator_r_fail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "'\r'"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }

        #[test]
        fn test_ct_main_separator_t_fail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "'\t'"];
            let result = split_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }
    }

    mod tests_settings {

        use crate::SpliceSettings;
        use crate::ct_app;
        use std::fs;
        use std::fs::File;
        use std::path::Path;
        use tempfile::Builder;

        #[test]
        fn test_from_invalid_strategy() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
            let result = command.try_get_matches_from(args);

            // Create a dummy `Option<String>` value for `obs_lines`
            let obs_lines = Some("dummy".to_string());

            // Call the `from` method and assert the result is `Err` with `SettingsError::Strategy`
            let result = SpliceSettings::from(&result.unwrap(), &obs_lines);
            assert!(result.is_err());
        }

        #[test]
        fn test_instantiate_current_writer_same_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
            let result = command.try_get_matches_from(args);

            let mut settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "input.txt";
            // Set the `input` to the same as `filename`
            settings.input = filename.to_string();

            // Call the `instantiate_current_writer` method and assert the result is `Err`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_different_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }
        #[test]
        fn test_instantiate_current_writer_b() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-b", "5"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_b_15() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-b", "15"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "100"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "1000"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10k() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10K"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10m() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10M"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10g() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10G"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10t() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10T"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10p() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10P"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10e() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10E"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10z() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Z"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10y() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Y"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10r() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10R"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_bytes_10q() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Q"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_c() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-C", "5"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_lines_bytes_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_lines_bytes_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "100"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_lines_bytes_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "1000"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_l() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-l", "5"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_lines_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_lines_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "100"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_lines_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "1000"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_n() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-n", "5"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_number_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--number", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_number_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--number", "100"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_number_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--number", "1000"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_additional_suffix_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "10",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_additional_suffix_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "100",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_additional_suffix_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "1000",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "ls"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_filter_cat() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cat"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_filter_cd() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cd"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_filter_tail() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "tail"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_number_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_number_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--additional-suffix",
                ".txt",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_filter_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--filter",
                "ls",
                "--additional-suffix",
                ".txt",
            ];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_number_additional_suffix_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--additional-suffix",
                ".txt",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_elide_empty_files() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--elide-empty-files"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_e() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-e"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_d() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-d", "txt"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_numeric_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--numeric-suffixes=333"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        // #[test]
        #[test]
        fn test_instantiate_current_writer_x() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-x", "111"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_hex_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--hex-suffixes=11"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_d_hex_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-d", "--hex-suffixes=11"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_a() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-a", "11"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--suffix-length=11"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_d_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--suffix-length=11",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--verbose"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_a_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-a", "111", "--verbose"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_suffix_length_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--suffix-length=11",
                "--verbose",
            ];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_d_suffix_length_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--suffix-length=11",
                "--verbose",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_t() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "-t", "\0"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_separator_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\0"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_separator_n() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\n"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_separator_r() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\r"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_separator_t() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\t"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_separator_t_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--separator",
                "\t",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_instantiate_current_writer_verbose_elide_empty_files_separator_t_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--verbose",
                " --elide-empty-files",
                "--separator",
                "\t",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let filename = "output.txt";

            // Call the `instantiate_current_writer` method and assert the result is `Ok`
            let result = settings.splice_instantiate_current_writer(filename, true);
            let file_path = Path::new(filename);
            match fs::remove_file(file_path) {
                Ok(()) => {
                    // println!("文件删除成功");
                }
                Err(_e) => {
                    // eprintln!("File remove fail: {}", e)
                }
            }

            assert!(result.is_ok());
        }
    }
    mod tests_setting_error {

        use crate::FilenameSuffixError;
        use crate::SpliceSettingsError;
        use crate::StrategyError;
        use crate::split_should_extract_obs_lines;

        #[test]
        fn test_strategy_lines_does_not_require_usage() {
            let error = SpliceSettingsError::Strategy(StrategyError::Lines(
                ctcore::ct_parse_size::ParseSizeError::SizeTooBig(
                    ctcore::ct_parse_size::ParseSizeError::InvalidSuffix(
                        "Invalid number of lines".to_string(),
                    )
                    .to_string(),
                ),
            ));

            assert_eq!(error.splice_requires_usage(), false);
        }

        #[test]
        fn test_strategy_bytes_does_not_require_usage() {
            let error = SpliceSettingsError::Strategy(StrategyError::Bytes(
                ctcore::ct_parse_size::ParseSizeError::SizeTooBig(
                    ctcore::ct_parse_size::ParseSizeError::InvalidSuffix(
                        "Invalid number of lines".to_string(),
                    )
                    .to_string(),
                ),
            ));

            assert_eq!(error.splice_requires_usage(), false);
        }

        #[test]
        fn test_strategy_multiple_ways_requires_usage() {
            let error = SpliceSettingsError::Strategy(StrategyError::MultipleWays);

            assert_eq!(error.splice_requires_usage(), true);
        }

        #[test]
        fn test_suffix_contains_separator_requires_usage() {
            let error = SpliceSettingsError::Suffix(FilenameSuffixError::ContainsSeparator(
                "Suffix contains a directory separator, which is not allowed".to_string(),
            ));

            assert_eq!(error.splice_requires_usage(), true);
        }

        #[test]
        fn test_suffix_not_parsable_does_not_require_usage() {
            let error = SpliceSettingsError::Suffix(FilenameSuffixError::NotParsable(
                "Invalid suffix length parameter".to_string(),
            ));

            assert_eq!(error.splice_requires_usage(), false);
        }

        #[test]
        fn test_suffix_too_small_does_not_require_usage() {
            let error = SpliceSettingsError::Suffix(FilenameSuffixError::TooSmall(20));

            assert_eq!(error.splice_requires_usage(), false);
        }

        #[test]
        fn test_multi_character_separator_does_not_require_usage() {
            let error = SpliceSettingsError::MultiCharacterSeparator(
                "Multi-character (Invalid) separator".to_string(),
            );

            assert_eq!(error.splice_requires_usage(), false);
        }

        #[test]
        fn test_multiple_separator_characters_does_not_require_usage() {
            let error = SpliceSettingsError::MultipleSeparatorCharacters;

            assert_eq!(error.splice_requires_usage(), false);
        }

        #[test]
        fn test_filter_with_kth_chunk_number_does_not_require_usage() {
            let error = SpliceSettingsError::FilterWithKthChunkNumber;

            assert_eq!(error.splice_requires_usage(), false);
        }

        #[test]
        fn test_invalid_io_block_size_does_not_require_usage() {
            let error =
                SpliceSettingsError::InvalidIOBlockSize("Invalid IO block size".to_string());

            assert_eq!(error.splice_requires_usage(), false);
        }

        #[cfg(windows)]
        #[test]
        fn test_not_supported_does_not_require_usage() {
            let error = SpliceSettingsError::NotSupported;

            assert_eq!(error.splice_requires_usage(), false);
        }

        #[test]
        fn test_should_extract_obs_lines_h() {
            assert_eq!(split_should_extract_obs_lines("-h", &false, &false), true);
        }

        #[test]
        fn test_should_extract_obs_lines_help() {
            assert_eq!(
                split_should_extract_obs_lines("--help", &false, &false),
                false
            );
        }

        #[test]
        fn test_should_extract_obs_lines_a() {
            assert_eq!(split_should_extract_obs_lines("-a", &false, &false), false);
        }

        #[test]
        fn test_should_extract_obs_lines_b() {
            assert_eq!(split_should_extract_obs_lines("-b", &false, &false), false);
        }

        #[test]
        fn test_should_extract_obs_lines_c() {
            assert_eq!(split_should_extract_obs_lines("-C", &false, &false), false);
        }

        #[test]
        fn test_should_extract_obs_lines_l() {
            assert_eq!(split_should_extract_obs_lines("-l", &false, &false), false);
        }

        #[test]
        fn test_should_extract_obs_lines_n() {
            assert_eq!(split_should_extract_obs_lines("-n", &false, &false), false);
        }

        #[test]
        fn test_should_extract_obs_lines_t() {
            assert_eq!(split_should_extract_obs_lines("-t", &false, &false), false);
        }

        #[test]
        fn test_should_extract_obs_lines_abc() {
            assert_eq!(
                split_should_extract_obs_lines("-abc", &false, &false),
                false
            );
        }

        #[test]
        fn test_should_extract_obs_lines_hvalue() {
            assert_eq!(
                split_should_extract_obs_lines("-hvalue", &false, &false),
                true
            );
        }

        #[test]
        fn test_should_extract_obs_lines_hv() {
            assert_eq!(split_should_extract_obs_lines("-hv", &false, &false), true);
        }

        use super::splice_handle_extract_obs_lines;
        use std::ffi::OsString;

        #[test]
        fn test_handle_extract_obs_lines_no_obs_lines() {
            let slice = "-x100";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("-x")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
        #[test]
        fn test_handle_extract_obs_lines_no_obs_lines_with_short_options() {
            let slice = "-x100a";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("-xa")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
        #[test]
        fn test_handle_extract_obs_lines_no_obs_lines_with_long_options() {
            let slice = "--extract-obs-lines=100";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("--extract-obs-lines=")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
        #[test]
        fn test_handle_extract_obs_lines_no_obs_lines_with_long_options_and_value() {
            let slice = "--extract-obs-lines=100a";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("--extract-obs-lines=a")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
        #[test]
        fn test_handle_extract_obs_lines_no_obs_lines_with_long_options_and_value_and_short_options()
         {
            let slice = "--extract-obs-lines=100a-x";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("--extract-obs-lines=a-x")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }

        #[test]
        fn test_handle_extract_obs_lines_with_obs_lines() {
            let slice = "-x200a4";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("-xa4")));
            assert_eq!(obs_lines, Some("200".to_string()));
        }

        #[test]
        fn test_handle_extract_obs_lines_with_short_options_before() {
            let slice = "-xd100";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("-xd")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }

        #[test]
        fn test_handle_extract_obs_lines_with_short_options_after() {
            let slice = "-100de";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("-de")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }

        #[test]
        fn test_handle_extract_obs_lines_with_short_options_before_and_after() {
            let slice = "-x100de";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("-xde")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
        #[test]
        fn test_handle_extract_obs_lines_with_long_options_before() {
            let slice = "--x100";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("--x")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
        #[test]
        fn test_handle_extract_obs_lines_with_long_options_after() {
            let slice = "--100de";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("--de")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }

        #[test]
        fn test_handle_extract_obs_lines_with_long_options_before_and_after() {
            let slice = "--x100de";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("--xde")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
        #[test]
        fn test_handle_extract_obs_lines_with_long_options_before_and_after_and_value() {
            let slice = "--x100de=100";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("--xde=100")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
        #[test]
        fn test_handle_extract_obs_lines_with_long_options_before_and_after_and_value_and_value() {
            let slice = "--x100de=100a100";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("--xde=100a100")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
        #[test]
        fn test_handle_extract_obs_lines_with_long_options_before_and_after_and_value_and_value_and_value()
         {
            let slice = "--x100de=100a100a100";
            let mut obs_lines = None;
            let result = splice_handle_extract_obs_lines(slice, &mut obs_lines);
            assert_eq!(result, Some(OsString::from("--xde=100a100a100")));
            assert_eq!(obs_lines, Some("100".to_string()));
        }
    }

    mod test_split {

        use crate::SpliceSettings;

        use crate::ct_app;
        use crate::split;
        use std::fs;
        use std::fs::File;

        use tempfile::Builder;

        #[test]
        fn test_split_same_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();

            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_different_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_b() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-b", "5"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_b_15() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-b", "15"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "100"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "1000"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10k() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10K"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10m() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10M"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10g() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10G"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10t() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10T"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10p() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10P"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10e() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10E"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10z() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Z"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10y() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Y"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10r() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10R"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_bytes_10q() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Q"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_c() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-C", "5"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_lines_bytes_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_lines_bytes_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "100"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_lines_bytes_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "1000"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_l() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-l", "5"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_lines_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_lines_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "100"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_lines_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--lines", "1000"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_n() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-n", "5"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_number_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--number", "10"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_number_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--number", "100"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_number_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--number", "1000"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_additional_suffix_10() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "10",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_additional_suffix_100() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "100",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_additional_suffix_1000() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--additional-suffix",
                "1000",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_filter_ls() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "ls"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_filter_cat() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cat"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_filter_cd() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cd"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_filter_tail() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--filter", "tail"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_number_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_number_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--additional-suffix",
                ".txt",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_filter_additional_suffix() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--filter",
                "ls",
                "--additional-suffix",
                ".txt",
            ];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_number_additional_suffix_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--number",
                "10",
                "--additional-suffix",
                ".txt",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_elide_empty_files() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--elide-empty-files"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_e() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-e"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_d() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-d", "txt"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with("txt") && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_numeric_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--numeric-suffixes=3"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        // #[test]
        #[test]
        fn test_split_x() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-x", "111"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('1') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_hex_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--hex-suffixes=11"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_d_hex_suffixes() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-d", "--hex-suffixes=11"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_a() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-a", "11"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--suffix-length=11"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_d_suffix_length() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--suffix-length=11",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "--verbose"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_a_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), filename1, "-a", "111", "--verbose"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_suffix_length_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--suffix-length=11",
                "--verbose",
            ];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_d_suffix_length_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-d",
                "--suffix-length=11",
                "--verbose",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_t() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "-t", "\0"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_separator_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\0"];
            let result = command.try_get_matches_from(args);
            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_separator_n() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\n"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_separator_r() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\r"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_separator_t() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\t"];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_elide_empty_files_separator_t() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--elide-empty-files",
                "--separator",
                "\t",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_verbose_separator_t() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--verbose",
                "--separator",
                "\t",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_verbose_elide_empty_files_separator_t() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--verbose",
                "--elide-empty-files",
                "--separator",
                "\t",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }

        #[test]
        fn test_split_verbose_elide_empty_files_separator_t_filter() {
            let temp_dir = Builder::new()
                .prefix("tests_instantiate_current_writer_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_111");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--verbose",
                "--elide-empty-files",
                "--separator",
                "\t",
                "--filter",
                "ls",
            ];
            let result = command.try_get_matches_from(args);

            let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
            let result = split(&settings);

            // 获取当前目录
            let current_dir = std::env::current_dir().unwrap();

            // 获取当前目录下的文件和目录
            let entries = fs::read_dir(current_dir).unwrap();

            // 遍历当前目录下的每一个文件和目录
            for entry in entries {
                let entry = entry.unwrap();
                let file_path = entry.path();

                // 检查文件名是否以 'x' 开头，并且是文件而不是目录
                if let Some(file_name) = file_path.file_name() {
                    if let Some(name) = file_name.to_str() {
                        if name.starts_with('x') && file_path.is_file() {
                            // 删除文件
                            let _ = fs::remove_file(file_path);
                        }
                    }
                }
            }

            assert!(result.is_ok());
        }
    }
    #[cfg(test)]
    mod tests_tool_implementation {
        use super::*;
        use ctcore::Tool;
        use std::ffi::OsString;

        #[test]
        fn test_tool_implementation() {
            let tool = Split::default();

            // 测试 name 方法
            assert_eq!(tool.name(), "split");

            // 测试 command 方法
            let command = tool.command();
            assert!(command.get_name().contains("split"));

            // 测试 execute 方法
            let args = vec![OsString::from("split"), OsString::from("--help")];
            assert!(tool.execute(&args).is_err()); // --help参数通常会返回错误
        }
    }
}
