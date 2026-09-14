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
use ctcore::ct_gnu_regex::{GnuRegex, GnuRegexCompileOptions, GnuRegexError, GnuRegexMatch};
use ctcore::ct_posix::GnuGetoptCommandExt;
use ctcore::ct_quoting_style::escape_shell_bytes_with_classifier;
use memchr::memmem;
use memmap2::Mmap;
use std::borrow::Cow;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::io::{Read, Seek, SeekFrom, Write, stdin, stdout};
use std::mem::MaybeUninit;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::{fs::File, fs::OpenOptions, path::Path, path::PathBuf};
use tempfile::{Builder, NamedTempFile};

const GNU_TAC_READ_SIZE: usize = 8192;

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

    /// 无法创建用于缓存非可定位输入的临时文件。
    TemporaryFileCreate(OsString, std::io::Error),

    /// 写入用于缓存非可定位输入的临时文件时出错。
    TemporaryFileWrite(OsString, std::io::Error),

    /// 重置用于缓存后续非可定位输入的临时文件时出错。
    TemporaryFileRewind(OsString, std::io::Error),
}

impl CTError for TacError {
    fn code(&self) -> i32 {
        1
    }
}

#[derive(Debug)]
struct TacUsageError {
    message: Vec<u8>,
    usage_hint: Vec<u8>,
}

impl TacUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        let usage_hint = if rust_i18n::locale() == "zh-CN" {
            r#"请尝试执行 "tac --help" 来获取更多信息。"#.as_bytes().to_vec()
        } else {
            b"Try 'tac --help' for more information.".to_vec()
        };
        Box::new(Self {
            message,
            usage_hint,
        })
    }
}

impl Display for TacUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for TacUsageError {}

impl CTError for TacUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage_hint_bytes(&self) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Borrowed(&self.usage_hint))
    }

    fn usage(&self) -> bool {
        true
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
        TacError::TemporaryFileCreate(path, error) => format!(
            "{} {}: {}",
            t!("tac.errors.temporary_file_create", locale = locale),
            tac_quote_path(path, true),
            strip_errno(error)
        ),
        TacError::TemporaryFileWrite(path, error) => format!(
            "{}: {}: {}",
            tac_quote_path(path, false),
            t!("tac.errors.write_error", locale = locale),
            strip_errno(error)
        ),
        TacError::TemporaryFileRewind(path, error) => format!(
            "{}: {}",
            t!(
                "tac.errors.temporary_file_rewind",
                locale = locale,
                path = tac_quote_path(path, false)
            ),
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
    tac_main_with_stdout_state(writer, args, false)
}

fn tac_main_with_stdout_state<W: Write>(
    writer: &mut W,
    args: impl ctcore::Args,
    stdout_was_closed: bool,
) -> CTResult<()> {
    let _sigpipe_guard = SigpipeGuard::for_cli();
    initialize_tac_locale();
    let settings = tac_parse_invocation(args)?;

    // 使用配置执行主要逻辑
    tac(writer, &settings, stdout_was_closed)
}

fn initialize_tac_locale() {
    let locale_is_valid =
        unsafe { !ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr()).is_null() };
    let locale = if locale_is_valid {
        tac_message_locale_from_values(
            std::env::var("LC_ALL").ok().as_deref(),
            std::env::var("LC_MESSAGES").ok().as_deref(),
            std::env::var("LANG").ok().as_deref(),
            std::env::var("LANGUAGE").ok().as_deref(),
        )
    } else {
        "en-US"
    };
    rust_i18n::set_locale(locale);
}

fn tac_message_locale_from_values(
    lc_all: Option<&str>,
    lc_messages: Option<&str>,
    lang: Option<&str>,
    language: Option<&str>,
) -> &'static str {
    let base = [lc_all, lc_messages, lang]
        .into_iter()
        .flatten()
        .find(|locale| !locale.is_empty())
        .unwrap_or("C");

    if base.eq_ignore_ascii_case("C") || base.eq_ignore_ascii_case("POSIX") {
        return "en-US";
    }

    if let Some(language) = language.filter(|value| !value.is_empty()) {
        return language
            .split(':')
            .find_map(known_tac_message_locale)
            .unwrap_or("en-US");
    }

    known_tac_message_locale(base).unwrap_or("en-US")
}

fn known_tac_message_locale(locale: &str) -> Option<&'static str> {
    let language = locale
        .split(['.', '@'])
        .next()
        .unwrap_or(locale)
        .replace('-', "_")
        .to_ascii_lowercase();

    if language == "c" || language == "posix" || language == "en" || language.starts_with("en_") {
        Some("en-US")
    } else if language == "zh" || language.starts_with("zh_cn") {
        Some("zh-CN")
    } else {
        None
    }
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
fn tac_command() -> Command {
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
            .allow_hyphen_values(true)
            .value_parser(OsStringValueParser::new())
            .value_name("STRING"),
        Arg::new(tac_flags::TAC_FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new("help")
            .long("help")
            .help("display this help and exit")
            .action(ArgAction::Help),
        Arg::new("version")
            .long("version")
            .help("output version information and exit")
            .action(ArgAction::Version),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args_override_self(true)
        .infer_long_args(true)
        .args(&args)
}

pub fn ct_app() -> Command {
    tac_command().gnu_getopt()
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
    File::open(path)
        .map_err(|error| TacError::OpenError(path.as_os_str().to_os_string(), error).into())
}

fn read_from_file(mut file: File, path: &Path) -> CTResult<Vec<u8>> {
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|error| TacError::ReadError(tac_quote_path(path.as_os_str(), false), error))?;

    Ok(buffer)
}

fn read_from_directory(mut file: File, path: &Path) -> CTResult<Vec<u8>> {
    if let Ok(end) = file.seek(SeekFrom::End(0)) {
        let aligned = end - end % GNU_TAC_READ_SIZE as u64;
        if aligned != end {
            let _ = file.seek(SeekFrom::Start(aligned));
        }
    }

    let mut buffer = vec![0; GNU_TAC_READ_SIZE];
    let count = file
        .read(&mut buffer)
        .map_err(|error| TacError::ReadError(tac_quote_path(path.as_os_str(), false), error))?;
    buffer.truncate(count);
    Ok(buffer)
}

fn tac_input_read_error(path: Option<&Path>, error: std::io::Error) -> TacError {
    match path {
        Some(path) => TacError::ReadError(tac_quote_path(path.as_os_str(), false), error),
        None => tac_stdin_read_error(error),
    }
}

