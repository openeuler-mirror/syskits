/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! tac 是一个非常有用的 Linux 命令，它的功能是反向读取和输出文件的内容。
//! 这个命令的名字是 cat（concatenate，连接）的反向拼写，因此它以相反的顺序显示文件的行。

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTResult;
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show};
use memchr::memmem;
use memmap2::Mmap;
use std::io::{Read, Write, stdin, stdout};
use std::{
    fs::{File, read},
    path::Path,
};

use ctcore::ct_error::CTError;
use std::error::Error;
use std::fmt::Display;

// 定义about和usage
static TAC_USAGE: &str = ct_help_usage!("tac.md");
static TAC_ABOUT: &str = ct_help_about!("tac.md");

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
#[derive(Debug)]
struct TacFlags {
    is_before: bool,
    is_regex: bool,
    separator: String,
    files: Vec<String>,
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
            separator: String::from("\n"),
            files: vec![String::from("-")],
        }
    }
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
            .get_one::<String>(tac_flags::TAC_SEPARATOR)
            .map(String::as_str)
            .unwrap_or("\n")
            .to_string();

        // 处理空分隔符的特殊情况
        let separator = if separator.is_empty() {
            String::from("\0")
        } else {
            separator
        };

        // 向量类型参数提取
        let files = matches
            .get_many::<String>(tac_flags::TAC_FILE)
            .map_or_else(|| vec![String::from("-")], |v| v.cloned().collect());

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
    InvalidRegex(regex::Error),

    /// tac 的参数无效。
    InvalidArgument(String),

    /// 在文件系统中找不到指定的文件。
    FileNotFound(String),

    /// 读取文件或标准输入的内容时出错。参数是文件名和导致此错误的底层 [`std::io::Error`]。
    ReadError(String, std::io::Error),

    /// 写入（反转的）文件或标准输入内容时出错。参数是导致此错误的底层 [`std::io::Error`]。
    WriteError(std::io::Error),
}

impl CTError for TacError {
    fn code(&self) -> i32 {
        1
    }
}

impl Error for TacError {}

impl Display for TacError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRegex(e) => write!(f, "invalid regular expression: {e}"),
            Self::InvalidArgument(s) => {
                write!(f, "{}: read error: Invalid argument", s.maybe_quote())
            }
            Self::FileNotFound(s) => write!(
                f,
                "failed to open {} for reading: No such file or directory",
                s.quote()
            ),
            Self::ReadError(s, e) => write!(f, "failed to read from {s}: {e}"),
            Self::WriteError(e) => write!(f, "failed to write to stdout: {e}"),
        }
    }
}

/// tac 命令的入口点函数
///
/// # 参数
/// * `args` - 命令行参数
///
/// # 返回值
/// 返回 `CTResult<()>`，表示命令执行的结果
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let stdout = stdout();
    let mut out = stdout.lock();
    tac_main(&mut out, args)
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
    // 解析命令行参数
    let matches = ct_app().try_get_matches_from(args)?;

    // 创建配置对象
    let settings = TacFlags::new(&matches)?;

    // 使用配置执行主要逻辑
    tac(writer, &settings)
}

