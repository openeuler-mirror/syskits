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

//! tac 是一个非常有用的 Linux 命令，它的功能是反向读取和输出文件的内容。
//! 这个命令的名字是 cat（concatenate，连接）的反向拼写，因此它以相反的顺序显示文件的行。

extern crate rust_i18n;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, strip_errno};

use ctcore::Tool;
use ctcore::ct_gnu_regex::{GnuRegex, GnuRegexCompileOptions, GnuRegexError};
use ctcore::ct_quoting_style::escape_shell_bytes_with_classifier;
use memchr::memmem;
use memmap2::Mmap;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::io::{Read, Write, stdin, stdout};
use std::os::unix::ffi::OsStrExt;
use std::{fs::File, path::Path};
use sys_locale::get_locale;

// 定义配置标志常量
pub mod tac_flags {
    pub const TAC_BEFORE: &str = "before";
    pub const TAC_REGEX: &str = "regex";
    pub const TAC_SEPARATOR: &str = "separator";
    pub const TAC_FILE: &str = "file";
}

/// tac 命令的配置结构体
///
/// 包含所有需要的配置选项：
/// - `is_before`: 是否在分隔符之前附加内容
/// - `is_regex`: 是否将分隔符作为正则表达式处理
/// - `separator`: 用于分隔行的字符串
/// - `files`: 要处理的文件列表
#[derive(Debug, Clone, PartialEq, Eq)]
struct TacFlags {
    is_before: bool,
    is_regex: bool,
    separator: Vec<u8>,
    files: Vec<OsString>,
}

impl Default for TacFlags {
    /// 提供 TacFlags 的默认值
    ///
    /// # 返回值
    /// 返回一个新的 TacFlags 实例，其中：
    /// - `is_before` = false
    /// - `is_regex` = false
    /// - `separator` = "\n"
    /// - `files` = ["-"] (表示标准输入)
    fn default() -> Self {
        Self {
            is_before: false,
            is_regex: false,
            separator: b"\n".to_vec(),
            files: vec![OsString::from("-")],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TacRow {
    pub source_name: String,
    pub file_index: usize,
    pub row_index: usize,
    pub chunk: String,
    pub byte_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TacSemantic {
    pub separator_kind: String,
    pub separator_text: String,
    pub before: bool,
    pub rows: Vec<TacRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

impl TacFlags {
    /// 从命令行参数解析创建 TacFlags 实例
    ///
    /// # 参数
    /// * `matches` - 解析后的命令行参数
    ///
    /// # 返回值
    /// 返回 `CTResult<TacFlags>`，包含解析后的配置
    ///
    /// # 错误
    /// 如果参数解析失败，返回相应的错误
    fn new(matches: &ArgMatches) -> CTResult<Self> {
        // 布尔标志提取
        let before = matches.get_flag(tac_flags::TAC_BEFORE);
        let regex = matches.get_flag(tac_flags::TAC_REGEX);

        // 字符串类型参数提取
        let separator = matches
            .get_one::<OsString>(tac_flags::TAC_SEPARATOR)
            .map_or_else(|| b"\n".to_vec(), |value| value.as_bytes().to_vec());

        if regex && separator.is_empty() {
            return Err(TacError::EmptyRegexSeparator.into());
        }

        // 固定空分隔符表示 NUL 字节。
        let separator = if separator.is_empty() {
            vec![0]
        } else {
            separator
        };

        // 向量类型参数提取
        let files = matches
            .get_many::<OsString>(tac_flags::TAC_FILE)
            .map_or_else(|| vec![OsString::from("-")], |v| v.cloned().collect());

        Ok(Self {
            is_before: before,
            is_regex: regex,
            separator,
            files,
        })
    }
}

#[derive(Debug)]
pub enum TacError {
    /// 用户给定的正则表达式无效。
    InvalidRegex(String),

    /// 正则搜索内部错误。
    RegexSearch,

    /// GNU正则偏移量无法表示当前记录。
    RecordTooLarge,

    /// 正则模式不允许空分隔符。
    EmptyRegexSeparator,

    /// tac 的参数无效。
    InvalidArgument(OsString),

    /// 无法打开指定的文件。
    OpenError(OsString, std::io::Error),

    /// 读取文件或标准输入的内容时出错。参数是文件名和导致此错误的底层 [`std::io::Error`]。
    ReadError(String, std::io::Error),

    /// 读取标准输入时出错。
    StdinReadError(std::io::Error),

    /// 写入（反转的）文件或标准输入内容时出错。参数是导致此错误的底层 [`std::io::Error`]。
    WriteError(std::io::Error),

    /// 刷新标准输出时出错。
    FlushError(std::io::Error),
}

impl CTError for TacError {
    fn code(&self) -> i32 {
        1
    }
}

impl Error for TacError {}

fn tac_quote_path(path: &OsStr, always_quote: bool) -> String {
    if path.to_str().is_some() {
        return if always_quote {
            path.quote().to_string()
        } else {
            path.maybe_quote().to_string()
        };
    }

    String::from_utf8(escape_shell_bytes_with_classifier(
        path.as_bytes(),
        |bytes| {
            let byte = bytes[0];
            (1, byte.is_ascii_graphic() || byte == b' ')
        },
    ))
    .expect("shell-escaped file names are valid UTF-8")
}

impl Display for TacError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&tac_error_message(self, &rust_i18n::locale()))
    }
}

fn tac_error_message(error: &TacError, locale: &str) -> String {
    match error {
        TacError::InvalidRegex(message) => message.clone(),
        TacError::RegexSearch => t!("tac.errors.regex_search", locale = locale).to_string(),
        TacError::RecordTooLarge => t!("tac.errors.record_too_large", locale = locale).to_string(),
        TacError::EmptyRegexSeparator => {
            t!("tac.errors.empty_separator", locale = locale).to_string()
        }
        TacError::InvalidArgument(path) => format!(
            "{}: {}: Invalid argument",
            tac_quote_path(path, false),
            t!("tac.errors.read_error", locale = locale)
        ),
        TacError::OpenError(path, source) => format!(
            "{}: {}",
            t!(
                "tac.errors.failed_open",
                locale = locale,
                path = tac_quote_path(path, true)
            ),
            strip_errno(source)
        ),
        TacError::ReadError(source, error) => format!(
            "{source}: {}: {}",
            t!("tac.errors.read_error", locale = locale),
            strip_errno(error)
        ),
        TacError::StdinReadError(error) => {
            let source = t!("tac.errors.standard_input", locale = locale);
            format!(
                "{}: {}: {}",
                tac_quote_path(OsStr::new(source.as_str()), false),
                t!("tac.errors.read_error", locale = locale),
                strip_errno(error)
            )
        }
        TacError::WriteError(_) => t!("tac.errors.write_error", locale = locale).to_string(),
        TacError::FlushError(error) => format!(
            "{}: {}",
            t!("tac.errors.write_error", locale = locale),
            strip_errno(error)
        ),
    }
}

/// tac 命令的主要实现函数
///
/// # 参数
/// * `writer` - 实现了 Write trait 的输出目标
/// * `args` - 命令行参数
///
/// # 返回值
/// 返回 `CTResult<()>`，表示命令执行的结果
pub fn tac_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    let _sigpipe_guard = SigpipeGuard::for_cli();
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
    }
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let settings = tac_parse_invocation(args)?;