fn tac_temporary_directory() -> PathBuf {
    std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn tac_create_temporary_file(directory: &Path) -> Result<NamedTempFile, TacError> {
    let mut attempted_path = directory.join("cutmpXXXXXX");
    Builder::new()
        .prefix("cutmp")
        .make_in(directory, |path| {
            attempted_path = path.to_path_buf();
            OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
        })
        .map_err(|error| TacError::TemporaryFileCreate(attempted_path.into_os_string(), error))
}

fn copy_nonseekable_to_temporary<R: Read>(
    reader: R,
    source: Option<&Path>,
) -> CTResult<(NamedTempFile, u64)> {
    let mut reusable = None;
    let bytes_copied = copy_nonseekable_to_reusable(reader, source, &mut reusable)?;
    Ok((
        reusable.expect("successful temporary copy initializes the stream"),
        bytes_copied,
    ))
}

fn copy_nonseekable_to_reusable<R: Read>(
    mut reader: R,
    source: Option<&Path>,
    reusable: &mut Option<NamedTempFile>,
) -> CTResult<u64> {
    if reusable.is_none() {
        let temporary_directory = tac_temporary_directory();
        let mut temporary = tac_create_temporary_file(&temporary_directory)?;
        // Match GNU temp_stream and avoid removing a different file if this path is later reused.
        if std::fs::remove_file(temporary.path()).is_ok() {
            temporary.disable_cleanup(true);
        }
        *reusable = Some(temporary);
    } else {
        let temporary = reusable
            .as_mut()
            .expect("the reusable stream is initialized");
        let temporary_name = temporary.path().as_os_str().to_os_string();
        temporary
            .as_file_mut()
            .seek(SeekFrom::Start(0))
            .and_then(|_| temporary.as_file_mut().set_len(0))
            .map_err(|error| TacError::TemporaryFileRewind(temporary_name, error))?;
    }

    let temporary = reusable
        .as_mut()
        .expect("the reusable stream is initialized");
    let temporary_name = temporary.path().as_os_str().to_os_string();
    let mut buffer = [0_u8; GNU_TAC_READ_SIZE];
    let mut bytes_copied = 0_u64;

    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| tac_input_read_error(source, error))?;
        if count == 0 {
            break;
        }
        temporary
            .as_file_mut()
            .write_all(&buffer[..count])
            .map_err(|error| TacError::TemporaryFileWrite(temporary_name.clone(), error))?;
        bytes_copied = bytes_copied
            .checked_add(count as u64)
            .ok_or(TacError::RecordTooLarge)?;
    }

    temporary
        .as_file_mut()
        .flush()
        .map_err(|error| TacError::TemporaryFileWrite(temporary_name.clone(), error))?;

    Ok(bytes_copied)
}

fn read_from_nonseekable<R: Read>(reader: R, source: Option<&Path>) -> CTResult<FileData> {
    let (mut temporary, _) = copy_nonseekable_to_temporary(reader, source)?;
    let temporary_name = temporary.path().as_os_str().to_os_string();
    temporary
        .as_file_mut()
        .seek(SeekFrom::Start(0))
        .map_err(|error| TacError::ReadError(tac_quote_path(&temporary_name, false), error))?;

    let file = temporary.into_file();
    if let Some(mmap) = tac_try_mmap_file(&file) {
        Ok(FileData::Mapped(mmap))
    } else {
        read_from_file(file, Path::new(&temporary_name)).map(FileData::Buffer)
    }
}

fn tac_seek_to_start_if_seek_end_supported(fd: ctcore::libc::c_int) -> bool {
    // SAFETY: lseek only updates the descriptor offset and does not dereference pointers.
    unsafe {
        ctcore::libc::lseek(fd, 0, ctcore::libc::SEEK_END) >= 0
            && ctcore::libc::lseek(fd, 0, ctcore::libc::SEEK_SET) >= 0
    }
}

fn tac_regular_file_size(fd: ctcore::libc::c_int) -> Option<u64> {
    let mut metadata = MaybeUninit::<ctcore::libc::stat>::uninit();
    if unsafe { ctcore::libc::fstat(fd, metadata.as_mut_ptr()) } != 0 {
        return None;
    }
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_mode & ctcore::libc::S_IFMT != ctcore::libc::S_IFREG {
        return None;
    }

    let end = unsafe { ctcore::libc::lseek(fd, 0, ctcore::libc::SEEK_END) };
    (end >= 0).then_some(end as u64)
}

fn tac_pread_some(
    fd: ctcore::libc::c_int,
    offset: u64,
    buffer: &mut [u8],
    source: Option<&Path>,
) -> CTResult<usize> {
    loop {
        let count = unsafe {
            ctcore::libc::pread(
                fd,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                offset as ctcore::libc::off_t,
            )
        };
        if count < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(tac_input_read_error(source, error).into());
        }
        return Ok(count as usize);
    }
}

fn tac_pread_exact(
    fd: ctcore::libc::c_int,
    mut offset: u64,
    mut buffer: &mut [u8],
    source: Option<&Path>,
) -> CTResult<()> {
    while !buffer.is_empty() {
        let count = tac_pread_some(fd, offset, buffer, source)?;
        if count == 0 {
            return Err(tac_input_read_error(
                source,
                std::io::Error::from(std::io::ErrorKind::UnexpectedEof),
            )
            .into());
        }

        offset += count as u64;
        buffer = &mut buffer[count..];
    }
    Ok(())
}

fn tac_actual_file_size(
    fd: ctcore::libc::c_int,
    estimated_size: u64,
    read_size: usize,
    source: Option<&Path>,
) -> CTResult<u64> {
    let read_size_u64 = read_size as u64;
    let mut offset = estimated_size - estimated_size % read_size_u64;
    let mut buffer = vec![0_u8; read_size];

    loop {
        let count = tac_pread_some(fd, offset, &mut buffer, source)?;
        if count == 0 && offset != 0 {
            offset = offset.saturating_sub(read_size_u64);
            continue;
        }
        offset += count as u64;
        if count < read_size {
            return Ok(offset);
        }
        break;
    }

    loop {
        let count = tac_pread_some(fd, offset, &mut buffer, source)?;
        offset += count as u64;
        if count < read_size {
            return Ok(offset);
        }
    }
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

fn get_open_file_data(file: File, path: &Path) -> CTResult<FileData> {
    if file.metadata().is_ok_and(|metadata| metadata.is_dir()) {
        return read_from_directory(file, path).map(FileData::Buffer);
    }

    if let Some(mmap) = tac_try_mmap_file(&file) {
        Ok(FileData::Mapped(mmap))
    } else if tac_seek_to_start_if_seek_end_supported(file.as_raw_fd()) {
        read_from_file(file, path).map(FileData::Buffer)
    } else {
        read_from_nonseekable(file, Some(path))
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
        if ctcore::ct_stdin_was_closed() {
            return read_from_stdin().map(FileData::Buffer);
        }
        // 处理标准输入
        if let Some(mmap) = tac_try_mmap_stdin() {
            Ok(FileData::Mapped(mmap))
        } else if tac_seek_to_start_if_seek_end_supported(ctcore::libc::STDIN_FILENO) {
            read_from_stdin().map(FileData::Buffer)
        } else {
            read_from_nonseekable(ctcore::ct_io::stdin_reader_box(), None)
        }
    } else {
        // 处理普通文件
        let path = Path::new(filename);
        let file = open_file(path)?;
        get_open_file_data(file, path)
    }
}

const TAC_LONG_OPTIONS: &[&str] = &["before", "regex", "separator", "help", "version"];
const TAC_SHORT_OPTIONS: &[u8] = b"brs";

enum TacLongOptionMatch {
    None,
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
}

fn tac_match_long_option(name: &[u8]) -> TacLongOptionMatch {
    if let Some(option) = TAC_LONG_OPTIONS
        .iter()
        .find(|option| option.as_bytes() == name)
    {
        return TacLongOptionMatch::Recognized(option);
    }

    let matches = TAC_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|option| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => TacLongOptionMatch::None,
        [option] => TacLongOptionMatch::Recognized(option),
        _ => TacLongOptionMatch::Ambiguous(matches),
    }
}