/// 创建并配置命令行参数解析器
///
/// # 返回值
/// 返回配置好的 `Command` 实例，用于解析命令行参数
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = TAC_ABOUT;
    let usage_description = ct_format_usage(TAC_USAGE);

    let args = vec![
        Arg::new(tac_flags::TAC_BEFORE)
            .short('b')
            .long(tac_flags::TAC_BEFORE)
            .help("attach the separator before instead of after")
            .action(ArgAction::SetTrue),
        Arg::new(tac_flags::TAC_REGEX)
            .short('r')
            .long(tac_flags::TAC_REGEX)
            .help("interpret the sequence as a regular expression")
            .action(ArgAction::SetTrue),
        Arg::new(tac_flags::TAC_SEPARATOR)
            .short('s')
            .long(tac_flags::TAC_SEPARATOR)
            .help("use STRING as the separator instead of newline")
            .value_name("STRING"),
        Arg::new(tac_flags::TAC_FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
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
fn tac_buffer_regex<W: Write>(
    writer: &mut W,
    data: &[u8],
    pattern: &regex::bytes::Regex,
    before: bool,
) -> std::io::Result<()> {
    let mut this_line_end = data.len();
    let mut following_line_start = data.len();

    for i in (0..data.len()).rev() {
        if let Some(match_) = pattern.find_at(&data[..this_line_end], i) {
            this_line_end = i;
            let slen = match_.end() - match_.start();

            if before {
                writer.write_all(&data[i + slen..following_line_start])?;
                if i > 0 {
                    writer.write_all(&data[i..i + slen])?;
                }
                following_line_start = i;
            } else {
                writer.write_all(&data[i + slen..following_line_start])?;
                following_line_start = i + slen;
            }
        }
    }

    writer.write_all(&data[0..following_line_start])?;
    Ok(())
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
fn tac_buffer<W: Write>(
    writer: &mut W,
    data: &[u8],
    before: bool,
    separator: &str,
) -> std::io::Result<()> {
    let slen = separator.len();
    let mut following_line_start = data.len();

    for i in memmem::rfind_iter(data, separator) {
        if before {
            writer.write_all(&data[i + slen..following_line_start])?;
            if i > 0 {
                writer.write_all(separator.as_bytes())?;
            }
            following_line_start = i;
        } else {
            writer.write_all(&data[i + slen..following_line_start])?;
            following_line_start = i + slen;
        }
    }

    writer.write_all(&data[0..following_line_start])?;
    Ok(())
}

/// 从标准输入读取数据
///
/// # 返回值
/// 返回 `CTResult<Vec<u8>>`，包含读取的数据或错误信息
fn read_from_stdin() -> CTResult<Vec<u8>> {
    let mut buffer = Vec::new();
    stdin()
        .read_to_end(&mut buffer)
        .map_err(|e| TacError::ReadError("stdin".to_string(), e))?;
    Ok(buffer)
}

/// 从指定文件读取数据
///
/// # 参数
/// * `path` - 文件路径
///
/// # 返回值
/// 返回 `CTResult<Vec<u8>>`，包含读取的数据或错误信息
fn read_from_file(path: &Path) -> CTResult<Vec<u8>> {
    read(path).map_err(|e| {
        let filename = path.to_string_lossy().to_string().quote().to_string();
        TacError::ReadError(filename, e).into()
    })
}

/// 验证文件路径的有效性
///
/// # 参数
/// * `path` - 要验证的文件路径
///
/// # 返回值
/// 返回 `CTResult<()>`，表示验证结果
///
/// # 错误
/// - 如果路径指向目录，返回 InvalidArgument 错误
/// - 如果文件不存在，返回 FileNotFound 错误
fn validate_file_path(path: &Path) -> CTResult<()> {
    if path.is_dir() {
        return Err(TacError::InvalidArgument(path.to_string_lossy().to_string()).into());
    }

    if path.metadata().is_err() {
        return Err(TacError::FileNotFound(path.to_string_lossy().to_string()).into());
    }

    Ok(())
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
fn get_file_data(filename: &str) -> CTResult<FileData> {
    if filename == "-" {
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
        validate_file_path(path)?;

        if let Some(mmap) = tac_try_mmap_path(path) {
            Ok(FileData::Mapped(mmap))
        } else {
            let buffer = read_from_file(path)?;
            Ok(FileData::Buffer(buffer))
        }
    }
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
fn tac_process_file<W: Write>(writer: &mut W, filename: &str, settings: &TacFlags) -> CTResult<()> {
    let data = get_file_data(filename)?;

    if settings.is_regex {
        let pattern =
            regex::bytes::Regex::new(&settings.separator).map_err(TacError::InvalidRegex)?;
        tac_buffer_regex(writer, data.as_ref(), &pattern, settings.is_before)
    } else {
        tac_buffer(
            writer,
            data.as_ref(),
            settings.is_before,
            &settings.separator,
        )
    }
    .map_err(TacError::WriteError)?;

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
    for filename in &settings.files {
        if let Err(e) = tac_process_file(writer, filename, settings) {
            ct_show!(e);
        }
    }
    Ok(())
}

/// 尝试对标准输入进行内存映射
///
/// # 返回值
/// 返回 `Option<Mmap>`，成功时返回内存映射对象，失败时返回 None
fn tac_try_mmap_stdin() -> Option<Mmap> {
    // SAFETY: 如果在映射文件时文件被截断，将会引发 SIGBUS 信号并终止我们的进程，从而防止访问无效内存。
    unsafe { Mmap::map(&stdin()).ok() }
}

/// 尝试对文件进行内存映射
///
/// # 参数
/// * `path` - 要映射的文件路径
///
/// # 返回值
/// 返回 `Option<Mmap>`，成功时返回内存映射对象，失败时返回 None
fn tac_try_mmap_path(path: &Path) -> Option<Mmap> {
    let file = File::open(path).ok()?;

    // SAFETY: 如果在映射文件时文件被截断，将会引发 SIGBUS 信号并终止我们的进程，从而防止访问无效内存。
    let mmap = unsafe { Mmap::map(&file).ok()? };

    Some(mmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod tac_flags_tests {
        use super::*;

        #[test]
        fn test_tac_flags_default() {
            let flags = TacFlags::default();
            assert!(!flags.is_before);
            assert!(!flags.is_regex);
            assert_eq!(flags.separator, "\n");
            assert_eq!(flags.files, vec!["-"]);
        }

        #[test]
        fn test_tac_flags_new_with_defaults() {
            let app = ct_app();
            let matches = app.try_get_matches_from(vec!["tac"]).unwrap();
            let flags = TacFlags::new(&matches).unwrap();
            assert!(!flags.is_before);
            assert!(!flags.is_regex);
            assert_eq!(flags.separator, "\n");
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
            assert_eq!(flags.separator, ":");
        }

        #[test]
        fn test_tac_flags_new_with_empty_separator() {
            let app = ct_app();
            let matches = app
                .try_get_matches_from(vec!["tac", "--separator", ""])
                .unwrap();
            let flags = TacFlags::new(&matches).unwrap();
            assert_eq!(flags.separator, "\0");
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
                    .get_one::<String>(tac_flags::TAC_SEPARATOR)
                    .map(String::as_str),
                Some(":")
            );
        }
    }

    #[cfg(test)]
    mod tac_error_tests {
        use super::*;
        use regex::Error as RegexError;

        #[test]
        fn test_tac_error_invalid_regex() {
            let regex_error = RegexError::Syntax("invalid regex".to_string());
            let error = TacError::InvalidRegex(regex_error);
            assert_eq!(
                error.to_string(),
                "invalid regular expression: invalid regex"
            );
            assert_eq!(error.code(), 1);
        }

        #[test]
        fn test_tac_error_invalid_argument() {
            let error = TacError::InvalidArgument("test.txt".to_string());
            assert_eq!(error.to_string(), "test.txt: read error: Invalid argument");
            assert_eq!(error.code(), 1);
        }

        #[test]
        fn test_tac_error_file_not_found() {
            let error = TacError::FileNotFound("test.txt".to_string());
            assert_eq!(
                error.to_string(),
                "failed to open 'test.txt' for reading: No such file or directory"
            );
            assert_eq!(error.code(), 1);
        }

        #[test]
        fn test_tac_error_read_error() {
            let io_error = std::io::Error::new(std::io::ErrorKind::Other, "read error");
            let error = TacError::ReadError("test.txt".to_string(), io_error);
            assert_eq!(
                error.to_string(),
                "failed to read from test.txt: read error"
            );
            assert_eq!(error.code(), 1);
        }

        #[test]
        fn test_tac_error_write_error() {
            let io_error = std::io::Error::new(std::io::ErrorKind::Other, "write error");
            let error = TacError::WriteError(io_error);
            assert_eq!(error.to_string(), "failed to write to stdout: write error");
            assert_eq!(error.code(), 1);
        }
    }

    #[cfg(test)]
    mod file_operations_tests {
        use super::*;
        use std::io::Write;
        use tempfile::NamedTempFile;

        #[test]
        fn test_read_from_file_success() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"test content").unwrap();
            let result = read_from_file(temp_file.path());
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), b"test content");
        }

        #[test]
        fn test_read_from_file_nonexistent() {
            let result = read_from_file(Path::new("nonexistent.txt"));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("failed to read from"));
        }

        #[test]
        fn test_validate_file_path_directory() {
            let result = validate_file_path(Path::new("."));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("Invalid argument"));
        }

        #[test]
        fn test_validate_file_path_nonexistent() {
            let result = validate_file_path(Path::new("nonexistent.txt"));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("No such file or directory"));
        }

        #[test]
        fn test_validate_file_path_valid() {
            let temp_file = NamedTempFile::new().unwrap();
            let result = validate_file_path(temp_file.path());
            assert!(result.is_ok());
        }

        #[test]
        #[ignore]
        fn test_get_file_data_stdin() {
            // This test is ignored because it requires real stdin
            // In a real environment, we would need integration tests
            // or a more sophisticated mock setup
            let result = get_file_data("-");
            assert!(result.is_ok());
        }

        #[test]
        fn test_get_file_data_regular_file() {
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"test content").unwrap();
            let result = get_file_data(temp_file.path().to_str().unwrap());
            assert!(result.is_ok());
            match result.unwrap() {
                FileData::Mapped(_) | FileData::Buffer(_) => (),
            }
        }

        #[test]
        fn test_get_file_data_nonexistent() {
            let result = get_file_data("nonexistent.txt");
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod tac_buffer_tests {
        use super::*;

        #[test]
        fn test_tac_buffer_simple() {
            let mut output = Vec::new();
            let data = b"line1\nline2\nline3";
            tac_buffer(&mut output, data, false, "\n").unwrap();
            assert_eq!(output, b"line3line2\nline1\n");
        }

        #[test]
        fn test_tac_buffer_before() {
            let mut output = Vec::new();
            let data = b"line1\nline2\nline3";
            tac_buffer(&mut output, data, true, "\n").unwrap();
            assert_eq!(output, b"line3\nline2\nline1");
        }

        #[test]
        fn test_tac_buffer_custom_separator() {
            let mut output = Vec::new();
            let data = b"line1:line2:line3";
            tac_buffer(&mut output, data, false, ":").unwrap();
            assert_eq!(output, b"line3line2:line1:");
        }

        #[test]
        fn test_tac_buffer_empty_input() {
            let mut output = Vec::new();
            let data = b"";
            tac_buffer(&mut output, data, false, "\n").unwrap();
            assert_eq!(output, b"");
        }

        #[test]
        fn test_tac_buffer_single_line() {
            let mut output = Vec::new();
            let data = b"single line";
            tac_buffer(&mut output, data, false, "\n").unwrap();
            assert_eq!(output, b"single line");
        }

        #[test]
        fn test_tac_buffer_with_trailing_separator() {
            let mut output = Vec::new();
            let data = b"line1\nline2\nline3\n";
            tac_buffer(&mut output, data, false, "\n").unwrap();
            assert_eq!(output, b"line3\nline2\nline1\n");
        }

        #[test]
        fn test_tac_buffer_with_multiple_separators() {
            let mut output = Vec::new();
            let data = b"line1\n\nline2\n\nline3";
            tac_buffer(&mut output, data, false, "\n").unwrap();
            assert_eq!(output, b"line3\nline2\n\nline1\n");
        }

        #[test]
        fn test_tac_buffer_with_empty_lines() {
            let mut output = Vec::new();
            let data = b"\n\n\n";
            tac_buffer(&mut output, data, false, "\n").unwrap();
            assert_eq!(output, b"\n\n\n");
        }

        #[test]
        fn test_tac_buffer_with_custom_multi_byte_separator() {
            let mut output = Vec::new();
            let data = b"line1<sep>line2<sep>line3";
            tac_buffer(&mut output, data, false, "<sep>").unwrap();
            assert_eq!(output, b"line3line2<sep>line1<sep>");
        }

        #[test]
        fn test_tac_buffer_with_no_separator() {
            let mut output = Vec::new();
            let data = b"content";
            tac_buffer(&mut output, data, false, "|").unwrap();
            assert_eq!(output, b"content");
        }
    }
}