    // 使用配置执行主要逻辑
    tac(writer, &settings)
}

struct SigpipeGuard {
    previous: ctcore::libc::sighandler_t,
}

impl SigpipeGuard {
    #[cfg(target_os = "linux")]
    fn for_cli() -> Option<Self> {
        if parent_ignores_sigpipe() {
            return None;
        }

        let previous =
            unsafe { ctcore::libc::signal(ctcore::libc::SIGPIPE, ctcore::libc::SIG_DFL) };
        (previous != ctcore::libc::SIG_ERR).then_some(Self { previous })
    }

    #[cfg(not(target_os = "linux"))]
    fn for_cli() -> Option<Self> {
        None
    }
}

impl Drop for SigpipeGuard {
    fn drop(&mut self) {
        unsafe {
            ctcore::libc::signal(ctcore::libc::SIGPIPE, self.previous);
        }
    }
}

#[cfg(target_os = "linux")]
fn parent_ignores_sigpipe() -> bool {
    let parent = unsafe { ctcore::libc::getppid() };
    let Ok(status) = std::fs::read_to_string(format!("/proc/{parent}/status")) else {
        return false;
    };
    sigpipe_is_ignored_in_status(&status)
}

#[cfg(target_os = "linux")]
fn sigpipe_is_ignored_in_status(status: &str) -> bool {
    let Some(mask) = status
        .lines()
        .find_map(|line| line.strip_prefix("SigIgn:\t"))
        .and_then(|mask| u64::from_str_radix(mask, 16).ok())
    else {
        return false;
    };
    mask & (1_u64 << (ctcore::libc::SIGPIPE - 1)) != 0
}

/// 创建并配置命令行参数解析器
///
/// # 返回值
/// 返回配置好的 `Command` 实例，用于解析命令行参数
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("tac.about");
    let usage_description = t!("tac.usage");

    let args = vec![
        Arg::new(tac_flags::TAC_BEFORE)
            .short('b')
            .long(tac_flags::TAC_BEFORE)
            .help(t!("tac.clap.tac_before"))
            .action(ArgAction::SetTrue),
        Arg::new(tac_flags::TAC_REGEX)
            .short('r')
            .long(tac_flags::TAC_REGEX)
            .help(t!("tac.clap.tac_regex"))
            .action(ArgAction::SetTrue),
        Arg::new(tac_flags::TAC_SEPARATOR)
            .short('s')
            .long(tac_flags::TAC_SEPARATOR)
            .help(t!("tac.clap.tac_separator"))
            .value_parser(OsStringValueParser::new())
            .value_name("STRING"),
        Arg::new(tac_flags::TAC_FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .args_override_self(true)
        .infer_long_args(true)
        .args(&args)
}

/// 使用正则表达式处理并反向输出数据
///
/// # 参数
/// * `writer` - 输出目标
/// * `data` - 要处理的数据
/// * `pattern` - 用于分割的正则表达式模式
/// * `before` - 是否在分隔符之前附加内容
///
/// # 返回值
/// 返回 `std::io::Result<()>`，表示写入操作的结果
#[cfg_attr(not(test), allow(dead_code))]
fn tac_buffer_regex<W: Write>(
    writer: &mut W,
    data: &[u8],
    pattern: &mut GnuRegex,
    before: bool,
) -> CTResult<()> {
    let segments = tac_collect_regex_segments(data, pattern, before)?;
    tac_write_segments(writer, &segments).map_err(|error| TacError::WriteError(error).into())
}

/// 使用固定字符串作为分隔符反向输出数据
///
/// # 参数
/// * `writer` - 输出目标
/// * `data` - 要处理的数据
/// * `before` - 是否在分隔符之前附加内容
/// * `separator` - 用于分割的字符串
///
/// # 返回值
/// 返回 `std::io::Result<()>`，表示写入操作的结果
#[cfg_attr(not(test), allow(dead_code))]
fn tac_buffer<W: Write>(
    writer: &mut W,
    data: &[u8],
    before: bool,
    separator: &[u8],
) -> std::io::Result<()> {
    tac_write_segments(
        writer,
        &tac_collect_string_segments(data, before, separator),
    )
}

/// 从标准输入读取数据
///
/// # 返回值
/// 返回 `CTResult<Vec<u8>>`，包含读取的数据或错误信息
fn read_from_stdin() -> CTResult<Vec<u8>> {
    if ctcore::ct_stdin_was_closed() {
        return Err(
            tac_stdin_read_error(std::io::Error::from_raw_os_error(ctcore::libc::EBADF)).into(),
        );
    }

    let mut buffer = Vec::new();
    ctcore::ct_io::stdin_reader_box()
        .read_to_end(&mut buffer)
        .map_err(tac_stdin_read_error)?;
    Ok(buffer)
}

fn tac_stdin_read_error(error: std::io::Error) -> TacError {
    let error = if error.raw_os_error() == Some(ctcore::libc::EISDIR) {
        std::io::Error::from_raw_os_error(ctcore::libc::EINVAL)
    } else {
        error
    };
    TacError::StdinReadError(error)
}

fn open_file(path: &Path) -> CTResult<File> {
    let file = File::open(path)
        .map_err(|error| TacError::OpenError(path.as_os_str().to_os_string(), error))?;
    if file
        .metadata()
        .map_err(|error| TacError::ReadError(tac_quote_path(path.as_os_str(), false), error))?
        .is_dir()
    {
        return Err(TacError::InvalidArgument(path.as_os_str().to_os_string()).into());
    }

    Ok(file)
}

fn read_from_file(mut file: File, path: &Path) -> CTResult<Vec<u8>> {
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|error| TacError::ReadError(tac_quote_path(path.as_os_str(), false), error))?;

    Ok(buffer)
}

/// 文件数据的枚举类型，支持内存映射和缓冲区两种模式
#[derive(Debug)]
enum FileData {
    Mapped(Mmap),
    Buffer(Vec<u8>),
}

impl AsRef<[u8]> for FileData {
    /// 获取数据的字节切片引用
    fn as_ref(&self) -> &[u8] {
        match self {
            FileData::Mapped(mmap) => mmap.as_ref(),
            FileData::Buffer(buf) => buf.as_ref(),
        }
    }
}