fn tac_validate_long_option(argument: &[u8], has_next: bool) -> CTResult<bool> {
    let long = &argument[2..];
    let separator = long.iter().position(|byte| *byte == b'=');
    let name = &long[..separator.unwrap_or(long.len())];

    match tac_match_long_option(name) {
        TacLongOptionMatch::None => {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(argument);
            message.push(b'\'');
            Err(TacUsageError::boxed(message))
        }
        TacLongOptionMatch::Ambiguous(matches) => {
            let possibilities = matches
                .into_iter()
                .map(|option| format!("'--{option}'"))
                .collect::<Vec<_>>()
                .join(" ");
            let mut message = b"option '".to_vec();
            message.extend_from_slice(argument);
            message.extend_from_slice(b"' is ambiguous; possibilities: ");
            message.extend_from_slice(possibilities.as_bytes());
            Err(TacUsageError::boxed(message))
        }
        TacLongOptionMatch::Recognized("separator") if separator.is_none() && !has_next => Err(
            TacUsageError::boxed(b"option '--separator' requires an argument".to_vec()),
        ),
        TacLongOptionMatch::Recognized("separator") => Ok(separator.is_none()),
        TacLongOptionMatch::Recognized(canonical) if separator.is_some() => {
            Err(TacUsageError::boxed(
                format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
            ))
        }
        TacLongOptionMatch::Recognized("help" | "version") => Ok(false),
        TacLongOptionMatch::Recognized(_) => Ok(false),
    }
}

fn tac_validate_options(args: &[OsString], posixly_correct: bool) -> CTResult<()> {
    let mut index = 1;
    while index < args.len() {
        let argument = args[index].as_bytes();
        if argument == b"--" {
            break;
        }
        if argument.len() <= 1 || argument[0] != b'-' {
            if posixly_correct {
                break;
            }
            index += 1;
            continue;
        }

        if argument.starts_with(b"--") {
            let consumes_next = tac_validate_long_option(argument, index + 1 < args.len())?;
            let name_end = argument[2..]
                .iter()
                .position(|byte| *byte == b'=')
                .map_or(argument.len(), |equals| equals + 2);
            if matches!(
                tac_match_long_option(&argument[2..name_end]),
                TacLongOptionMatch::Recognized("help" | "version")
            ) {
                return Ok(());
            }
            index += usize::from(consumes_next) + 1;
            continue;
        }

        let mut option_index = 1;
        while option_index < argument.len() {
            let option = argument[option_index];
            if !TAC_SHORT_OPTIONS.contains(&option) {
                let mut message = b"invalid option -- '".to_vec();
                message.push(option);
                message.push(b'\'');
                return Err(TacUsageError::boxed(message));
            }
            if option == b's' {
                if option_index + 1 < argument.len() {
                    break;
                }
                if index + 1 == args.len() {
                    return Err(TacUsageError::boxed(
                        b"option requires an argument -- 's'".to_vec(),
                    ));
                }
                index += 1;
                break;
            }
            option_index += 1;
        }
        index += 1;
    }

    Ok(())
}

fn tac_parse_invocation(args: impl ctcore::Args) -> CTResult<TacFlags> {
    let args = args.collect::<Vec<_>>();
    tac_validate_options(&args, ctcore::ct_posix::posixly_correct())?;
    let matches = ct_app().try_get_matches_from(args)?;
    TacFlags::new(&matches)
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

fn tac_write_slice<W: Write>(
    writer: &mut W,
    data: &[u8],
    write_error: &mut Option<std::io::Error>,
) {
    if write_error.is_none()
        && let Err(error) = writer.write_all(data)
    {
        *write_error = Some(error);
    }
}

fn tac_write_string_slices<W: Write>(
    writer: &mut W,
    data: &[u8],
    before: bool,
    separator: &[u8],
    write_error: &mut Option<std::io::Error>,
) {
    let separator_length = separator.len();
    let mut following_line_start = data.len();

    for start in memmem::rfind_iter(data, separator) {
        let segment = if before {
            let segment = &data[start..following_line_start];
            following_line_start = start;
            segment
        } else {
            let segment = &data[start + separator_length..following_line_start];
            following_line_start = start + separator_length;
            segment
        };
        tac_write_slice(writer, segment, write_error);
    }

    tac_write_slice(writer, &data[..following_line_start], write_error);
}

fn tac_write_regex_slices<W: Write>(
    writer: &mut W,
    data: &[u8],
    pattern: &mut GnuRegex,
    before: bool,
    write_error: &mut Option<std::io::Error>,
) -> Result<(), TacError> {
    let mut search_end = data.len();
    let mut past_end = data.len();
    let mut first_match = true;

    while let Some(found) = pattern
        .search_backward(&data[..search_end])
        .map_err(TacError::from)?
    {
        let (start, end) = (found.start, found.end);
        if before {
            tac_write_slice(writer, &data[start..past_end], write_error);
            past_end = start;
        } else {
            if !first_match || end != past_end {
                tac_write_slice(writer, &data[end..past_end], write_error);
            }
            past_end = end;
            first_match = false;
        }

        search_end = start;
    }

    tac_write_slice(writer, &data[..past_end], write_error);
    Ok(())
}

fn tac_search_backward_prefix(
    pattern: &mut GnuRegex,
    data: &[u8],
) -> Result<Option<GnuRegexMatch>, TacError> {
    if data.is_empty() {
        return Ok(None);
    }
    let range = 1isize
        .checked_sub(isize::try_from(data.len()).map_err(|_| TacError::RecordTooLarge)?)
        .ok_or(TacError::RecordTooLarge)?;
    pattern
        .search(data, data.len() - 1, range)
        .map_err(TacError::from)
}

struct TacSeekableInput<'a> {
    fd: ctcore::libc::c_int,
    estimated_size: u64,
    source: Option<&'a Path>,
}

struct TacInputState {
    temporary_stream: Option<NamedTempFile>,
    read_size: usize,
}