/// 获取文件数据，优先使用内存映射方式
///
/// # 参数
/// * `filename` - 文件名，"-" 表示标准输入
///
/// # 返回值
/// 返回 `CTResult<FileData>`，包含文件数据或错误信息
fn get_file_data(filename: &OsStr) -> CTResult<FileData> {
    if filename.as_bytes() == b"-" {
        if ctcore::ct_io::injected_stdin_bytes().is_some() {
            let buffer = read_from_stdin()?;
            return Ok(FileData::Buffer(buffer));
        }
        // 处理标准输入
        if let Some(mmap) = tac_try_mmap_stdin() {
            Ok(FileData::Mapped(mmap))
        } else {
            let buffer = read_from_stdin()?;
            Ok(FileData::Buffer(buffer))
        }
    } else {
        // 处理普通文件
        let path = Path::new(filename);
        let file = open_file(path)?;

        if let Some(mmap) = tac_try_mmap_file(&file) {
            Ok(FileData::Mapped(mmap))
        } else {
            let buffer = read_from_file(file, path)?;
            Ok(FileData::Buffer(buffer))
        }
    }
}

fn tac_parse_invocation(args: impl ctcore::Args) -> CTResult<TacFlags> {
    let args = prepare_tac_args_with_mode(args, std::env::var_os("POSIXLY_CORRECT").is_some());
    let matches = ct_app().try_get_matches_from(args)?;
    TacFlags::new(&matches)
}

fn prepare_tac_args_with_mode(args: impl ctcore::Args, posixly_correct: bool) -> Vec<OsString> {
    let mut args = args.collect::<Vec<_>>();
    if !posixly_correct {
        return args;
    }

    let mut index = 1;
    while index < args.len() {
        let bytes = args[index].as_encoded_bytes();
        if bytes == b"--" {
            break;
        }
        if bytes == b"-" || !bytes.starts_with(b"-") {
            args.insert(index, OsString::from("--"));
            break;
        }

        let consumes_next = if let Some(long) = bytes.strip_prefix(b"--") {
            !long.contains(&b'=') && b"separator".starts_with(long)
        } else {
            bytes[1..]
                .iter()
                .position(|option| *option == b's')
                .is_some_and(|position| position + 2 == bytes.len())
        };
        index += if consumes_next { 2 } else { 1 };
    }

    args
}

fn tac_separator_kind(settings: &TacFlags) -> &'static str {
    if settings.is_regex { "regex" } else { "string" }
}

fn tac_lossy_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

impl From<GnuRegexError> for TacError {
    fn from(error: GnuRegexError) -> Self {
        match error {
            GnuRegexError::Compile(message) => {
                Self::InvalidRegex(String::from_utf8_lossy(&message).into_owned())
            }
            GnuRegexError::Search => Self::RegexSearch,
            GnuRegexError::RecordTooLarge => Self::RecordTooLarge,
        }
    }
}

fn tac_collect_regex_segments(
    data: &[u8],
    pattern: &mut GnuRegex,
    before: bool,
) -> Result<Vec<Vec<u8>>, TacError> {
    let mut segments = Vec::new();
    let mut search_end = data.len();
    let mut past_end = data.len();
    let mut first_match = true;

    while let Some(found) = pattern
        .search_backward(&data[..search_end])
        .map_err(TacError::from)?
    {
        let (start, end) = (found.start, found.end);
        if before {
            segments.push(data[start..past_end].to_vec());
            past_end = start;
        } else {
            // A separator ending at EOF does not create an empty trailing record.
            if !first_match || end != past_end {
                segments.push(data[end..past_end].to_vec());
            }
            past_end = end;
            first_match = false;
        }

        search_end = start;
    }

    segments.push(data[..past_end].to_vec());
    Ok(segments)
}

fn tac_collect_string_segments(data: &[u8], before: bool, separator: &[u8]) -> Vec<Vec<u8>> {
    let slen = separator.len();
    let mut segments = Vec::new();
    let mut following_line_start = data.len();

    for i in memmem::rfind_iter(data, separator) {
        let segment = if before {
            let segment = data[i..following_line_start].to_vec();
            following_line_start = i;
            segment
        } else {
            let segment = data[i + slen..following_line_start].to_vec();
            following_line_start = i + slen;
            segment
        };
        segments.push(segment);
    }

    segments.push(data[0..following_line_start].to_vec());
    segments
}

fn tac_write_segments<W: Write>(writer: &mut W, segments: &[Vec<u8>]) -> std::io::Result<()> {
    for segment in segments {
        writer.write_all(segment)?;
    }

    Ok(())
}

fn tac_compile_regex(settings: &TacFlags) -> Result<Option<GnuRegex>, TacError> {
    settings
        .is_regex
        .then(|| GnuRegex::compile(&settings.separator, GnuRegexCompileOptions::emacs()))
        .transpose()
        .map_err(TacError::from)
}

fn tac_collect_file_segments_with_regex(
    filename: &OsStr,
    settings: &TacFlags,
    pattern: Option<&mut GnuRegex>,
) -> CTResult<Vec<Vec<u8>>> {
    if let Some(pattern) = pattern {
        let data = get_file_data(filename)?;
        Ok(tac_collect_regex_segments(
            data.as_ref(),
            pattern,
            settings.is_before,
        )?)
    } else {
        let data = get_file_data(filename)?;
        Ok(tac_collect_string_segments(
            data.as_ref(),
            settings.is_before,
            &settings.separator,
        ))
    }
}

#[cfg(test)]
fn tac_collect_file_segments(filename: &OsStr, settings: &TacFlags) -> CTResult<Vec<Vec<u8>>> {
    let mut pattern = tac_compile_regex(settings)?;
    tac_collect_file_segments_with_regex(filename, settings, pattern.as_mut())
}

/// 处理单个文件的 tac 操作
///
/// # 参数
/// * `writer` - 输出目标
/// * `filename` - 要处理的文件名
/// * `settings` - tac 操作的配置
///
/// # 返回值
/// 返回 `CTResult<()>`，表示处理结果
#[cfg(test)]
fn tac_process_file<W: Write>(
    writer: &mut W,
    filename: &OsStr,
    settings: &TacFlags,
) -> CTResult<()> {
    let segments = tac_collect_file_segments(filename, settings)?;
    tac_write_segments(writer, &segments).map_err(TacError::WriteError)?;

    Ok(())
}

/// 执行 tac 操作的主函数
///
/// # 参数
/// * `writer` - 输出目标
/// * `settings` - tac 操作的配置
///
/// # 返回值
/// 返回 `CTResult<()>`，表示执行结果
fn tac<W: Write>(writer: &mut W, settings: &TacFlags) -> CTResult<()> {
    let mut has_error = false;
    let mut read_stdin = false;
    let mut pattern = tac_compile_regex(settings)?;

    for filename in &settings.files {
        read_stdin |= filename.as_bytes() == b"-";
        match tac_collect_file_segments_with_regex(filename, settings, pattern.as_mut()) {
            Ok(segments) => {
                tac_write_segments(writer, &segments).map_err(TacError::WriteError)?;
            }
            Err(error) => {
                ctcore::ct_show_error!("{}", error);
                has_error = true;
            }
        }
    }

    if read_stdin && ctcore::ct_stdin_was_closed() {
        ctcore::ct_show_error!("-: Bad file descriptor");
    }

    writer.flush().map_err(TacError::FlushError)?;

    if has_error {
        // 使用 CtSimpleError 返回一个通用的非零退出码
        return Err(ctcore::ct_error::CtSimpleError::new(1, String::new()));
    }

    Ok(())
}

pub fn tac_native_semantic(args: impl ctcore::Args) -> CTResult<TacSemantic> {
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
    }
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let settings = tac_parse_invocation(args)?;
    let mut rows = Vec::new();
    let mut classic_bytes = Vec::new();
    let mut stderr_text = String::new();
    let mut global_row_index = 1_usize;
    let mut exit_code = 0;
    let mut pattern = match tac_compile_regex(&settings) {
        Ok(pattern) => pattern,
        Err(error) => {
            return Ok(TacSemantic {
                separator_kind: tac_separator_kind(&settings).into(),
                separator_text: tac_lossy_string(&settings.separator),
                before: settings.is_before,
                rows,
                classic_text: String::new(),
                stderr_text: format!("tac: {error}\n"),
                exit_code: 1,
            });
        }
    };

    for (file_index, filename) in settings.files.iter().enumerate() {
        match tac_collect_file_segments_with_regex(filename, &settings, pattern.as_mut()) {
            Ok(segments) => {
                for segment in segments {
                    classic_bytes.extend_from_slice(&segment);
                    if segment.is_empty() {
                        continue;
                    }
                    rows.push(TacRow {
                        source_name: filename.to_string_lossy().into_owned(),
                        file_index: file_index + 1,
                        row_index: global_row_index,
                        chunk: tac_lossy_string(&segment),
                        byte_len: segment.len(),
                    });
                    global_row_index += 1;
                }
            }
            Err(err) => {
                stderr_text.push_str(&format!("tac: {err}\n"));
                exit_code = 1;
            }
        }
    }

    Ok(TacSemantic {
        separator_kind: tac_separator_kind(&settings).into(),
        separator_text: tac_lossy_string(&settings.separator),
        before: settings.is_before,
        rows,
        classic_text: tac_lossy_string(&classic_bytes),
        stderr_text,
        exit_code,
    })
}

/// 尝试对标准输入进行内存映射
///
/// # 返回值
/// 返回 `Option<Mmap>`，成功时返回内存映射对象，失败时返回 None
fn tac_try_mmap_stdin() -> Option<Mmap> {
    // SAFETY: 如果在映射文件时文件被截断，将会引发 SIGBUS 信号并终止我们的进程，从而防止访问无效内存。
    unsafe { Mmap::map(&stdin()).ok() }
}

/// 尝试对已打开的文件进行内存映射。
fn tac_try_mmap_file(file: &File) -> Option<Mmap> {
    // SAFETY: 如果在映射文件时文件被截断，将会引发 SIGBUS 信号并终止我们的进程，从而防止访问无效内存。
    let mmap = unsafe { Mmap::map(file).ok()? };

    Some(mmap)
}

#[derive(Default)]
pub struct Tac;
impl Tool for Tac {
    fn name(&self) -> &'static str {
        "tac"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let mut stdout = stdout().lock();
        tac_main(&mut stdout, args.iter().cloned())
    }
}