fn tac_write_seekable_fd<W: Write>(
    writer: &mut W,
    input: TacSeekableInput<'_>,
    settings: &TacFlags,
    mut pattern: Option<&mut GnuRegex>,
    read_size: &mut usize,
    write_error: &mut Option<std::io::Error>,
    stdout_was_closed: bool,
) -> CTResult<()> {
    let file_size = tac_actual_file_size(input.fd, input.estimated_size, *read_size, input.source)?;
    if stdout_was_closed && file_size != 0 && write_error.is_none() {
        *write_error = Some(std::io::Error::from_raw_os_error(ctcore::libc::EBADF));
    }

    let mut position = file_size;
    let mut pending = Vec::new();
    let mut fixed_buffer = Vec::new();
    let mut fixed_initialized_len = 0_usize;
    let mut first_match = true;
    let mut first_read = true;

    while position != 0 {
        let count = if first_read {
            let remainder = position % *read_size as u64;
            first_read = false;
            if remainder == 0 {
                position.min(*read_size as u64)
            } else {
                remainder
            }
        } else if position < *read_size as u64 {
            *read_size = usize::try_from(position).map_err(|_| TacError::RecordTooLarge)?;
            position
        } else {
            *read_size as u64
        };
        let count = usize::try_from(count).map_err(|_| TacError::RecordTooLarge)?;
        position -= count as u64;

        let mut next = vec![0_u8; count];
        tac_pread_exact(input.fd, position, &mut next, input.source)?;
        if let Some(pattern) = pattern.as_deref_mut() {
            next.extend_from_slice(&pending);
            pending = next;
            let mut search_end = pending.len();
            let mut past_end = pending.len();
            loop {
                let Some(found) = tac_search_backward_prefix(pattern, &pending[..search_end])?
                else {
                    break;
                };

                if settings.is_before {
                    tac_write_slice(writer, &pending[found.start..past_end], write_error);
                    past_end = found.start;
                } else {
                    if !first_match || found.end != past_end {
                        tac_write_slice(writer, &pending[found.end..past_end], write_error);
                    }
                    past_end = found.end;
                    first_match = false;
                }
                search_end = found.start;
            }
            pending.truncate(past_end);
        } else {
            let saved_record_size = pending.len();
            let initialized_end = count
                .checked_add(saved_record_size)
                .ok_or(TacError::RecordTooLarge)?;
            let search_capacity = count
                .checked_add(settings.separator.len() - 1)
                .ok_or(TacError::RecordTooLarge)?;
            let required_capacity = initialized_end.max(search_capacity);
            if fixed_buffer.len() < required_capacity {
                fixed_buffer.resize(required_capacity, 0);
            }
            fixed_buffer[count..initialized_end].copy_from_slice(&pending);
            fixed_buffer[..count].copy_from_slice(&next);
            fixed_initialized_len = fixed_initialized_len.max(initialized_end);

            let mut past_end = initialized_end;
            let fixed_search_end = search_capacity.min(fixed_initialized_len);
            for start in memmem::rfind_iter(&fixed_buffer[..fixed_search_end], &settings.separator)
            {
                let end = start + settings.separator.len();
                if settings.is_before {
                    tac_write_slice(writer, &fixed_buffer[start..past_end], write_error);
                    past_end = start;
                } else {
                    if !first_match || end != past_end {
                        tac_write_slice(writer, &fixed_buffer[end..past_end], write_error);
                    }
                    past_end = end;
                    first_match = false;
                }
            }
            pending.clear();
            pending.extend_from_slice(&fixed_buffer[..past_end]);
        }
        if position != 0 && pending.len() > *read_size {
            *read_size = read_size.checked_mul(2).ok_or(TacError::RecordTooLarge)?;
        }
    }

    tac_write_slice(writer, &pending, write_error);
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

fn tac_write_file_with_regex<W: Write>(
    writer: &mut W,
    filename: &OsStr,
    settings: &TacFlags,
    pattern: Option<&mut GnuRegex>,
    input_state: &mut TacInputState,
    write_error: &mut Option<std::io::Error>,
    stdout_was_closed: bool,
) -> CTResult<()> {
    let data = if filename.as_bytes() == b"-" {
        if ctcore::ct_io::injected_stdin_bytes().is_none() && !ctcore::ct_stdin_was_closed() {
            if let Some(file_size) = tac_regular_file_size(ctcore::libc::STDIN_FILENO) {
                return tac_write_seekable_fd(
                    writer,
                    TacSeekableInput {
                        fd: ctcore::libc::STDIN_FILENO,
                        estimated_size: file_size,
                        source: None,
                    },
                    settings,
                    pattern,
                    &mut input_state.read_size,
                    write_error,
                    stdout_was_closed,
                );
            }

            let file_size = copy_nonseekable_to_reusable(
                ctcore::ct_io::stdin_reader_box(),
                None,
                &mut input_state.temporary_stream,
            )?;
            let temporary = input_state
                .temporary_stream
                .as_ref()
                .expect("successful temporary copy initializes the stream");
            return tac_write_seekable_fd(
                writer,
                TacSeekableInput {
                    fd: temporary.as_file().as_raw_fd(),
                    estimated_size: file_size,
                    source: Some(temporary.path()),
                },
                settings,
                pattern,
                &mut input_state.read_size,
                write_error,
                stdout_was_closed,
            );
        }
        get_file_data(filename)?
    } else {
        let path = Path::new(filename);
        let file = open_file(path)?;
        if let Some(file_size) = tac_regular_file_size(file.as_raw_fd()) {
            return tac_write_seekable_fd(
                writer,
                TacSeekableInput {
                    fd: file.as_raw_fd(),
                    estimated_size: file_size,
                    source: Some(path),
                },
                settings,
                pattern,
                &mut input_state.read_size,
                write_error,
                stdout_was_closed,
            );
        }
        if !file.metadata().is_ok_and(|metadata| metadata.is_dir())
            && !tac_seek_to_start_if_seek_end_supported(file.as_raw_fd())
        {
            let file_size =
                copy_nonseekable_to_reusable(file, Some(path), &mut input_state.temporary_stream)?;
            let temporary = input_state
                .temporary_stream
                .as_ref()
                .expect("successful temporary copy initializes the stream");
            return tac_write_seekable_fd(
                writer,
                TacSeekableInput {
                    fd: temporary.as_file().as_raw_fd(),
                    estimated_size: file_size,
                    source: Some(temporary.path()),
                },
                settings,
                pattern,
                &mut input_state.read_size,
                write_error,
                stdout_was_closed,
            );
        }
        get_open_file_data(file, path)?
    };
    if stdout_was_closed && !data.as_ref().is_empty() {
        if write_error.is_none() {
            *write_error = Some(std::io::Error::from_raw_os_error(ctcore::libc::EBADF));
        }
        return Ok(());
    }
    if let Some(pattern) = pattern {
        tac_write_regex_slices(
            writer,
            data.as_ref(),
            pattern,
            settings.is_before,
            write_error,
        )?;
    } else {
        tac_write_string_slices(
            writer,
            data.as_ref(),
            settings.is_before,
            &settings.separator,
            write_error,
        );
    }
    Ok(())
}

fn tac_finish_write_error<W: Write>(
    writer: &mut W,
    write_error: std::io::Error,
    stdout_was_closed: bool,
) -> CTResult<()> {
    if stdout_was_closed {
        return Err(
            TacError::FlushError(std::io::Error::from_raw_os_error(ctcore::libc::EBADF)).into(),
        );
    }
    writer.flush().map_err(TacError::FlushError)?;
    Err(TacError::WriteError(write_error).into())
}

fn tac_fd_is_unwritable(fd: ctcore::libc::c_int) -> bool {
    let flags = unsafe { ctcore::libc::fcntl(fd, ctcore::libc::F_GETFL) };
    flags < 0 || flags & ctcore::libc::O_ACCMODE == ctcore::libc::O_RDONLY
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
fn tac<W: Write>(writer: &mut W, settings: &TacFlags, stdout_was_closed: bool) -> CTResult<()> {
    let mut has_error = false;
    let mut write_error = None;
    let mut read_stdin = false;
    let mut pattern = tac_compile_regex(settings)?;
    let mut read_size = GNU_TAC_READ_SIZE;
    if pattern.is_none() {
        while settings.separator.len() >= read_size / 2 {
            read_size = read_size.checked_mul(2).ok_or(TacError::RecordTooLarge)?;
        }
    }
    let mut input_state = TacInputState {
        temporary_stream: None,
        read_size,
    };

    for filename in &settings.files {
        read_stdin |= filename.as_bytes() == b"-";
        match tac_write_file_with_regex(
            writer,
            filename,
            settings,
            pattern.as_mut(),
            &mut input_state,
            &mut write_error,
            stdout_was_closed,
        ) {
            Ok(()) => {}
            Err(error) => {
                ctcore::ct_show_error!("{}", error);
                has_error = true;
            }
        }
    }

    if read_stdin && ctcore::ct_stdin_was_closed() {
        ctcore::ct_show_error!("-: Bad file descriptor");
    }

    if let Some(error) = write_error {
        return tac_finish_write_error(writer, error, stdout_was_closed);
    }

    writer.flush().map_err(TacError::FlushError)?;

    if has_error {
        // 使用 CtSimpleError 返回一个通用的非零退出码
        return Err(ctcore::ct_error::CtSimpleError::new(1, String::new()));
    }

    Ok(())
}

pub fn tac_native_semantic(args: impl ctcore::Args) -> CTResult<TacSemantic> {
    initialize_tac_locale();
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
        tac_main_with_stdout_state(
            &mut stdout,
            args.iter().cloned(),
            ctcore::ct_stdout_was_closed() || tac_fd_is_unwritable(ctcore::libc::STDOUT_FILENO),
        )
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
        fn test_tac_flags_accept_option_looking_separator_values() {
            let short_matches = tac_command()
                .gnu_getopt_with_mode(false)
                .try_get_matches_from(["tac", "-s", "--", "input"])
                .unwrap();
            let short_flags = TacFlags::new(&short_matches).unwrap();
            assert_eq!(short_flags.separator, b"--");
            assert_eq!(short_flags.files, vec!["input"]);

            let long_matches = tac_command()
                .gnu_getopt_with_mode(false)
                .try_get_matches_from(["tac", "--separator", "-b", "input"])
                .unwrap();
            let long_flags = TacFlags::new(&long_matches).unwrap();
            assert_eq!(long_flags.separator, b"-b");
            assert!(!long_flags.is_before);
            assert_eq!(long_flags.files, vec!["input"]);
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
            let matches = tac_command()
                .gnu_getopt_with_mode(true)
                .try_get_matches_from(["tac", "-s", ":", "input", "-b"])
                .unwrap();
            let flags = TacFlags::new(&matches).unwrap();

            assert!(!flags.is_before);
            assert_eq!(flags.separator, b":");
            assert_eq!(flags.files, vec!["input", "-b"]);
        }

        #[test]
        fn test_posixly_correct_recognizes_attached_separator_value() {
            let matches = tac_command()
                .gnu_getopt_with_mode(true)
                .try_get_matches_from(["tac", "-brs:", "input", "--regex"])
                .unwrap();
            let flags = TacFlags::new(&matches).unwrap();

            assert!(flags.is_before);
            assert!(flags.is_regex);
            assert_eq!(flags.separator, b":");
            assert_eq!(flags.files, vec!["input", "--regex"]);
        }

        #[test]
        fn test_tac_reports_gnu_option_diagnostics() {
            for (args, expected) in [
                (
                    vec!["tac", "--bad_flag"],
                    "unrecognized option '--bad_flag'",
                ),
                (vec!["tac", "-x"], "invalid option -- 'x'"),
                (vec!["tac", "-h"], "invalid option -- 'h'"),
                (vec!["tac", "-bV"], "invalid option -- 'V'"),
                (
                    vec!["tac", "--before=x"],
                    "option '--before' doesn't allow an argument",
                ),
                (
                    vec!["tac", "--separator"],
                    "option '--separator' requires an argument",
                ),
                (vec!["tac", "-s"], "option requires an argument -- 's'"),
            ] {
                let error = tac_parse_invocation(args.into_iter().map(OsString::from))
                    .expect_err("invalid GNU option form must fail");
                assert_eq!(error.to_string(), expected);
                assert!(error.usage());
            }
        }

        #[test]
        fn test_tac_reports_ambiguous_and_raw_option_diagnostics() {
            let ambiguous = tac_parse_invocation(["tac", "--="].into_iter().map(OsString::from))
                .expect_err("empty long option prefix must be ambiguous");
            assert_eq!(
                ambiguous.to_string(),
                "option '--=' is ambiguous; possibilities: '--before' '--regex' '--separator' '--help' '--version'"
            );

            let raw = tac_validate_options(
                &[
                    OsString::from("tac"),
                    OsStr::from_bytes(&[b'-', 0xff]).to_os_string(),
                ],
                false,
            )
            .expect_err("unknown option byte must fail");
            assert_eq!(raw.diagnostic_bytes().as_ref(), b"invalid option -- '\xff'");
        }

        #[test]
        fn test_tac_option_validation_preserves_getopt_control_flow() {
            for args in [
                vec!["tac", "--help", "--bad"],
                vec!["tac", "-s", "--bad", "input"],
                vec!["tac", "-brs:", "input"],
                vec!["tac", "--sep=:", "input"],
            ] {
                tac_validate_options(
                    &args.into_iter().map(OsString::from).collect::<Vec<_>>(),
                    false,
                )
                .expect("valid GNU option flow must pass prevalidation");
            }

            tac_validate_options(
                &[
                    OsString::from("tac"),
                    OsString::from("input"),
                    OsString::from("--bad"),
                ],
                true,
            )
            .expect("POSIX mode must stop option validation at the first operand");
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
        fn test_tac_message_locale_uses_gettext_category_priority() {
            assert_eq!(
                tac_message_locale_from_values(
                    None,
                    Some("zh_CN.UTF-8"),
                    Some("en_US.UTF-8"),
                    None
                ),
                "zh-CN"
            );
            assert_eq!(
                tac_message_locale_from_values(None, None, Some("zh_CN.UTF-8"), Some("C")),
                "en-US"
            );
            assert_eq!(
                tac_message_locale_from_values(
                    None,
                    None,
                    Some("zh_CN.UTF-8"),
                    Some("does_NOT_exist")
                ),
                "en-US"
            );
            assert_eq!(
                tac_message_locale_from_values(
                    Some("C"),
                    Some("zh_CN.UTF-8"),
                    Some("zh_CN.UTF-8"),
                    Some("zh_CN")
                ),
                "en-US"
            );
        }

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

        #[test]
        fn test_tac_temporary_create_error_includes_attempted_path() {
            let error = TacError::TemporaryFileCreate(
                OsString::from("/proc/cutmpXXXXXX"),
                std::io::Error::from_raw_os_error(ctcore::libc::ENOENT),
            );
            assert_eq!(
                tac_error_message(&error, "en-US"),
                "failed to create temporary file '/proc/cutmpXXXXXX': No such file or directory"
            );
        }

        #[test]
        fn test_tac_temporary_create_failure_captures_generated_path() {
            let error = tac_create_temporary_file(Path::new("/proc"))
                .expect_err("procfs must reject regular temporary files");
            let TacError::TemporaryFileCreate(path, _) = error else {
                panic!("expected a temporary-file creation error");
            };
            assert!(path.as_bytes().starts_with(b"/proc/cutmp"));
            assert_eq!(path.as_bytes().len(), b"/proc/cutmpXXXXXX".len());
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
        use std::os::unix::process::ExitStatusExt;
        use std::process::Stdio;
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
        fn test_get_file_data_seekable_directory_preserves_read_error() {
            let error = get_file_data(OsStr::new(".")).unwrap_err();

            assert_eq!(error.to_string(), ".: read error: Invalid argument");
        }

        #[test]
        fn test_get_file_data_nonseekable_directory_preserves_read_error() {
            let error = get_file_data(OsStr::new("/tmp")).unwrap_err();

            assert_eq!(error.to_string(), "/tmp: read error: Is a directory");
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

        #[test]
        fn test_nonseekable_temporary_is_unlinked_while_open() {
            let (mut temporary, _) = copy_nonseekable_to_temporary(&b"first\nsecond\n"[..], None)
                .expect("nonseekable input must be copied to a temporary file");
            let temporary_path = temporary.path().to_path_buf();

            assert!(
                !temporary_path.exists(),
                "GNU tac unlinks its temporary file immediately after creation"
            );
            let mut contents = Vec::new();
            temporary
                .as_file_mut()
                .seek(SeekFrom::Start(0))
                .expect("the unlinked temporary file must remain seekable");
            temporary
                .as_file_mut()
                .read_to_end(&mut contents)
                .expect("the unlinked temporary file must remain readable");
            assert_eq!(contents, b"first\nsecond\n");
        }

        #[test]
        fn test_tac_main_reuses_temporary_file_across_nonseekable_operands() {
            const CHILD_ENV: &str = "TAC_REUSABLE_TEMPORARY_CHILD";
            if std::env::var_os(CHILD_ENV).is_some() {
                let directory = tempdir().unwrap();
                unsafe {
                    std::env::set_var("TMPDIR", directory.path());
                }
                let directory_path = CString::new(directory.path().as_os_str().as_bytes()).unwrap();
                let inotify_fd = unsafe {
                    ctcore::libc::inotify_init1(
                        ctcore::libc::IN_NONBLOCK | ctcore::libc::IN_CLOEXEC,
                    )
                };
                assert!(inotify_fd >= 0);
                assert!(
                    unsafe {
                        ctcore::libc::inotify_add_watch(
                            inotify_fd,
                            directory_path.as_ptr(),
                            ctcore::libc::IN_CREATE,
                        )
                    } >= 0
                );

                let mut output = std::io::sink();
                tac_main(
                    &mut output,
                    ["tac", "/proc/self/status", "/proc/self/status"]
                        .into_iter()
                        .map(OsString::from),
                )
                .unwrap();

                let mut events = [0_u8; 4096];
                let bytes_read = unsafe {
                    ctcore::libc::read(inotify_fd, events.as_mut_ptr().cast(), events.len())
                };
                unsafe {
                    ctcore::libc::close(inotify_fd);
                }
                assert!(bytes_read > 0);
                let mut offset = 0_usize;
                let mut create_count = 0_usize;
                while offset < bytes_read as usize {
                    let event = unsafe {
                        &*events
                            .as_ptr()
                            .add(offset)
                            .cast::<ctcore::libc::inotify_event>()
                    };
                    if event.mask & ctcore::libc::IN_CREATE != 0 {
                        create_count += 1;
                    }
                    offset +=
                        std::mem::size_of::<ctcore::libc::inotify_event>() + event.len as usize;
                }
                assert_eq!(create_count, 1, "GNU tac reuses one temporary stream");
                return;
            }

            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::file_operations_tests::test_tac_main_reuses_temporary_file_across_nonseekable_operands",
                ])
                .env(CHILD_ENV, "1")
                .status()
                .unwrap();
            assert!(status.success(), "child status: {status:?}");
        }

        #[test]
        fn test_nonseekable_input_honors_temporary_file_size_limit() {
            const CHILD_ENV: &str = "TAC_NONSEEKABLE_FSIZE_CHILD";
            if std::env::var_os(CHILD_ENV).is_some() {
                let mut pipe_fds = [-1; 2];
                assert_eq!(unsafe { ctcore::libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
                let input = [b'x'; 4096];
                assert_eq!(
                    unsafe { ctcore::libc::write(pipe_fds[1], input.as_ptr().cast(), input.len()) },
                    input.len() as isize
                );
                unsafe {
                    ctcore::libc::close(pipe_fds[1]);
                    ctcore::libc::dup2(pipe_fds[0], ctcore::libc::STDIN_FILENO);
                    ctcore::libc::close(pipe_fds[0]);
                    ctcore::libc::signal(ctcore::libc::SIGXFSZ, ctcore::libc::SIG_DFL);
                }
                let limit = ctcore::libc::rlimit {
                    rlim_cur: 1024,
                    rlim_max: 1024,
                };
                assert_eq!(
                    unsafe { ctcore::libc::setrlimit(ctcore::libc::RLIMIT_FSIZE, &limit) },
                    0
                );

                let _ = get_file_data(OsStr::new("-")).unwrap();
                std::process::exit(0);
            }

            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::file_operations_tests::test_nonseekable_input_honors_temporary_file_size_limit",
                ])
                .env(CHILD_ENV, "1")
                .status()
                .unwrap();

            assert_eq!(status.signal(), Some(ctcore::libc::SIGXFSZ));
        }

        #[test]
        fn test_proc_file_without_seek_end_uses_temporary_file() {
            const CHILD_ENV: &str = "TAC_PROC_TEMP_CHILD";
            if std::env::var_os(CHILD_ENV).is_some() {
                let error = get_file_data(OsStr::new("/proc/self/status"))
                    .expect_err("procfs input without SEEK_END must use a temporary file");
                assert!(
                    error
                        .to_string()
                        .starts_with("failed to create temporary file '/proc/cutmp")
                );
                assert!(error.to_string().ends_with(": No such file or directory"));
                return;
            }

            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::file_operations_tests::test_proc_file_without_seek_end_uses_temporary_file",
                ])
                .env(CHILD_ENV, "1")
                .env("TMPDIR", "/proc")
                .status()
                .unwrap();

            assert!(status.success());
        }

        #[test]
        fn test_large_seekable_input_does_not_copy_all_segments() {
            const CHILD_ENV: &str = "TAC_MEMORY_LIMIT_CHILD";
            const PATH_ENV: &str = "TAC_MEMORY_LIMIT_PATH";
            if std::env::var_os(CHILD_ENV).is_some() {
                let limit = ctcore::libc::rlimit {
                    rlim_cur: 32 * 1024 * 1024,
                    rlim_max: 32 * 1024 * 1024,
                };
                assert_eq!(
                    unsafe { ctcore::libc::setrlimit(ctcore::libc::RLIMIT_AS, &limit) },
                    0
                );
                let path = std::env::var_os(PATH_ENV).unwrap();
                let mut output = std::io::sink();
                tac_main(
                    &mut output,
                    [OsString::from("tac"), path.clone()].into_iter(),
                )
                .unwrap();

                let input = File::open(path).unwrap();
                assert_eq!(
                    unsafe { ctcore::libc::dup2(input.as_raw_fd(), ctcore::libc::STDIN_FILENO) },
                    ctcore::libc::STDIN_FILENO
                );
                tac_main(&mut output, [OsString::from("tac")].into_iter()).unwrap();
                return;
            }

            let mut input = NamedTempFile::new().unwrap();
            let mut block = [b'x'; GNU_TAC_READ_SIZE];
            block[GNU_TAC_READ_SIZE - 1] = b'\n';
            for _ in 0..(64 * 1024 * 1024 / GNU_TAC_READ_SIZE) {
                input.write_all(&block).unwrap();
            }
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::file_operations_tests::test_large_seekable_input_does_not_copy_all_segments",
                ])
                .env(CHILD_ENV, "1")
                .env(PATH_ENV, input.path())
                .status()
                .unwrap();

            assert!(status.success(), "child status: {status:?}");
        }

        #[test]
        fn test_large_nonseekable_input_does_not_map_temporary_file() {
            const CHILD_ENV: &str = "TAC_NONSEEKABLE_MEMORY_LIMIT_CHILD";
            if std::env::var_os(CHILD_ENV).is_some() {
                let limit = ctcore::libc::rlimit {
                    rlim_cur: 32 * 1024 * 1024,
                    rlim_max: 32 * 1024 * 1024,
                };
                assert_eq!(
                    unsafe { ctcore::libc::setrlimit(ctcore::libc::RLIMIT_AS, &limit) },
                    0
                );
                let mut output = std::io::sink();
                tac_main(&mut output, [OsString::from("tac")].into_iter()).unwrap();
                return;
            }

            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::file_operations_tests::test_large_nonseekable_input_does_not_map_temporary_file",
                ])
                .env(CHILD_ENV, "1")
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            let mut child_stdin = child.stdin.take().unwrap();
            let mut block = [b'x'; GNU_TAC_READ_SIZE];
            block[GNU_TAC_READ_SIZE - 1] = b'\n';
            for _ in 0..(64 * 1024 * 1024 / GNU_TAC_READ_SIZE) {
                if child_stdin.write_all(&block).is_err() {
                    break;
                }
            }
            drop(child_stdin);
            let status = child.wait().unwrap();

            assert!(status.success(), "child status: {status:?}");
        }

        #[test]
        fn test_tac_main_regex_uses_gnu_aligned_read_blocks() {
            let mut input = NamedTempFile::new().unwrap();
            input.write_all(&vec![b'A'; GNU_TAC_READ_SIZE]).unwrap();
            input.write_all(&vec![b'B'; 808]).unwrap();
            let args = [
                OsString::from("tac"),
                OsString::from("-r"),
                OsString::from("-s"),
                OsString::from("^"),
                input.path().as_os_str().to_os_string(),
            ];
            let mut output = Vec::new();

            tac_main(&mut output, args.into_iter()).unwrap();

            let mut expected = vec![b'B'; 808];
            expected.extend_from_slice(&vec![b'A'; GNU_TAC_READ_SIZE]);
            assert_eq!(output, expected);
        }

        #[test]
        fn test_tac_main_handles_regular_files_with_estimated_size() {
            let path = Path::new("/sys/kernel/profiling");
            let Ok(expected) = fs::read(path) else {
                return;
            };
            let mut output = Vec::new();

            tac_main(
                &mut output,
                [OsString::from("tac"), path.as_os_str().to_os_string()].into_iter(),
            )
            .unwrap();

            assert_eq!(output, expected);
        }

        #[test]
        fn test_tac_main_fixed_separator_preserves_gnu_boundary_overlap() {
            let mut input = NamedTempFile::new().unwrap();
            input.write_all(&vec![b'A'; GNU_TAC_READ_SIZE - 1]).unwrap();
            input.write_all(b"::: ").unwrap();
            input.write_all(&vec![b'B'; 805]).unwrap();
            let args = [
                OsString::from("tac"),
                OsString::from("-s"),
                OsString::from("::"),
                input.path().as_os_str().to_os_string(),
            ];
            let mut output = Vec::new();

            tac_main(&mut output, args.into_iter()).unwrap();

            let mut expected = vec![b' '];
            expected.extend_from_slice(&vec![b'B'; 805]);
            expected.push(b':');
            expected.extend_from_slice(&vec![b'A'; GNU_TAC_READ_SIZE - 1]);
            expected.extend_from_slice(b"::");
            assert_eq!(output, expected);
        }

        #[test]
        fn test_tac_main_fixed_before_preserves_cross_block_overlap() {
            let mut data = vec![b'X'; GNU_TAC_READ_SIZE * 2 + 1];
            data[GNU_TAC_READ_SIZE - 2..GNU_TAC_READ_SIZE + 3].copy_from_slice(b"ababa");
            *data.last_mut().unwrap() = b'a';
            let mut input = NamedTempFile::new().unwrap();
            input.write_all(&data).unwrap();
            let args = [
                OsString::from("tac"),
                OsString::from("--before"),
                OsString::from("--separator=aba"),
                input.path().as_os_str().to_os_string(),
            ];
            let mut output = Vec::new();

            tac_main(&mut output, args.into_iter()).unwrap();

            let mut expected = b"aba".to_vec();
            expected.extend_from_slice(&vec![b'X'; GNU_TAC_READ_SIZE - 3]);
            expected.extend_from_slice(b"aab");
            expected.extend_from_slice(&vec![b'X'; GNU_TAC_READ_SIZE - 2]);
            assert_eq!(output, expected);
        }

        #[test]
        fn test_tac_main_fixed_before_ignores_uninitialized_cross_block_bytes() {
            let mut data = vec![b'X'; GNU_TAC_READ_SIZE * 2 + 1];
            data[GNU_TAC_READ_SIZE - 2..GNU_TAC_READ_SIZE + 3].copy_from_slice(b"ababa");
            let mut input = NamedTempFile::new().unwrap();
            input.write_all(&data).unwrap();
            let args = [
                OsString::from("tac"),
                OsString::from("--before"),
                OsString::from("--separator=aba"),
                input.path().as_os_str().to_os_string(),
            ];
            let mut output = Vec::new();

            tac_main(&mut output, args.into_iter()).unwrap();

            let mut expected = b"aba".to_vec();
            expected.extend_from_slice(&vec![b'X'; GNU_TAC_READ_SIZE * 2 - 4]);
            expected.extend_from_slice(b"ab");
            assert_eq!(output, expected);
        }

        #[test]
        fn test_tac_main_preserves_grown_read_size_across_operands() {
            let mut first = NamedTempFile::new().unwrap();
            first.write_all(&vec![b'A'; 50_000]).unwrap();

            let mut second_data = vec![b'b'; 50_000];
            second_data[40_960] = b'Z';
            let mut second = NamedTempFile::new().unwrap();
            second.write_all(&second_data).unwrap();

            let args = [
                OsString::from("tac"),
                OsString::from("-r"),
                OsString::from("-s"),
                OsString::from("^Z"),
                first.path().as_os_str().to_os_string(),
                second.path().as_os_str().to_os_string(),
            ];
            let mut output = Vec::new();

            tac_main(&mut output, args.into_iter()).unwrap();

            let mut expected = vec![b'A'; 50_000];
            expected.extend_from_slice(&second_data);
            assert_eq!(output, expected);
        }

        #[test]
        fn test_tac_main_preserves_final_partial_read_size_across_operands() {
            let mut first = NamedTempFile::new().unwrap();
            first.write_all(&vec![b'A'; 50_000]).unwrap();

            let mut second_data = vec![b'b'; 50_000];
            second_data[24_576] = b'Z';
            let mut second = NamedTempFile::new().unwrap();
            second.write_all(&second_data).unwrap();

            let args = [
                OsString::from("tac"),
                OsString::from("-r"),
                OsString::from("-s"),
                OsString::from("^Z"),
                first.path().as_os_str().to_os_string(),
                second.path().as_os_str().to_os_string(),
            ];
            let mut output = Vec::new();

            tac_main(&mut output, args.into_iter()).unwrap();

            let mut expected = vec![b'A'; 50_000];
            expected.extend_from_slice(&vec![b'b'; 49_999]);
            expected.push(b'Z');
            assert_eq!(output, expected);
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
            const CHILD_ENV: &str = "TAC_C_LOCALE_REGEX_CHILD";
            if std::env::var_os(CHILD_ENV).is_some() {
                assert_eq!(
                    reverse(b"A\xc3\xa9B\xc3\xa9C", b".", false),
                    b"C\xa9\xc3B\xa9\xc3A"
                );
                return;
            }

            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::tac_buffer_regex_tests::test_tac_buffer_regex_c_locale_dot_matches_one_byte",
                ])
                .env(CHILD_ENV, "1")
                .env("LC_ALL", "C")
                .status()
                .unwrap();

            assert!(status.success(), "child status: {status:?}");
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
        use std::ffi::{CString, OsString};
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

        #[derive(Default)]
        struct WriteAndFlushFailWriter {
            write_count: usize,
            flush_count: usize,
        }

        impl Write for WriteAndFlushFailWriter {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                self.write_count += 1;
                Err(io::Error::from_raw_os_error(ctcore::libc::EBADF))
            }

            fn flush(&mut self) -> io::Result<()> {
                self.flush_count += 1;
                Err(io::Error::from_raw_os_error(ctcore::libc::EBADF))
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
        fn test_tac_main_continues_reading_operands_after_write_error() {
            let mut first = NamedTempFile::new().unwrap();
            let mut second = NamedTempFile::new().unwrap();
            first.write_all(b"first").unwrap();
            second.write_all(b"second").unwrap();
            let second_path = CString::new(second.path().as_os_str().as_bytes()).unwrap();
            let inotify_fd = unsafe {
                ctcore::libc::inotify_init1(ctcore::libc::IN_NONBLOCK | ctcore::libc::IN_CLOEXEC)
            };
            assert!(inotify_fd >= 0);
            assert!(
                unsafe {
                    ctcore::libc::inotify_add_watch(
                        inotify_fd,
                        second_path.as_ptr(),
                        ctcore::libc::IN_OPEN,
                    )
                } >= 0
            );
            let args = [
                OsString::from("tac"),
                first.path().as_os_str().to_os_string(),
                second.path().as_os_str().to_os_string(),
            ];
            let mut output = WriteFailWriter::default();

            let error = tac_main(&mut output, args.into_iter()).unwrap_err();

            assert_eq!(output.write_count, 1);
            assert_eq!(error.to_string(), "write error");

            let mut events = [0_u8; 4096];
            let bytes_read =
                unsafe { ctcore::libc::read(inotify_fd, events.as_mut_ptr().cast(), events.len()) };
            unsafe {
                ctcore::libc::close(inotify_fd);
            }
            assert!(
                bytes_read > 0,
                "operands after a stdout error must still be opened"
            );
        }

        #[test]
        fn test_tac_main_preserves_errno_when_write_and_flush_fail() {
            let mut input = NamedTempFile::new().unwrap();
            input.write_all(b"data").unwrap();
            let args = [
                OsString::from("tac"),
                input.path().as_os_str().to_os_string(),
            ];
            let mut output = WriteAndFlushFailWriter::default();

            let error = tac_main(&mut output, args.into_iter()).unwrap_err();

            assert_eq!(output.write_count, 1);
            assert_eq!(output.flush_count, 1);
            assert_eq!(error.to_string(), "write error: Bad file descriptor");
        }

        #[test]
        fn test_tac_write_error_restores_closed_stdout_errno() {
            let mut output = WriteFailWriter::default();

            let error = tac_finish_write_error(
                &mut output,
                io::Error::from_raw_os_error(ctcore::libc::ENOSPC),
                true,
            )
            .unwrap_err();

            assert_eq!(error.to_string(), "write error: Bad file descriptor");
        }

        #[test]
        fn test_tac_detects_unwritable_file_descriptors() {
            let read_only = OpenOptions::new().read(true).open("/dev/null").unwrap();
            let write_only = OpenOptions::new().write(true).open("/dev/null").unwrap();

            assert!(tac_fd_is_unwritable(read_only.as_raw_fd()));
            assert!(!tac_fd_is_unwritable(write_only.as_raw_fd()));
        }

        #[test]
        fn test_tac_closed_stdout_rejects_output_before_buffering() {
            let mut input = NamedTempFile::new().unwrap();
            input.write_all(b"x").unwrap();
            let args = [
                OsString::from("tac"),
                input.path().as_os_str().to_os_string(),
            ];
            let mut output = Vec::new();

            let error =
                tac_main_with_stdout_state(&mut output, args.into_iter(), true).unwrap_err();

            assert!(output.is_empty());
            assert_eq!(error.to_string(), "write error: Bad file descriptor");
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