#[cfg(test)]
#[allow(clippy::useless_vec)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    mod sigpipe_tests {
        use super::*;

        #[test]
        fn test_sigpipe_is_ignored_in_status_detects_sigpipe_bit() {
            assert!(sigpipe_is_ignored_in_status(
                "Name:\ttest\nSigIgn:\t0000000000001000\n"
            ));
        }

        #[test]
        fn test_sigpipe_is_ignored_in_status_rejects_clear_or_invalid_mask() {
            assert!(!sigpipe_is_ignored_in_status(
                "Name:\ttest\nSigIgn:\t0000000000000000\n"
            ));
            assert!(!sigpipe_is_ignored_in_status(
                "Name:\ttest\nSigIgn:\tinvalid\n"
            ));
            assert!(!sigpipe_is_ignored_in_status("Name:\ttest\n"));
        }
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Tac;

        // 测试 name 方法
        assert_eq!(tool.name(), "tac");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("tac"));

        // 测试 execute 方法 - 帮助命令应该返回错误，但不会崩溃
        let args = vec![OsString::from("tac"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod tac_flags_tests {
        use super::*;

        #[test]
        fn test_tac_flags_default() {
            let flags = TacFlags::default();
            assert!(!flags.is_before);
            assert!(!flags.is_regex);
            assert_eq!(flags.separator, b"\n");
            assert_eq!(flags.files, vec!["-"]);
        }

        #[test]
        fn test_tac_flags_new_with_defaults() {
            let app = ct_app();
            let matches = app.try_get_matches_from(vec!["tac"]).unwrap();
            let flags = TacFlags::new(&matches).unwrap();
            assert!(!flags.is_before);
            assert!(!flags.is_regex);
            assert_eq!(flags.separator, b"\n");
            assert_eq!(flags.files, vec!["-"]);
        }

        #[test]
        fn test_tac_flags_new_with_before() {
            let app = ct_app();
            let matches = app.try_get_matches_from(vec!["tac", "--before"]).unwrap();
            let flags = TacFlags::new(&matches).unwrap();
            assert!(flags.is_before);
        }

        #[test]
        fn test_tac_flags_new_with_regex() {
            let app = ct_app();
            let matches = app.try_get_matches_from(vec!["tac", "--regex"]).unwrap();
            let flags = TacFlags::new(&matches).unwrap();
            assert!(flags.is_regex);
        }

        #[test]
        fn test_tac_flags_new_with_separator() {
            let app = ct_app();
            let matches = app
                .try_get_matches_from(vec!["tac", "--separator", ":"])
                .unwrap();
            let flags = TacFlags::new(&matches).unwrap();
            assert_eq!(flags.separator, b":");
        }

        #[test]
        fn test_tac_flags_accept_repeated_options_and_use_last_separator() {
            let matches = ct_app()
                .try_get_matches_from([
                    "tac",
                    "-b",
                    "--before",
                    "-r",
                    "--regex",
                    "-s",
                    ":",
                    "--separator",
                    ",",
                ])
                .unwrap();
            let flags = TacFlags::new(&matches).unwrap();

            assert!(flags.is_before);
            assert!(flags.is_regex);
            assert_eq!(flags.separator, b",");
        }

        #[test]
        fn test_tac_flags_new_with_empty_separator() {
            let app = ct_app();
            let matches = app
                .try_get_matches_from(vec!["tac", "--separator", ""])
                .unwrap();
            let flags = TacFlags::new(&matches).unwrap();
            assert_eq!(flags.separator, b"\0");
        }

        #[test]
        fn test_tac_flags_reject_empty_regex_separator() {
            let matches = ct_app()
                .try_get_matches_from(["tac", "--regex", "--separator", ""])
                .unwrap();

            let error = TacFlags::new(&matches).unwrap_err();

            assert_eq!(error.to_string(), "separator cannot be empty");
        }

        #[test]
        fn test_tac_flags_new_with_files() {
            let app = ct_app();
            let matches = app
                .try_get_matches_from(vec!["tac", "file1.txt", "file2.txt"])
                .unwrap();
            let flags = TacFlags::new(&matches).unwrap();
            assert_eq!(flags.files, vec!["file1.txt", "file2.txt"]);
        }

        #[test]
        fn test_posixly_correct_stops_option_parsing_at_first_operand() {
            let prepared = prepare_tac_args_with_mode(
                ["tac", "-s", ":", "input", "-b"]
                    .into_iter()
                    .map(OsString::from),
                true,
            );

            assert_eq!(
                prepared,
                ["tac", "-s", ":", "--", "input", "-b"]
                    .into_iter()
                    .map(OsString::from)
                    .collect::<Vec<_>>()
            );
        }

        #[test]
        fn test_posixly_correct_recognizes_attached_separator_value() {
            let prepared = prepare_tac_args_with_mode(
                ["tac", "-brs:", "input", "--regex"]
                    .into_iter()
                    .map(OsString::from),
                true,
            );

            assert_eq!(
                prepared,
                ["tac", "-brs:", "--", "input", "--regex"]
                    .into_iter()
                    .map(OsString::from)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use super::*;
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let command = ct_app();
            let args = vec!["tac", "--version"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let command = ct_app();
            let args = vec!["tac", "--help"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_invalid_flag() {
            let command = ct_app();
            let args = vec!["tac", "--invalid-flag"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_before_flag() {
            let command = ct_app();
            let args = vec!["tac", "--before"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(tac_flags::TAC_BEFORE));
        }

        #[test]
        fn test_ct_app_regex_flag() {
            let command = ct_app();
            let args = vec!["tac", "--regex"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(tac_flags::TAC_REGEX));
        }

        #[test]
        fn test_ct_app_separator_flag() {
            let command = ct_app();
            let args = vec!["tac", "--separator", ":"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<OsString>(tac_flags::TAC_SEPARATOR)
                    .map(OsString::as_os_str),
                Some(OsStr::new(":"))
            );
        }
    }

    #[cfg(test)]
    mod tac_error_tests {
        use super::*;

        #[test]
        fn test_tac_error_uses_simplified_chinese_runtime_messages() {
            let open_error = TacError::OpenError(
                OsString::from("missing"),
                std::io::Error::from_raw_os_error(ctcore::libc::ENOENT),
            );
            let read_error = TacError::ReadError(
                "input".to_string(),
                std::io::Error::from_raw_os_error(ctcore::libc::EIO),
            );
            let flush_error =
                TacError::FlushError(std::io::Error::from_raw_os_error(ctcore::libc::ENOSPC));

            assert_eq!(
                tac_error_message(&open_error, "zh-CN"),
                "以读模式打开 'missing' 时失败: No such file or directory"
            );
            assert_eq!(
                tac_error_message(&read_error, "zh-CN"),
                "input: 读取错误: Input/output error"
            );
            assert_eq!(
                tac_error_message(&flush_error, "zh-CN"),
                "写入错误: No space left on device"
            );
            assert_eq!(
                tac_error_message(&TacError::EmptyRegexSeparator, "zh-CN"),
                "分隔符不能为空"
            );
            assert_eq!(
                tac_error_message(
                    &TacError::StdinReadError(std::io::Error::from_raw_os_error(
                        ctcore::libc::EBADF,
                    )),
                    "zh-CN"
                ),
                "标准输入: 读取错误: Bad file descriptor"
            );
        }

        #[test]
        fn test_tac_error_invalid_regex() {
            let error = TacError::InvalidRegex("Invalid regular expression".to_string());
            assert_eq!(error.to_string(), "Invalid regular expression");
            assert_eq!(error.code(), 1);
        }

        #[test]
        fn test_tac_error_invalid_argument() {
            let error = TacError::InvalidArgument(OsString::from("test.txt"));
            assert_eq!(error.to_string(), "test.txt: read error: Invalid argument");
            assert_eq!(error.code(), 1);
        }

        #[test]
        fn test_tac_error_file_not_found() {
            let error = TacError::OpenError(
                OsString::from("test.txt"),
                std::io::Error::from_raw_os_error(ctcore::libc::ENOENT),
            );
            assert_eq!(
                error.to_string(),
                "failed to open 'test.txt' for reading: No such file or directory"
            );
            assert_eq!(error.code(), 1);
        }

        #[test]
        fn test_tac_error_read_error() {
            let io_error = std::io::Error::other("read error");
            let error = TacError::ReadError("test.txt".to_string(), io_error);
            assert_eq!(error.to_string(), "test.txt: read error: read error");
            assert_eq!(error.code(), 1);
        }

        #[test]
        fn test_stdin_directory_read_error_uses_invalid_argument() {
            let error =
                tac_stdin_read_error(std::io::Error::from_raw_os_error(ctcore::libc::EISDIR));

            assert_eq!(
                error.to_string(),
                "'standard input': read error: Invalid argument"
            );
        }

        #[test]
        fn test_stdin_read_error_preserves_other_errno() {
            let error =
                tac_stdin_read_error(std::io::Error::from_raw_os_error(ctcore::libc::EBADF));

            assert_eq!(
                error.to_string(),
                "'standard input': read error: Bad file descriptor"
            );
        }

        #[test]
        fn test_tac_error_write_error() {
            let io_error = std::io::Error::other("write error");
            let error = TacError::WriteError(io_error);
            assert_eq!(error.to_string(), "write error");
            assert_eq!(error.code(), 1);
        }
    }

    #[cfg(test)]
    mod file_operations_tests {
        use super::*;
        use std::ffi::CString;
        use std::fs;
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::os::unix::ffi::OsStringExt;
        use std::os::unix::fs::symlink;
        use std::thread;
        use std::time::Duration;
        use tempfile::NamedTempFile;
        use tempfile::tempdir;

        #[test]
        fn test_read_from_file_success() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"test content").unwrap();
            let file = File::open(temp_file.path()).unwrap();
            let result = read_from_file(file, temp_file.path());
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), b"test content");
        }

        #[test]
        fn test_open_file_nonexistent() {
            let result = open_file(Path::new("nonexistent.txt"));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("failed to open"));
        }

        #[test]
        fn test_open_file_directory() {
            let result = open_file(Path::new("."));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("Invalid argument"));
        }

        #[test]
        fn test_open_file_reports_nonexistent() {
            let result = open_file(Path::new("nonexistent.txt"));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("No such file or directory"));
        }

        #[test]
        fn test_open_file_preserves_not_a_directory_error() {
            let directory = tempdir().unwrap();
            let parent = directory.path().join("regular");
            fs::write(&parent, b"data").unwrap();
            let path = parent.join("child");

            let error = open_file(&path).unwrap_err();

            assert!(error.to_string().ends_with("Not a directory"));
        }

        #[test]
        fn test_open_file_preserves_symlink_loop_error() {
            let directory = tempdir().unwrap();
            let path = directory.path().join("loop");
            symlink("loop", &path).unwrap();

            let error = open_file(&path).unwrap_err();

            assert!(
                error
                    .to_string()
                    .ends_with("Too many levels of symbolic links")
            );
        }

        #[test]
        fn test_open_file_quotes_non_utf8_bytes() {
            let directory = tempdir().unwrap();
            let path = directory.path().join(OsString::from_vec(vec![0xff]));

            let error = open_file(&path).unwrap_err();
            let diagnostic = error.to_string();

            assert!(diagnostic.contains("\\377"), "{diagnostic}");
            assert!(!diagnostic.contains('\u{fffd}'), "{diagnostic}");
        }

        #[test]
        fn test_open_file_valid() {
            let temp_file = NamedTempFile::new().unwrap();
            let result = open_file(temp_file.path());
            assert!(result.is_ok());
        }

        #[test]
        #[ignore]
        fn test_get_file_data_stdin() {
            // This test is ignored because it requires real stdin
            // In a real environment, we would need integration tests
            // or a more sophisticated mock setup
            let result = get_file_data(OsStr::new("-"));
            assert!(result.is_ok());
        }

        #[test]
        fn test_get_file_data_regular_file() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"test content").unwrap();
            let result = get_file_data(temp_file.path().as_os_str());
            assert!(result.is_ok());
            match result.unwrap() {
                FileData::Mapped(_) | FileData::Buffer(_) => (),
            }
        }

        #[test]
        fn test_get_file_data_nonexistent() {
            let result = get_file_data(OsStr::new("nonexistent.txt"));
            assert!(result.is_err());
        }

        #[test]
        fn test_get_file_data_opens_fifo_once() {
            let directory = tempdir().unwrap();
            let path = directory.path().join("input.fifo");
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { ctcore::libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);

            // Keep one writer present so both the broken double-open path and the
            // corrected single-open path can finish without hanging the test.
            let keepalive = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            let inotify_fd = unsafe {
                ctcore::libc::inotify_init1(ctcore::libc::IN_NONBLOCK | ctcore::libc::IN_CLOEXEC)
            };
            assert!(inotify_fd >= 0);
            assert!(
                unsafe {
                    ctcore::libc::inotify_add_watch(
                        inotify_fd,
                        c_path.as_ptr(),
                        ctcore::libc::IN_OPEN | ctcore::libc::IN_CLOSE_NOWRITE,
                    )
                } >= 0
            );

            let closer = thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                drop(keepalive);
            });
            let data = get_file_data(path.as_os_str()).unwrap();
            closer.join().unwrap();
            assert!(data.as_ref().is_empty());

            let mut events = [0_u8; 4096];
            let bytes_read =
                unsafe { ctcore::libc::read(inotify_fd, events.as_mut_ptr().cast(), events.len()) };
            unsafe {
                ctcore::libc::close(inotify_fd);
            }
            assert!(bytes_read > 0);

            let mut offset = 0_usize;
            let mut open_count = 0_usize;
            while offset < bytes_read as usize {
                let event = unsafe {
                    &*events
                        .as_ptr()
                        .add(offset)
                        .cast::<ctcore::libc::inotify_event>()
                };
                if event.mask & ctcore::libc::IN_OPEN != 0 {
                    open_count += 1;
                }
                offset += std::mem::size_of::<ctcore::libc::inotify_event>() + event.len as usize;
            }

            assert_eq!(open_count, 1, "a FIFO operand must be opened only once");
        }
    }

    #[cfg(test)]
    mod tac_buffer_tests {
        use super::*;

        #[test]
        fn test_tac_buffer_simple() {
            let mut output = Vec::new();
            let data = b"line1\nline2\nline3";
            tac_buffer(&mut output, data, false, b"\n").unwrap();
            assert_eq!(output, b"line3line2\nline1\n");
        }

        #[test]
        fn test_tac_buffer_before() {
            let mut output = Vec::new();
            let data = b"line1\nline2\nline3";
            tac_buffer(&mut output, data, true, b"\n").unwrap();
            assert_eq!(output, b"\nline3\nline2line1");
        }

        #[test]
        fn test_tac_buffer_custom_separator() {
            let mut output = Vec::new();
            let data = b"line1:line2:line3";
            tac_buffer(&mut output, data, false, b":").unwrap();
            assert_eq!(output, b"line3line2:line1:");
        }

        #[test]
        fn test_tac_buffer_empty_input() {
            let mut output = Vec::new();
            let data = b"";
            tac_buffer(&mut output, data, false, b"\n").unwrap();
            assert_eq!(output, b"");
        }

        #[test]
        fn test_tac_buffer_single_line() {
            let mut output = Vec::new();
            let data = b"single line";
            tac_buffer(&mut output, data, false, b"\n").unwrap();
            assert_eq!(output, b"single line");
        }

        #[test]
        fn test_tac_buffer_with_trailing_separator() {
            let mut output = Vec::new();
            let data = b"line1\nline2\nline3\n";
            tac_buffer(&mut output, data, false, b"\n").unwrap();
            assert_eq!(output, b"line3\nline2\nline1\n");
        }

        #[test]
        fn test_tac_buffer_with_multiple_separators() {
            let mut output = Vec::new();
            let data = b"line1\n\nline2\n\nline3";
            tac_buffer(&mut output, data, false, b"\n").unwrap();
            assert_eq!(output, b"line3\nline2\n\nline1\n");
        }

        #[test]
        fn test_tac_buffer_with_empty_lines() {
            let mut output = Vec::new();
            let data = b"\n\n\n";
            tac_buffer(&mut output, data, false, b"\n").unwrap();
            assert_eq!(output, b"\n\n\n");
        }

        #[test]
        fn test_tac_buffer_with_custom_multi_byte_separator() {
            let mut output = Vec::new();
            let data = b"line1<sep>line2<sep>line3";
            tac_buffer(&mut output, data, false, b"<sep>").unwrap();
            assert_eq!(output, b"line3line2<sep>line1<sep>");
        }

        #[test]
        fn test_tac_buffer_with_no_separator() {
            let mut output = Vec::new();
            let data = b"content";
            tac_buffer(&mut output, data, false, b"|").unwrap();
            assert_eq!(output, b"content");
        }
    }

    #[cfg(test)]
    mod tac_buffer_regex_tests {
        use super::*;

        fn reverse(data: &[u8], pattern: &[u8], before: bool) -> Vec<u8> {
            let mut output = Vec::new();
            let mut pattern = GnuRegex::compile(pattern, GnuRegexCompileOptions::emacs()).unwrap();
            tac_buffer_regex(&mut output, data, &mut pattern, before).unwrap();
            output
        }

        #[test]
        fn test_tac_buffer_regex_simple() {
            assert_eq!(
                reverse(b"line1\nline2\nline3", b"\n", false),
                b"line3line2\nline1\n"
            );
        }

        #[test]
        fn test_tac_buffer_regex_before() {
            assert_eq!(
                reverse(b"line1\nline2\nline3", b"\n", true),
                b"\nline3\nline2line1"
            );
        }

        #[test]
        fn test_tac_buffer_regex_complex_pattern() {
            assert_eq!(
                reverse(b"line1\r\nline2\nline3\r\n", b"\r?\n", false),
                b"line3\r\nline2\nline1\r\n"
            );
        }

        #[test]
        fn test_tac_buffer_regex_supports_emacs_backreferences() {
            assert_eq!(reverse(b"ababXcdcd", b"\\(..\\)\\1", false), b"Xcdcdabab");
        }

        #[test]
        fn test_tac_buffer_regex_treats_plain_parentheses_as_literals() {
            assert_eq!(reverse(b"aa(xx)bb(xx)", b"(xx)", false), b"bb(xx)aa(xx)");
        }

        #[test]
        fn test_tac_buffer_regex_with_overlapping_matches() {
            assert_eq!(reverse(b"aaaa", b"aa", false), b"aaaa");
        }

        #[test]
        fn test_tac_buffer_regex_variable_length_separator() {
            assert_eq!(reverse(b"aa::bbb::c::", b":+", false), b":c::bbb::aa:");
        }

        #[test]
        fn test_tac_buffer_regex_variable_length_separator_before() {
            assert_eq!(reverse(b"aa::bbb::c::", b":+", true), b":::c::bbb:aa");
        }

        #[test]
        fn test_tac_buffer_regex_handles_zero_length_anchors() {
            assert_eq!(reverse(b"ab\ncd\n", b"^", false), b"cd\nab\n");
            assert_eq!(reverse(b"ab\ncd\n", b"$", false), b"\n\ncdab");
        }

        #[test]
        fn test_tac_buffer_regex_c_locale_dot_matches_one_byte() {
            assert_eq!(
                reverse(b"A\xc3\xa9B\xc3\xa9C", b".", false),
                b"C\xa9\xc3B\xa9\xc3A"
            );
        }

        #[test]
        fn test_tac_buffer_regex_accepts_non_utf8_pattern() {
            assert_eq!(reverse(b"x\xffy\xff", b"\xff", false), b"y\xffx\xff");
        }
    }

    #[cfg(test)]
    mod tac_process_file_tests {
        use super::*;
        use std::io::Write;
        use tempfile::NamedTempFile;

        fn create_temp_file_with_content(content: &[u8]) -> NamedTempFile {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(content).unwrap();
            temp_file
        }

        #[test]
        fn test_tac_process_file_simple() {
            let temp_file = create_temp_file_with_content(b"line1\nline2\nline3\n");
            let mut output = Vec::new();
            let settings = TacFlags::default();
            tac_process_file(&mut output, temp_file.path().as_os_str(), &settings).unwrap();
            assert_eq!(output, b"line3\nline2\nline1\n");
        }

        #[test]
        fn test_tac_process_file_before() {
            let temp_file = create_temp_file_with_content(b"line1\nline2\nline3");
            let mut output = Vec::new();
            let settings = TacFlags {
                is_before: true,
                ..Default::default()
            };
            tac_process_file(&mut output, temp_file.path().as_os_str(), &settings).unwrap();
            assert_eq!(output, b"\nline3\nline2line1");
        }

        #[test]
        fn test_tac_process_file_with_empty_file() {
            let temp_file = NamedTempFile::new().unwrap();
            let mut output = Vec::new();
            let settings = TacFlags::default();
            tac_process_file(&mut output, temp_file.path().as_os_str(), &settings).unwrap();
            assert_eq!(output, b"");
        }

        #[test]
        fn test_tac_process_file_with_invalid_regex() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"test content").unwrap();
            let mut output = Vec::new();
            let settings = TacFlags {
                is_regex: true,
                separator: b"[".to_vec(), // Invalid regex
                ..Default::default()
            };
            let result = tac_process_file(&mut output, temp_file.path().as_os_str(), &settings);
            assert!(result.is_err());
        }

        #[test]
        fn test_tac_process_file_with_custom_separator_and_before() {
            let temp_file = create_temp_file_with_content(b"1,2,3");
            let mut output = Vec::new();
            let settings = TacFlags {
                is_before: true,
                separator: b",".to_vec(),
                ..Default::default()
            };
            tac_process_file(&mut output, temp_file.path().as_os_str(), &settings).unwrap();
            assert_eq!(output, b",3,21");
        }

        #[test]
        fn test_tac_process_file_with_binary_content() {
            let content = vec![0u8, 1u8, 2u8, 255u8];
            let temp_file = create_temp_file_with_content(&content);
            let mut output = Vec::new();
            let settings = TacFlags::default();
            tac_process_file(&mut output, temp_file.path().as_os_str(), &settings).unwrap();
            assert_eq!(output, content);
        }
    }

    #[cfg(test)]
    mod tac_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::fs;
        use std::io;
        use std::os::unix::ffi::OsStringExt;
        use tempfile::{NamedTempFile, tempdir};

        #[derive(Default)]
        struct FlushFailWriter {
            output: Vec<u8>,
            flush_count: usize,
        }

        impl Write for FlushFailWriter {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                self.output.extend_from_slice(buffer);
                Ok(buffer.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.flush_count += 1;
                Err(io::Error::from_raw_os_error(ctcore::libc::ENOSPC))
            }
        }

        #[derive(Default)]
        struct WriteFailWriter {
            write_count: usize,
        }

        impl Write for WriteFailWriter {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                self.write_count += 1;
                Err(io::Error::from_raw_os_error(ctcore::libc::ENOSPC))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        #[test]
        fn test_tac_main_simple() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"line1\nline2\nline3\n").unwrap();

            let args = [
                "tac".to_string(),
                temp_file.path().to_str().unwrap().to_string(),
            ];
            let mut output = Vec::new();
            let result = tac_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            assert_eq!(output, b"line3\nline2\nline1\n");
        }

        #[test]
        fn test_tac_main_multiple_files() {
            let mut file1 = NamedTempFile::new().unwrap();
            let mut file2 = NamedTempFile::new().unwrap();
            file1.write_all(b"1\n2\n").unwrap();
            file2.write_all(b"a\nb\n").unwrap();

            let args = [
                "tac".to_string(),
                file1.path().to_str().unwrap().to_string(),
                file2.path().to_str().unwrap().to_string(),
            ];
            let mut output = Vec::new();
            let result = tac_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            assert_eq!(output, b"2\n1\nb\na\n");
        }

        #[test]
        fn test_tac_main_reports_final_flush_error() {
            let mut input = NamedTempFile::new().unwrap();
            input.write_all(b"a").unwrap();
            let args = [
                OsString::from("tac"),
                input.path().as_os_str().to_os_string(),
            ];
            let mut output = FlushFailWriter::default();

            let error = tac_main(&mut output, args.into_iter()).unwrap_err();

            assert_eq!(output.output, b"a");
            assert_eq!(output.flush_count, 1);
            assert_eq!(error.to_string(), "write error: No space left on device");
        }

        #[test]
        fn test_tac_main_stops_after_first_write_error() {
            let mut first = NamedTempFile::new().unwrap();
            let mut second = NamedTempFile::new().unwrap();
            first.write_all(b"first").unwrap();
            second.write_all(b"second").unwrap();
            let args = [
                OsString::from("tac"),
                first.path().as_os_str().to_os_string(),
                second.path().as_os_str().to_os_string(),
            ];
            let mut output = WriteFailWriter::default();

            let error = tac_main(&mut output, args.into_iter()).unwrap_err();

            assert_eq!(output.write_count, 1);
            assert_eq!(error.to_string(), "write error");
        }

        #[test]
        fn test_tac_main_accepts_non_utf8_filename() {
            let directory = tempdir().unwrap();
            let path = directory.path().join(OsString::from_vec(vec![0xff]));
            fs::write(&path, b"first\nsecond\n").unwrap();
            let args = [OsString::from("tac"), path.into_os_string()];
            let mut output = Vec::new();

            tac_main(&mut output, args.into_iter()).unwrap();

            assert_eq!(output, b"second\nfirst\n");
        }

        #[test]
        fn test_tac_main_accepts_non_utf8_fixed_separator() {
            let mut input = NamedTempFile::new().unwrap();
            input.write_all(b"x\xffy\xff").unwrap();
            let args = [
                OsString::from("tac"),
                OsString::from("-s"),
                OsString::from_vec(vec![0xff]),
                input.path().as_os_str().to_os_string(),
            ];
            let mut output = Vec::new();

            tac_main(&mut output, args.into_iter()).unwrap();

            assert_eq!(output, b"y\xffx\xff");
        }

        #[test]
        fn test_tac_main_with_all_options() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"1::2::3").unwrap();

            let args = [
                "tac".to_string(),
                "--before".to_string(),
                "--regex".to_string(),
                "--separator".to_string(),
                "::".to_string(),
                temp_file.path().to_str().unwrap().to_string(),
            ];
            let mut output = Vec::new();
            let result = tac_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            // before 模式：分隔符属于后面的记录，输出为 "::3::21"
            assert_eq!(output, b"::3::21");
        }

        #[test]
        fn test_tac_main_regex_separator_placement() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"aa::bbb::c::").unwrap();

            let args = [
                "tac".to_string(),
                "--regex".to_string(),
                "--separator".to_string(),
                ":+".to_string(),
                temp_file.path().to_str().unwrap().to_string(),
            ];
            let mut output = Vec::new();
            let result = tac_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            assert_eq!(output, b":c::bbb::aa:");
        }

        #[test]
        fn test_tac_main_with_invalid_regex_pattern() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"test").unwrap();

            let args = vec![
                "tac".to_string(),
                "--regex".to_string(),
                "--separator".to_string(),
                "[".to_string(), // Invalid regex
                temp_file.path().to_str().unwrap().to_string(),
            ];
            let mut output = Vec::new();
            let result = tac_main(&mut output, args.iter().map(OsString::from));
            // 无效的正则表达式会导致错误返回
            assert!(result.is_err());
        }

        #[test]
        fn test_tac_main_with_empty_separator() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"abc").unwrap();

            let args = [
                "tac".to_string(),
                "--separator".to_string(),
                "".to_string(),
                temp_file.path().to_str().unwrap().to_string(),
            ];
            let mut output = Vec::new();
            let result = tac_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            assert_eq!(output, b"abc");
        }

        #[test]
        fn test_tac_main_rejects_empty_regex_separator_before_opening_files() {
            let args = ["tac", "-r", "-s", "", "missing"]
                .into_iter()
                .map(OsString::from);
            let mut output = Vec::new();

            let error = tac_main(&mut output, args).unwrap_err();

            assert_eq!(error.to_string(), "separator cannot be empty");
            assert!(output.is_empty());
        }
    }

    #[cfg(test)]
    mod semantic_tests {
        use super::*;
        use tempfile::NamedTempFile;

        #[test]
        fn test_tac_native_semantic_collects_rows_and_classic_text() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"alpha\nbeta\ngamma\n").unwrap();

            let args = [
                OsString::from("tac"),
                temp_file.path().as_os_str().to_os_string(),
            ];

            let semantic = tac_native_semantic(args.into_iter()).expect("semantic");

            assert_eq!(semantic.separator_kind, "string");
            assert_eq!(semantic.separator_text, "\n");
            assert!(!semantic.before);
            assert_eq!(semantic.exit_code, 0);
            assert!(semantic.stderr_text.is_empty());
            assert_eq!(semantic.classic_text, "gamma\nbeta\nalpha\n");
            assert_eq!(semantic.rows.len(), 3);
            assert_eq!(semantic.rows[0].row_index, 1);
            assert_eq!(semantic.rows[0].chunk, "gamma\n");
            assert_eq!(semantic.rows[0].byte_len, 6);
        }

        #[test]
        fn test_tac_native_semantic_missing_file_reports_runtime_error() {
            let args = [OsString::from("tac"), OsString::from("missing-file.txt")];

            let semantic = tac_native_semantic(args.into_iter()).expect("semantic");

            assert_eq!(semantic.exit_code, 1);
            assert!(semantic.classic_text.is_empty());
            assert!(semantic.rows.is_empty());
            assert!(
                semantic
                    .stderr_text
                    .contains("tac: failed to open 'missing-file.txt' for reading")
            );
        }

        #[test]
        fn test_tac_native_semantic_reports_invalid_regex_once_for_multiple_files() {
            let args = [
                OsString::from("tac"),
                OsString::from("-r"),
                OsString::from("-s"),
                OsString::from("["),
                OsString::from("missing-one"),
                OsString::from("missing-two"),
            ];

            let semantic = tac_native_semantic(args.into_iter()).expect("semantic");

            assert_eq!(semantic.exit_code, 1);
            assert!(semantic.classic_text.is_empty());
            assert_eq!(semantic.stderr_text, "tac: Invalid regular expression\n");
        }
    }
}
