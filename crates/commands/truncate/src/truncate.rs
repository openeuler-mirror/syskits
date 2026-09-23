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

//! truncate 是一个 Linux 命令，用于修改文件的大小，它可以将文件的大小缩小或扩展到指定的大小。

extern crate rust_i18n;
use rust_i18n::t;
use std::fs::{File, OpenOptions, metadata};
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, ArgAction, Command, builder::OsStringValueParser, crate_version};
#[cfg(unix)]
use std::ffi::CStr;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::linux::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::path::Path;
use sys_locale::get_locale;

use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, CTsageError, CtSimpleError, FromIo, set_ct_exit_code};
use ctcore::ct_parse_size::{CtParser, ParseSizeError};
use ctcore::ct_posix::GnuGetoptCommandExt;
use ctcore::ct_quoting_style::{CtQuotes, CtQuotingStyle, escape_name, gnu_quote_shell};

use std::ffi::{OsStr, OsString};

#[derive(Debug, Eq, PartialEq)]
enum TruncateMode {
    Absolute(u64),
    Extend(u64),
    Reduce(u64),
    AtMost(u64),
    AtLeast(u64),
    RoundDown(u64),
    RoundUp(u64),
}

#[derive(Debug, Eq, PartialEq)]
enum TruncateSizeError {
    ExtendOverflow,
    BlockOverflow { blocks: String, block_size: u64 },
}

const TRUNCATE_SIZE_UNITS: &[&str] = &[
    "K", "k", "M", "m", "G", "g", "T", "t", "P", "E", "Z", "Y", "R", "Q", "KB", "kB", "MB", "mB",
    "GB", "gB", "TB", "tB", "PB", "EB", "ZB", "YB", "RB", "QB", "KiB", "kiB", "MiB", "miB", "GiB",
    "giB", "TiB", "tiB", "PiB", "EiB", "ZiB", "YiB", "RiB", "QiB",
];

impl TruncateMode {
    /// 根据这个截断模式计算目标文件的字节数。
    ///
    /// `fsize` 是参考文件的大小，以字节为单位。
    ///
    /// 如果模式是 [`TruncateMode::Reduce`] 并且要减去的值大于 `fsize`，那么该函数将返回0（因为它不能返回负数）。
    ///
    /// # 示例
    ///
    /// 将一个10字节的文件扩展5字节：
    ///
    /// ```rust,ignore
    /// let mode = TruncateMode::Extend(5);
    /// let fsize = 10;
    /// assert_eq!(mode.to_size(fsize), Ok(15));
    /// ```
    ///
    /// 如果减小的字节数超过文件的大小，结果将为0：
    ///
    /// ```rust,ignore
    /// let mode = TruncateMode::Reduce(5);
    /// let fsize = 3;
    /// assert_eq!(mode.to_size(fsize), Ok(0));
    /// ```
    fn to_size(&self, fsize: u64) -> Result<u64, TruncateSizeError> {
        let size = match self {
            Self::Absolute(size) => *size,
            Self::Extend(size) => checked_file_size_add(fsize, *size)?,
            Self::Reduce(size) => {
                if *size > fsize {
                    0
                } else {
                    fsize - size
                }
            }
            Self::AtMost(size) => fsize.min(*size),
            Self::AtLeast(size) => fsize.max(*size),
            Self::RoundDown(size) => fsize - fsize % size,
            Self::RoundUp(size) => {
                let mut rp = fsize % size;
                if rp != 0 {
                    rp = size - rp;
                }

                checked_file_size_add(fsize, rp)?
            }
        };

        Ok(size)
    }

    fn to_block_size(&self, fsize: u64, blocksize: u64) -> Result<u64, TruncateSizeError> {
        let blocks = match self {
            Self::Absolute(size)
            | Self::Extend(size)
            | Self::Reduce(size)
            | Self::AtMost(size)
            | Self::AtLeast(size)
            | Self::RoundDown(size)
            | Self::RoundUp(size) => *size,
        };
        let is_off_t_min = matches!(self, Self::Reduce(_)) && blocks == off_t_min_magnitude();
        let size = checked_block_size(blocks, blocksize, is_off_t_min)?;

        let target_size = match self {
            Self::Absolute(_) => size,
            Self::Extend(_) => checked_file_size_add(fsize, size)?,
            Self::Reduce(_) => fsize.saturating_sub(size),
            Self::AtMost(_) => fsize.min(size),
            Self::AtLeast(_) => fsize.max(size),
            Self::RoundDown(_) => fsize - fsize % size,
            Self::RoundUp(_) => {
                let mut rp = fsize % size;
                if rp != 0 {
                    rp = size - rp;
                }

                checked_file_size_add(fsize, rp)?
            }
        };

        Ok(target_size)
    }
}

fn checked_file_size_add(left: u64, right: u64) -> Result<u64, TruncateSizeError> {
    left.checked_add(right)
        .filter(|size| *size <= i64::MAX as u64)
        .ok_or(TruncateSizeError::ExtendOverflow)
}

fn off_t_min_magnitude() -> u64 {
    i64::MAX as u64 + 1
}

fn checked_block_size(
    blocks: u64,
    block_size: u64,
    is_off_t_min: bool,
) -> Result<u64, TruncateSizeError> {
    blocks
        .checked_mul(block_size)
        .filter(|size| *size <= i64::MAX as u64 || (is_off_t_min && *size == off_t_min_magnitude()))
        .ok_or_else(|| TruncateSizeError::BlockOverflow {
            blocks: if is_off_t_min {
                i64::MIN.to_string()
            } else {
                blocks.to_string()
            },
            block_size,
        })
}

fn truncate_size_error(filename: &OsStr, error: TruncateSizeError) -> Box<dyn CTError> {
    match error {
        TruncateSizeError::ExtendOverflow => CtSimpleError::new(
            1,
            format!(
                "overflow extending size of file {}",
                truncate_quote_operand(filename)
            ),
        ),
        TruncateSizeError::BlockOverflow { blocks, block_size } => CtSimpleError::new(
            1,
            format!(
                "overflow in {blocks} * {block_size} byte blocks for file {}",
                truncate_quote_operand(filename)
            ),
        ),
    }
}

fn truncate_quote_operand(operand: &OsStr) -> String {
    gnu_quote_shell(operand, true)
}

fn truncate_quote_size(size: &str) -> String {
    escape_name(
        OsStr::new(size),
        &CtQuotingStyle::C {
            quotes: CtQuotes::Single,
        },
    )
}

pub mod truncate_flags {
    pub const TRUNCATE_IO_BLOCKS: &str = "io-blocks";
    pub const TRUNCATE_NO_CREATE: &str = "no-create";
    pub const TRUNCATE_REFERENCE: &str = "reference";
    pub const TRUNCATE_SIZE: &str = "size";
    pub const TRUNCATE_ARG_FILES: &str = "files";
}

#[derive(Default)]
pub struct Truncate;
impl Tool for Truncate {
    fn name(&self) -> &'static str {
        "truncate"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        truncate_main(args.iter().cloned())
    }
}

pub fn truncate_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args).map_err(|e| {
        e.print().expect("Error writing clap::Error");
        match e.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => 0,
            _ => 1,
        }
    })?;

    let is_io_blocks = matches.get_flag(truncate_flags::TRUNCATE_IO_BLOCKS);
    let is_no_create = matches.get_flag(truncate_flags::TRUNCATE_NO_CREATE);
    let reference = matches
        .get_one::<OsString>(truncate_flags::TRUNCATE_REFERENCE)
        .cloned();
    let size = matches
        .get_one::<String>(truncate_flags::TRUNCATE_SIZE)
        .map(String::from);
    let files: Vec<OsString> = matches
        .get_many::<OsString>(truncate_flags::TRUNCATE_ARG_FILES)
        .map(|v| v.cloned().collect())
        .unwrap_or_default();

    if reference.is_none() && size.is_none() {
        return Err(CTsageError::new(
            1,
            "you must specify either '--size' or '--reference'",
        ));
    }
    if is_io_blocks && size.is_none() {
        return Err(CTsageError::new(
            1,
            "'--io-blocks' was specified but '--size' was not",
        ));
    }
    if files.is_empty() {
        return Err(CTsageError::new(1, "missing file operand"));
    }

    truncate(is_no_create, is_io_blocks, reference, size, &files)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("truncate.about");
    let usage_description = t!("truncate.usage");
    let args = vec![
        Arg::new(truncate_flags::TRUNCATE_IO_BLOCKS)
            .short('o')
            .long(truncate_flags::TRUNCATE_IO_BLOCKS)
            .help(
                "treat SIZE as the number of I/O blocks of the file rather than bytes \
            (NOT IMPLEMENTED)",
            )
            .action(ArgAction::SetTrue)
            .overrides_with(truncate_flags::TRUNCATE_IO_BLOCKS),
        Arg::new(truncate_flags::TRUNCATE_NO_CREATE)
            .short('c')
            .long(truncate_flags::TRUNCATE_NO_CREATE)
            .help(t!("truncate.clap.truncate_no_create"))
            .action(ArgAction::SetTrue)
            .overrides_with(truncate_flags::TRUNCATE_NO_CREATE),
        Arg::new(truncate_flags::TRUNCATE_REFERENCE)
            .short('r')
            .long(truncate_flags::TRUNCATE_REFERENCE)
            .help(t!("truncate.clap.truncate_reference"))
            .action(ArgAction::Set)
            .overrides_with(truncate_flags::TRUNCATE_REFERENCE)
            .value_parser(OsStringValueParser::new())
            .value_name("RFILE")
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(truncate_flags::TRUNCATE_SIZE)
            .short('s')
            .long(truncate_flags::TRUNCATE_SIZE)
            .help(
                "set or adjust the size of each file according to SIZE, which is in \
            bytes unless --io-blocks is specified",
            )
            .action(ArgAction::Set)
            .overrides_with(truncate_flags::TRUNCATE_SIZE)
            .value_name("SIZE")
            .allow_hyphen_values(true),
        Arg::new(truncate_flags::TRUNCATE_ARG_FILES)
            .value_name("FILE")
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .after_help(t!("truncate.after_help"))
        .args(args)
        .gnu_getopt()
}

/// 将指定文件截断到给定的大小。
///
/// 如果 `create` 为真，那么如果文件尚不存在，文件将会被创建。如果 `size` 大于文件中的字节数，文件将用零填充。
/// 如果 `size` 小于文件的字节数，文件将被截断，`size` 之后的任何字节都将丢失。
///
/// # 错误
///
/// 如果文件无法被打开，或者设置文件大小时出现错误。
fn truncate_file_with_size<P, F>(filename: P, create: bool, size_for_file: F) -> CTResult<()>
where
    P: AsRef<OsStr>,
    F: FnOnce(&File) -> CTResult<u64>,
{
    let filename = filename.as_ref();
    let path = Path::new(filename);
    let mut options = OpenOptions::new();
    options.write(true).create(create);
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);

    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if !create && error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error.map_err_context(|| {
                format!(
                    "cannot open {} for writing",
                    truncate_quote_operand(filename)
                )
            }));
        }
    };
    let size = size_for_file(&file)?;

    #[cfg(unix)]
    {
        let size = libc::off_t::try_from(size).map_err(|_| {
            CtSimpleError::new(
                1,
                format!(
                    "failed to truncate {} at {size} bytes",
                    truncate_quote_operand(filename)
                ),
            )
        })?;
        if unsafe { libc::ftruncate(file.as_raw_fd(), size) } != 0 {
            let error = std::io::Error::last_os_error();
            let error_message = os_error_message(&error);
            return Err(CtSimpleError::new(
                1,
                format!(
                    "failed to truncate {} at {size} bytes: {error_message}",
                    truncate_quote_operand(filename)
                ),
            ));
        }
        Ok(())
    }

    #[cfg(not(unix))]
    file.set_len(size).map_err_context(|| {
        format!(
            "failed to truncate {} at {size} bytes",
            truncate_quote_operand(filename)
        )
    })
}

fn truncate_file<P: AsRef<OsStr>>(filename: P, create: bool, size: u64) -> CTResult<()> {
    truncate_file_with_size(filename, create, |_| Ok(size))
}

#[cfg(unix)]
fn os_error_message(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(errno) => {
            // SAFETY: strerror returns a NUL-terminated message for a valid errno value.
            unsafe { CStr::from_ptr(libc::strerror(errno)) }
                .to_string_lossy()
                .into_owned()
        }
        None => error.to_string(),
    }
}

fn truncate_files<P, F>(filenames: &[P], mut truncate_one: F) -> CTResult<()>
where
    P: AsRef<OsStr>,
    F: FnMut(&OsStr) -> CTResult<()>,
{
    let mut failed = false;

    for filename in filenames {
        if let Err(error) = truncate_one(filename.as_ref()) {
            ctcore::ct_show!(error);
            failed = true;
        }
    }

    if failed {
        set_ct_exit_code(1);
    }

    Ok(())
}

#[cfg(unix)]
fn reference_file_size<R: AsRef<OsStr>>(reference: R) -> CTResult<u64> {
    let reference = reference.as_ref();
    let reference_path = Path::new(reference);
    let metadata = metadata(reference_path).map_err(|error| {
        CtSimpleError::new(
            1,
            format!(
                "cannot stat {}: {}",
                truncate_quote_operand(reference),
                os_error_message(&error)
            ),
        )
    })?;

    if metadata.file_type().is_file() {
        return Ok(metadata.len());
    }

    let file = File::open(reference_path).map_err(|error| {
        CtSimpleError::new(
            1,
            format!(
                "cannot get the size of {}: {}",
                truncate_quote_operand(reference),
                os_error_message(&error)
            ),
        )
    })?;
    let size = unsafe { libc::lseek(file.as_raw_fd(), 0, libc::SEEK_END) };
    if size < 0 {
        let error = std::io::Error::last_os_error();
        return Err(CtSimpleError::new(
            1,
            format!(
                "cannot get the size of {}: {}",
                truncate_quote_operand(reference),
                os_error_message(&error)
            ),
        ));
    }

    Ok(size as u64)
}

#[cfg(not(unix))]
fn reference_file_size<R: AsRef<OsStr>>(reference: R) -> CTResult<u64> {
    let reference = reference.as_ref();
    metadata(Path::new(reference))
        .map(|metadata| metadata.len())
        .map_err_context(|| format!("cannot stat {}", truncate_quote_operand(reference)))
}

/// 将文件截断到相对于给定文件的大小。
///
/// `r_file_name` 是参考文件的名称。
///
/// `size_string` 提供了相对于参考文件的大小，以设定目标文件的大小。
/// 例如，"+3K" 表示 "将每个文件设置为比参考文件大3千字节"。
///
/// 如果 `create` 为真，那么如果文件尚不存在，每个文件都将被创建。
///
/// # 错误
///
/// 如果有任何文件无法被打开，或者在设置至少一个文件的大小时出现问题。
///
/// 如果至少有一个文件是命名管道（也称为FIFO）。
fn truncate_reference_and_size<P, R>(
    r_file_name: R,
    size_string: &str,
    filenames: &[P],
    is_create: bool,
    is_block: bool,
) -> CTResult<()>
where
    P: AsRef<OsStr>,
    R: AsRef<OsStr>,
{
    let r_file_name = r_file_name.as_ref();
    let truncate_mode = match truncate_parse_mode_and_size(size_string) {
        Err(e) => {
            let err_massage = format!("Invalid number: {e}");
            return Err(CtSimpleError::new(1, err_massage));
        }
        Ok(TruncateMode::Absolute(_)) => {
            let err_massage =
                String::from("you must specify a relative '--size' with '--reference'");
            return Err(CtSimpleError::new(1, err_massage));
        }
        Ok(mode) => mode,
    };
    if let TruncateMode::RoundDown(0) | TruncateMode::RoundUp(0) = truncate_mode {
        return Err(CtSimpleError::new(1, "division by zero"));
    }
    let reference_size = reference_file_size(r_file_name)?;
    truncate_files(filenames, |filename| {
        if is_block {
            truncate_file_with_size(filename, is_create, |file| {
                let blocksize = file
                    .metadata()
                    .map_err_context(|| {
                        format!("cannot fstat {}", truncate_quote_operand(filename))
                    })?
                    .st_blksize();
                truncate_mode
                    .to_block_size(reference_size, blocksize)
                    .map_err(|error| truncate_size_error(filename, error))
            })
        } else {
            let target_size = truncate_mode
                .to_size(reference_size)
                .map_err(|error| truncate_size_error(filename, error))?;
            truncate_file(filename, is_create, target_size)
        }
    })
}

/// 将文件截断以匹配给定参考文件的大小。
///
/// `r_file_name` 是参考文件的名称。
///
/// 如果 `create` 为真，则如果文件尚不存在，每个文件都将被创建。
///
/// # 错误
///
/// 如果有任何文件无法被打开，或者在设置至少一个文件的大小时出现问题。
///
/// 如果至少有一个文件是命名管道（也称为FIFO）。
fn truncate_reference_file_only<P, R>(
    r_file_name: R,
    filenames: &[P],
    is_create: bool,
) -> CTResult<()>
where
    P: AsRef<OsStr>,
    R: AsRef<OsStr>,
{
    let r_file_name = r_file_name.as_ref();
    let target_size = reference_file_size(r_file_name)?;
    truncate_files(filenames, |filename| {
        truncate_file(filename, is_create, target_size)
    })
}

#[cfg(unix)]
fn io_block_size_for_new_file(filename: &OsStr) -> u64 {
    let path = Path::new(filename);
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));

    metadata(parent).map(|md| md.st_blksize()).unwrap_or(0)
}

#[cfg(not(unix))]
fn io_block_size_for_new_file(_filename: &OsStr) -> u64 {
    0
}

/// 将文件截断到指定的大小。
///
/// `size_string` 提供的是绝对大小或相对大小。相对大小会根据文件的当前大小调整每个文件的大小。
/// 例如，"3K" 表示 "将每个文件设置为3千字节"，而 "+3K" 表示 "将每个文件设置为其当前大小基础上增加3千字节"。
///
/// 如果 `create` 为真，那么如果文件不存在，每个文件都将被创建。
///
/// # 错误
///
/// 如果有任何文件无法打开，或者至少有一个文件设置大小时出现问题。
///
/// 如果至少有一个文件是命名管道（也称为fifo）。
fn truncate_size_only<P>(
    size_string: &str,
    filenames: &[P],
    is_create: bool,
    is_blocks: bool,
) -> CTResult<()>
where
    P: AsRef<OsStr>,
{
    let truncate_mode = truncate_parse_mode_and_size(size_string)
        .map_err(|e| CtSimpleError::new(1, format!("Invalid number: {e}")))?;
    if let TruncateMode::RoundDown(0) | TruncateMode::RoundUp(0) = truncate_mode {
        return Err(CtSimpleError::new(1, "division by zero"));
    }

    truncate_files(filenames, |filename| {
        let (f_size, blocksize) = match metadata(filename) {
            Ok(md) => {
                let blocksize_md = md.st_blksize();

                (md.len(), blocksize_md)
            }
            Err(_) => {
                if is_blocks && is_create {
                    (0, io_block_size_for_new_file(filename))
                } else {
                    (0, 0)
                }
            }
        };

        let t_size = match is_blocks {
            true => truncate_mode
                .to_block_size(f_size, blocksize)
                .map_err(|error| truncate_size_error(filename, error))?,
            false => truncate_mode
                .to_size(f_size)
                .map_err(|error| truncate_size_error(filename, error))?,
        };

        truncate_file(filename, is_create, t_size)
    })
}

fn truncate<P>(
    is_no_create: bool,
    is_io_blocks: bool,
    reference: Option<OsString>,
    size: Option<String>,
    filenames: &[P],
) -> CTResult<()>
where
    P: AsRef<OsStr>,
{
    if size.as_deref().is_some_and(has_multiple_relative_modifiers) {
        return Err(CTsageError::new(1, "multiple relative modifiers specified"));
    }

    let is_create = !is_no_create;
    // 存在四种可能的情况：
    // - 已给出参考文件且已给出大小，
    // - 已给出参考文件但未给出大小，
    // - 未给出参考文件但已给出大小，
    // - 既未给出参考文件也未给出大小，
    match (reference, size) {
        (Some(r_file_name), Some(size_string)) => truncate_reference_and_size(
            &r_file_name,
            &size_string,
            filenames,
            is_create,
            is_io_blocks,
        ),
        (Some(r_file_name), None) => {
            truncate_reference_file_only(&r_file_name, filenames, is_create)
        }
        (None, Some(size_string)) => {
            truncate_size_only(&size_string, filenames, is_create, is_io_blocks)
        }
        (None, None) => unreachable!(), // 这种情况现在不可能发生，因为它已经被clap处理了
    }
}

/// 判断一个字符是否是大小修饰符，如 '+' 或 '<'。
fn is_modifier(c: char) -> bool {
    c == '+' || c == '-' || c == '<' || c == '>' || c == '/' || c == '%'
}

fn has_multiple_relative_modifiers(size_string: &str) -> bool {
    let size_string =
        size_string.trim_start_matches(|character: char| character.is_ascii_whitespace());
    if !matches!(size_string.chars().next(), Some('<' | '>' | '/' | '%')) {
        return false;
    }

    let remaining =
        size_string[1..].trim_start_matches(|character: char| character.is_ascii_whitespace());
    matches!(remaining.chars().next(), Some('+' | '-'))
}

fn normalize_truncate_decimal_number(number: &str) -> String {
    let number_end = number
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(number.len());
    let digits = &number[..number_end];
    if digits.len() <= 1 || !digits.starts_with('0') {
        return number.to_string();
    }

    let normalized_digits = digits.trim_start_matches('0');
    let normalized_digits = if normalized_digits.is_empty() {
        "0"
    } else {
        normalized_digits
    };
    format!("{normalized_digits}{}", &number[number_end..])
}

/// 解析带有可选修饰符符号作为第一个字符的大小字符串。
///
/// 大小字符串的描述与 `parse_size_u64` 函数相同。`size_string` 的第一个字符可能是一个修饰符符号，
/// 如 '+' 或 '<'。此函数返回的元组的第一个元素表示存在的修饰符符号，
/// 如果不存在修饰符，则为 `TruncateMode::Absolute`。
///
/// # 错误情况
///
/// 如果 `size_string` 为空，或者无法从给定的字符串中解析出数字（例如，字符串为 "abc"）时，函数会引发恐慌（panic）。
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(parse_mode_and_size("+123"), (TruncateMode::Extend, 123));
/// ```
fn truncate_parse_mode_and_size(size_string: &str) -> Result<TruncateMode, ParseSizeError> {
    let size_string =
        size_string.trim_start_matches(|character: char| character.is_ascii_whitespace());
    let Some(modifier) = size_string.chars().next() else {
        return Err(ParseSizeError::ParseFailure(size_string.to_string()));
    };

    let (mode, number, displayed_size): (fn(u64) -> TruncateMode, &str, &str) =
        if is_modifier(modifier) {
            match modifier {
                '<' | '>' | '/' | '%' => {
                    let number = size_string[1..]
                        .trim_start_matches(|character: char| character.is_ascii_whitespace());
                    let mode = match modifier {
                        '<' => TruncateMode::AtMost,
                        '>' => TruncateMode::AtLeast,
                        '/' => TruncateMode::RoundDown,
                        '%' => TruncateMode::RoundUp,
                        _ => unreachable!(),
                    };
                    (mode, number, number)
                }
                '+' => (TruncateMode::Extend, &size_string[1..], size_string),
                '-' => (TruncateMode::Reduce, &size_string[1..], size_string),
                _ => unreachable!(),
            }
        } else {
            (TruncateMode::Absolute, size_string, size_string)
        };

    let invalid_number = || ParseSizeError::ParseFailure(truncate_quote_size(displayed_size));
    if number.ends_with('b')
        || number.starts_with("0x")
        || matches!(modifier, '+' | '-')
            && number
                .chars()
                .next()
                .is_some_and(|character| !character.is_ascii_digit())
    {
        return Err(invalid_number());
    }
    let mut parser = CtParser::default();
    parser.with_allow_list(TRUNCATE_SIZE_UNITS);
    let normalized_number = normalize_truncate_decimal_number(number);
    let size = parser
        .parse_u64(&normalized_number)
        .map_err(|error| match error {
            ParseSizeError::SizeTooBig(_) => ParseSizeError::SizeTooBig(format!(
                "{}: Value too large for defined data type",
                truncate_quote_size(displayed_size)
            )),
            _ => invalid_number(),
        })?;
    if size > i64::MAX as u64 && !(modifier == '-' && size == off_t_min_magnitude()) {
        return Err(ParseSizeError::SizeTooBig(format!(
            "{}: Value too large for defined data type",
            truncate_quote_size(displayed_size)
        )));
    }

    Ok(mode(size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code};
    use std::ffi::OsString;
    use std::fs::File;
    use std::sync::Mutex;

    static EXIT_CODE_LOCK: Mutex<()> = Mutex::new(());
    static POSIXLY_CORRECT_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_tool_implementation() {
        let tool = Truncate;

        // Test name method
        assert_eq!(tool.name(), "truncate");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("truncate"));

        // Test execute method with help flag (should work)
        let args = vec![OsString::from("truncate"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_posixly_correct_stops_option_parsing_after_file_operand() {
        let _guard = POSIXLY_CORRECT_LOCK.lock().unwrap();
        let previous = std::env::var_os("POSIXLY_CORRECT");
        unsafe { std::env::set_var("POSIXLY_CORRECT", "1") };

        let error = truncate_main(
            [
                OsString::from("truncate"),
                OsString::from("input"),
                OsString::from("-s"),
                OsString::from("7"),
            ]
            .into_iter(),
        )
        .unwrap_err();

        match previous {
            Some(value) => unsafe { std::env::set_var("POSIXLY_CORRECT", value) },
            None => unsafe { std::env::remove_var("POSIXLY_CORRECT") },
        }

        assert_eq!(
            error.to_string(),
            "you must specify either '--size' or '--reference'"
        );
    }

    #[cfg(test)]
    mod is_modifier_tests {
        use super::*;

        #[test]
        fn test_is_modifier() {
            // 测试所有有效的修饰符
            assert!(is_modifier('+'));
            assert!(is_modifier('-'));
            assert!(is_modifier('<'));
            assert!(is_modifier('>'));
            assert!(is_modifier('/'));
            assert!(is_modifier('%'));

            // 测试无效的修饰符
            assert!(!is_modifier('a'));
            assert!(!is_modifier('1'));
            assert!(!is_modifier(' '));
            assert!(!is_modifier('='));
            assert!(!is_modifier('!'));
            assert!(!is_modifier('@'));
        }

        #[test]
        fn test_is_modifier_edge_cases() {
            // 测试边界条件
            assert!(!is_modifier('\0')); // 空字符
            assert!(!is_modifier('\n')); // 换行字符
            assert!(!is_modifier('\t')); // 制表符
        }
    }
    #[test]
    fn invalid_size_diagnostic_uses_gnu_shell_always_quotes() {
        let error = truncate_parse_mode_and_size("1\tK").unwrap_err();

        assert_eq!(error.to_string(), "'1\\tK'");
    }

    #[cfg(unix)]
    #[test]
    fn path_diagnostic_uses_gnu_quoteaf_segments() {
        use std::os::unix::ffi::OsStrExt;

        assert_eq!(
            truncate_quote_operand(OsStr::from_bytes(b"line\nbreak")),
            "'line'$'\\n''break'"
        );
    }

    #[cfg(test)]
    mod truncate_tests {
        use std::fs::metadata;
        use std::io::Write;

        use tempfile::NamedTempFile;

        use super::*;

        #[test]
        fn test_truncate_with_reference_and_size() {
            // 创建参考文件并写入一些数据
            let mut reference_file = NamedTempFile::new().unwrap();
            writeln!(reference_file, "Hello, world!").unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 使用参考文件和相对大小调整目标文件的大小
            truncate(
                false,
                false,
                Some(reference_file_path.clone().into()),
                Some("+5".to_string()),
                &target_files,
            )
            .unwrap();
            assert_eq!(metadata(&target_file_path).unwrap().len(), 19); // "Hello, world!" 长度 + 5

            truncate(
                false,
                false,
                Some(reference_file_path.clone().into()),
                Some("-3".to_string()),
                &target_files,
            )
            .unwrap();
            assert_eq!(metadata(&target_file_path).unwrap().len(), 11); // "Hello, world!" 长度 - 3
        }

        #[test]
        fn test_truncate_with_reference_only() {
            // 创建参考文件并写入一些数据
            let mut reference_file = NamedTempFile::new().unwrap();
            writeln!(reference_file, "Hello, world!").unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 使用参考文件调整目标文件的大小
            truncate(
                false,
                false,
                Some(reference_file_path.clone().into()),
                None,
                &target_files,
            )
            .unwrap();
            assert_eq!(metadata(&target_file_path).unwrap().len(), 14); // "Hello, world!" 的长度是 13
        }

        #[test]
        fn test_truncate_with_size_only() {
            // 创建目标文件并写入一些数据
            let mut target_file1 = NamedTempFile::new().unwrap();
            writeln!(target_file1, "Target file 1").unwrap();
            let target_file1_path = target_file1.path().to_str().unwrap().to_string();

            let mut target_file2 = NamedTempFile::new().unwrap();
            writeln!(target_file2, "Target file 2").unwrap();
            let target_file2_path = target_file2.path().to_str().unwrap().to_string();

            let target_files = vec![target_file1_path.clone(), target_file2_path.clone()];

            // 使用绝对大小进行调整
            truncate(false, false, None, Some("10".to_string()), &target_files).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 10);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 10);
        }

        #[test]
        fn test_truncate_rejects_multiple_relative_modifiers() {
            let error = truncate(
                false,
                false,
                None,
                Some(">+0".to_string()),
                &["multiple-relative-modifier-target"],
            )
            .unwrap_err();

            assert_eq!(error.to_string(), "multiple relative modifiers specified");
        }

        #[test]
        fn test_truncate_no_create() {
            // 文件不存在的情况
            let non_existent_file = "test_truncate_no_create";
            let target_files = vec![non_existent_file.to_string()];

            // 设置 create 为 false
            let result = truncate(true, false, None, Some("+5".to_string()), &target_files);
            assert!(result.is_ok());
            assert!(!std::path::Path::new(non_existent_file).exists());

            // 设置 create 为 true
            truncate(false, false, None, Some("+5".to_string()), &target_files).unwrap();
            assert_eq!(metadata(non_existent_file).unwrap().len(), 5);

            // 清理
            std::fs::remove_file(non_existent_file).unwrap();
        }

        #[test]
        fn test_truncate_errors() {
            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 测试无效大小字符串
            let result = truncate(
                false,
                false,
                None,
                Some("invalid".to_string()),
                &target_files,
            );
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("Invalid number"));

            // 测试参考文件不存在的情况
            let result = truncate(
                false,
                false,
                Some("test_truncate_errors2".into()),
                Some("+5".to_string()),
                &target_files,
            );
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("cannot stat"));

            // 测试除以零的情况
            let result = truncate(false, false, None, Some("/0".to_string()), &target_files);
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("division by zero"));
        }
    }
    #[cfg(test)]
    mod truncate_size_only_tests {
        use std::fs::metadata;
        use std::io::Write;
        #[cfg(unix)]
        use std::os::linux::fs::MetadataExt;

        use tempfile::NamedTempFile;

        use super::*;

        #[test]
        fn test_truncate_size_only() {
            // 创建目标文件并写入一些数据
            let mut target_file1 = NamedTempFile::new().unwrap();
            writeln!(target_file1, "Target file 1").unwrap();
            let target_file1_path = target_file1.path().to_str().unwrap().to_string();

            let mut target_file2 = NamedTempFile::new().unwrap();
            writeln!(target_file2, "Target file 2").unwrap();
            let target_file2_path = target_file2.path().to_str().unwrap().to_string();

            let target_files = vec![target_file1_path.clone(), target_file2_path.clone()];

            // 测试相对调整大小：Extend
            truncate_size_only("+5", &target_files, true, false).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 19);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 19);

            // 测试相对调整大小：Reduce
            truncate_size_only("-3", &target_files, true, false).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 16);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 16);

            // 测试相对调整大小：AtMost
            truncate_size_only("<8", &target_files, true, false).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 8);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 8);

            // 测试相对调整大小：AtLeast
            truncate_size_only(">20", &target_files, true, false).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 20);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 20);

            // 测试相对调整大小：RoundDown
            truncate_size_only("/4", &target_files, true, false).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 20); // 20 already multiple of 4
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 20); // 20 already multiple of 4

            // 测试相对调整大小：RoundUp
            truncate_size_only("%3", &target_files, true, false).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 21); // next multiple of 3
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 21); // next multiple of 3
        }

        #[test]
        fn test_truncate_size_only_errors() {
            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 测试无效大小字符串
            let result = truncate_size_only("invalid", &target_files, true, false);
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("Invalid number"));

            // 测试除以零的情况
            let result = truncate_size_only("/0", &target_files, true, false);
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("division by zero"));

            let result = truncate_size_only("%0", &target_files, true, false);
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("division by zero"));
        }

        #[test]
        fn test_truncate_size_only_no_create() {
            // 文件不存在的情况
            let non_existent_file = "test_truncate_size_only_no_create";
            let target_files = vec![non_existent_file.to_string()];

            // 设置 create 为 false
            let result = truncate_size_only("+5", &target_files, false, false);
            assert!(result.is_ok());
            assert!(!std::path::Path::new(non_existent_file).exists());

            // 设置 create 为 true
            truncate_size_only("+5", &target_files, true, false).unwrap();
            assert_eq!(metadata(non_existent_file).unwrap().len(), 5);

            // 清理
            std::fs::remove_file(non_existent_file).unwrap();
        }

        #[test]
        fn test_truncate_size_only_io_blocks_creates_missing_file_with_parent_block_size() {
            let temp_dir = tempfile::tempdir().unwrap();
            let target_path = temp_dir.path().join("new-file");
            let target = target_path.to_str().unwrap().to_string();
            let target_files = vec![target.clone()];

            truncate_size_only("1", &target_files, true, true).unwrap();

            #[cfg(unix)]
            let expected_size = metadata(temp_dir.path()).unwrap().st_blksize();
            #[cfg(not(unix))]
            let expected_size = 0;

            assert_eq!(metadata(&target).unwrap().len(), expected_size);
        }

        #[test]
        fn test_truncate_size_only_io_blocks_no_create_keeps_missing_file_absent() {
            let temp_dir = tempfile::tempdir().unwrap();
            let target_path = temp_dir.path().join("new-file");
            let target = target_path.to_str().unwrap().to_string();
            let target_files = vec![target.clone()];

            truncate_size_only("1", &target_files, false, true).unwrap();

            assert!(!target_path.exists());
        }

        #[test]
        fn test_truncate_size_only_zero_length() {
            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 使用绝对大小进行调整（零长度）
            truncate_size_only("0", &target_files, true, false).unwrap();
            assert_eq!(metadata(&target_file_path).unwrap().len(), 0);
        }

        #[test]
        fn test_truncate_size_only_multiple_files() {
            // 创建多个目标文件并写入一些数据
            let mut target_file1 = NamedTempFile::new().unwrap();
            writeln!(target_file1, "Target file 1").unwrap();
            let target_file1_path = target_file1.path().to_str().unwrap().to_string();

            let mut target_file2 = NamedTempFile::new().unwrap();
            writeln!(target_file2, "Target file 2").unwrap();
            let target_file2_path = target_file2.path().to_str().unwrap().to_string();

            let target_files = vec![target_file1_path.clone(), target_file2_path.clone()];

            // 使用绝对大小进行调整
            truncate_size_only("10", &target_files, true, false).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 10);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 10);
        }

        #[test]
        fn test_truncate_size_only_continues_after_file_error() {
            let _exit_code_guard = EXIT_CODE_LOCK.lock().unwrap();
            set_ct_exit_code(0);

            let temp_dir = tempfile::tempdir().unwrap();
            let missing = temp_dir.path().join("missing").join("child");
            let valid = temp_dir.path().join("valid");
            std::fs::write(&valid, b"x").unwrap();
            let target_files = vec![
                missing.to_str().unwrap().to_string(),
                valid.to_str().unwrap().to_string(),
            ];

            assert!(truncate_size_only("7", &target_files, true, false).is_ok());
            assert_eq!(metadata(&valid).unwrap().len(), 7);
            assert_eq!(get_ct_exit_code(), 1);

            set_ct_exit_code(0);
        }
    }

    #[cfg(test)]
    mod truncate_reference_file_only_tests {
        use std::fs::metadata;
        use std::io::Write;

        use tempfile::NamedTempFile;

        use super::*;

        #[cfg(target_os = "linux")]
        #[test]
        fn test_truncate_reference_directory_uses_seek_fallback() {
            let _exit_code_guard = EXIT_CODE_LOCK.lock().unwrap();
            set_ct_exit_code(0);

            let temp_dir = tempfile::tempdir().unwrap();
            let reference_directory = temp_dir.path().join("reference-directory");
            std::fs::create_dir(&reference_directory).unwrap();
            let target = temp_dir.path().join("target");
            std::fs::write(&target, b"target").unwrap();

            truncate_reference_file_only(&reference_directory, &[&target], false).unwrap();

            assert_eq!(metadata(&target).unwrap().len(), 6);
            assert_eq!(get_ct_exit_code(), 1);
            set_ct_exit_code(0);
        }

        #[test]
        fn test_truncate_reference_file_only() {
            // 创建参考文件并写入一些数据
            let mut reference_file = NamedTempFile::new().unwrap();
            writeln!(reference_file, "Hello, world!").unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 创建多个目标文件并写入一些数据
            let mut target_file1 = NamedTempFile::new().unwrap();
            writeln!(target_file1, "Target file 1").unwrap();
            let target_file1_path = target_file1.path().to_str().unwrap().to_string();

            let mut target_file2 = NamedTempFile::new().unwrap();
            writeln!(target_file2, "Target file 2").unwrap();
            let target_file2_path = target_file2.path().to_str().unwrap().to_string();

            let target_files = vec![target_file1_path.clone(), target_file2_path.clone()];

            // 使用参考文件来调整目标文件的大小
            truncate_reference_file_only(&reference_file_path, &target_files, true).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 14); // "Hello, world!" 的长度是 13
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 14);
        }

        #[test]
        fn test_truncate_reference_file_only_no_create() {
            // 创建参考文件并写入一些数据
            let mut reference_file = NamedTempFile::new().unwrap();
            writeln!(reference_file, "Hello, world!").unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 文件不存在的情况
            let non_existent_file = "test_truncate_reference_file_only_no_create";
            let target_files = vec![non_existent_file.to_string()];

            // 设置 create 为 false
            let result = truncate_reference_file_only(&reference_file_path, &target_files, false);
            assert!(result.is_ok());

            // 设置 create 为 true
            truncate_reference_file_only(&reference_file_path, &target_files, true).unwrap();
            assert_eq!(metadata(non_existent_file).unwrap().len(), 14); // "Hello, world!" 的长度是 13

            // 清理
            std::fs::remove_file(non_existent_file).unwrap();
        }

        #[test]
        fn test_truncate_reference_file_only_empty_reference() {
            // 创建一个零长度的参考文件
            let reference_file = NamedTempFile::new().unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 使用零长度的参考文件进行调整大小
            truncate_reference_file_only(&reference_file_path, &target_files, true).unwrap();
            assert_eq!(metadata(&target_file_path).unwrap().len(), 0);
        }

        #[test]
        fn test_truncate_reference_file_only_multiple_files() {
            // 创建参考文件并写入一些数据
            let mut reference_file = NamedTempFile::new().unwrap();
            writeln!(reference_file, "Hello, world!").unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 创建多个目标文件并写入一些数据
            let mut target_file1 = NamedTempFile::new().unwrap();
            writeln!(target_file1, "Target file 1").unwrap();
            let target_file1_path = target_file1.path().to_str().unwrap().to_string();

            let mut target_file2 = NamedTempFile::new().unwrap();
            writeln!(target_file2, "Target file 2").unwrap();
            let target_file2_path = target_file2.path().to_str().unwrap().to_string();

            let target_files = vec![target_file1_path.clone(), target_file2_path.clone()];

            // 使用参考文件来调整多个目标文件的大小
            truncate_reference_file_only(&reference_file_path, &target_files, true).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 14); // "Hello, world!" 的长度是 13
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 14);
        }

        #[test]
        fn test_truncate_reference_file_only_reference_not_found() {
            // 参考文件不存在的情况
            let reference_file_path = "test_truncate_reference_file_only_reference_not_found";

            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 尝试使用不存在的参考文件进行调整大小
            let result = truncate_reference_file_only(reference_file_path, &target_files, true);
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("cannot stat"));
        }
    }

    #[cfg(test)]
    mod truncate_reference_and_size_tests {
        use std::io::Write;

        use tempfile::NamedTempFile;

        use super::*;

        #[cfg(target_os = "linux")]
        #[test]
        fn test_truncate_reference_and_size_io_blocks_uses_target_block_size() {
            let mut target_file = NamedTempFile::new().unwrap();
            target_file.write_all(b"target").unwrap();

            truncate_reference_and_size("/proc/cpuinfo", "+1", &[target_file.path()], false, true)
                .unwrap();

            let target_metadata = metadata(target_file.path()).unwrap();
            assert_eq!(target_metadata.len(), target_metadata.st_blksize());
        }

        #[test]
        fn test_truncate_reference_and_size() {
            // 创建参考文件并写入一些数据
            let mut reference_file = NamedTempFile::new().unwrap();
            writeln!(reference_file, "Hello, world!").unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 创建目标文件并写入一些数据
            let mut target_file1 = NamedTempFile::new().unwrap();
            writeln!(target_file1, "Target file 1").unwrap();
            let target_file1_path = target_file1.path().to_str().unwrap().to_string();

            let mut target_file2 = NamedTempFile::new().unwrap();
            writeln!(target_file2, "Target file 2").unwrap();
            let target_file2_path = target_file2.path().to_str().unwrap().to_string();

            let target_files = vec![target_file1_path.clone(), target_file2_path.clone()];

            // 测试相对调整大小：Extend
            truncate_reference_and_size(&reference_file_path, "+5", &target_files, true, false)
                .unwrap();
            assert_eq!(std::fs::metadata(&target_file1_path).unwrap().len(), 19);
            assert_eq!(std::fs::metadata(&target_file2_path).unwrap().len(), 19);

            // 测试相对调整大小：Reduce
            truncate_reference_and_size(&reference_file_path, "-3", &target_files, true, false)
                .unwrap();
            assert_eq!(std::fs::metadata(&target_file1_path).unwrap().len(), 11);
            assert_eq!(std::fs::metadata(&target_file2_path).unwrap().len(), 11);

            // 测试相对调整大小：AtMost
            truncate_reference_and_size(&reference_file_path, "<8", &target_files, true, false)
                .unwrap();
            assert_eq!(std::fs::metadata(&target_file1_path).unwrap().len(), 8);
            assert_eq!(std::fs::metadata(&target_file2_path).unwrap().len(), 8);

            // 测试相对调整大小：AtLeast
            truncate_reference_and_size(&reference_file_path, ">20", &target_files, true, false)
                .unwrap();
            assert_eq!(std::fs::metadata(&target_file1_path).unwrap().len(), 20);
            assert_eq!(std::fs::metadata(&target_file2_path).unwrap().len(), 20);

            // 测试相对调整大小：RoundDown
            truncate_reference_and_size(&reference_file_path, "/4", &target_files, true, false)
                .unwrap();
            assert_eq!(std::fs::metadata(&target_file1_path).unwrap().len(), 12);
            assert_eq!(std::fs::metadata(&target_file2_path).unwrap().len(), 12);

            // 测试相对调整大小：RoundUp
            truncate_reference_and_size(&reference_file_path, "%3", &target_files, true, false)
                .unwrap();
            assert_eq!(std::fs::metadata(&target_file1_path).unwrap().len(), 15);
            assert_eq!(std::fs::metadata(&target_file2_path).unwrap().len(), 15);
        }

        #[test]
        fn test_truncate_reference_and_size_errors() {
            // 创建参考文件并写入一些数据
            let mut reference_file = NamedTempFile::new().unwrap();
            writeln!(reference_file, "Hello, world!").unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 测试无效大小字符串
            let result = truncate_reference_and_size(
                &reference_file_path,
                "invalid",
                &target_files,
                true,
                false,
            );
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("Invalid number"));

            // 测试绝对大小与参考文件组合
            let result = truncate_reference_and_size(
                &reference_file_path,
                "100",
                &target_files,
                true,
                false,
            );
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(
                error_message.contains("you must specify a relative '--size' with '--reference'")
            );

            // 测试除以零的情况
            let result =
                truncate_reference_and_size(&reference_file_path, "/0", &target_files, true, false);
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("division by zero"));
        }
        #[test]
        fn test_truncate_reference_and_size_no_create() {
            // 创建参考文件并写入一些数据
            let mut reference_file = NamedTempFile::new().unwrap();
            writeln!(reference_file, "Hello, world!").unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 文件不存在的情况
            let non_existent_file = "test_truncate_reference_and_size_no_create";
            let target_files = vec![non_existent_file.to_string()];

            // 设置 create 为 false
            let result = truncate_reference_and_size(
                &reference_file_path,
                "+5",
                &target_files,
                false,
                false,
            );
            assert!(result.is_ok());

            // 设置 create 为 true
            truncate_reference_and_size(&reference_file_path, "+5", &target_files, true, false)
                .unwrap();
            assert_eq!(metadata(non_existent_file).unwrap().len(), 19);

            // 清理
            std::fs::remove_file(non_existent_file).unwrap();
        }

        #[test]
        fn test_truncate_reference_and_size_zero_length() {
            // 创建一个零长度的参考文件
            let reference_file = NamedTempFile::new().unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 使用零长度的参考文件进行调整大小
            truncate_reference_and_size(&reference_file_path, "+5", &target_files, true, false)
                .unwrap();
            assert_eq!(metadata(&target_file_path).unwrap().len(), 5);
        }

        #[test]
        fn test_truncate_reference_and_size_multiple_files() {
            // 创建参考文件并写入一些数据
            let mut reference_file = NamedTempFile::new().unwrap();
            writeln!(reference_file, "Hello, world!").unwrap();
            let reference_file_path = reference_file.path().to_str().unwrap().to_string();

            // 创建多个目标文件并写入一些数据
            let mut target_file1 = NamedTempFile::new().unwrap();
            writeln!(target_file1, "Target file 1").unwrap();
            let target_file1_path = target_file1.path().to_str().unwrap().to_string();

            let mut target_file2 = NamedTempFile::new().unwrap();
            writeln!(target_file2, "Target file 2").unwrap();
            let target_file2_path = target_file2.path().to_str().unwrap().to_string();

            let target_files = vec![target_file1_path.clone(), target_file2_path.clone()];

            // 使用参考文件和大小字符串调整多个文件的大小
            truncate_reference_and_size(&reference_file_path, "-3", &target_files, true, false)
                .unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 11);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 11);
        }
    }
    #[cfg(test)]
    mod truncate_file_tests {
        use std::io::Write;

        use tempfile::NamedTempFile;

        use super::*;

        #[test]
        fn test_truncate_file() {
            let mut temp_file = NamedTempFile::new().unwrap();
            let temp_path = temp_file.path().to_str().unwrap().to_string();

            // 向文件中写入一些数据
            writeln!(temp_file, "Hello, world!").unwrap();

            // 将文件截断到更小的大小
            truncate_file(&temp_path, true, 5).unwrap();
            let metadata = std::fs::metadata(&temp_path).unwrap();
            assert_eq!(metadata.len(), 5);

            // 扩展文件到更大的大小
            truncate_file(&temp_path, true, 20).unwrap();
            let metadata = std::fs::metadata(&temp_path).unwrap();
            assert_eq!(metadata.len(), 20);

            // 尝试截断一个不存在的文件，且创建标志设置为 false
            let non_existent_file = "test_truncate_file";
            assert!(truncate_file(non_existent_file, false, 10).is_ok());
            assert!(!std::path::Path::new(non_existent_file).exists());

            // 使用创建标志设置为 true 截断一个不存在的文件
            assert!(truncate_file(non_existent_file, true, 10).is_ok());
            assert!(std::path::Path::new(non_existent_file).exists());
            let metadata = std::fs::metadata(non_existent_file).unwrap();
            assert_eq!(metadata.len(), 10);

            // Clean up
            std::fs::remove_file(non_existent_file).unwrap();
        }

        #[test]
        fn test_truncate_file_fifo() {
            #[cfg(unix)]
            {
                use std::process::Command;

                let temp_dir = tempfile::tempdir().unwrap();
                let fifo_path = temp_dir.path().join("fifo");
                Command::new("mkfifo").arg(&fifo_path).status().unwrap();

                let result = truncate_file(fifo_path.to_str().unwrap(), true, 10);
                assert!(result.is_err());
                let error_message = format!("{}", result.unwrap_err());
                assert!(error_message.contains("failed to truncate"));
                assert!(error_message.contains("at 10 bytes"));
                assert!(error_message.contains("Invalid argument"));
                assert!(!error_message.contains("os error"));
            }
        }

        #[test]
        fn test_truncate_file_reports_ftruncate_error_for_character_device() {
            #[cfg(unix)]
            {
                let result = truncate_file("/dev/null", true, 0);
                assert!(result.is_err());
                let error_message = format!("{}", result.unwrap_err());
                assert!(error_message.contains("failed to truncate"));
                assert!(error_message.contains("at 0 bytes"));
                assert!(error_message.contains("Invalid argument"));
                assert!(!error_message.contains("os error"));
            }
        }

        #[test]
        fn test_truncate_file_no_permission() {
            #[cfg(unix)]
            {
                use std::fs::set_permissions;
                use std::os::unix::fs::PermissionsExt;

                if ctcore::ct_process::geteuid() != 0 {
                    println!("Skipping test_truncate_file_no_permission: requires root privileges");
                    return;
                }
                let mut temp_file = NamedTempFile::new().unwrap();
                let temp_path = temp_file.path().to_str().unwrap().to_string();
                writeln!(temp_file, "Hello, world!").unwrap();

                // Remove write permission
                let mut permissions = std::fs::metadata(&temp_path).unwrap().permissions();
                permissions.set_mode(0o444); // Read-only
                set_permissions(&temp_path, permissions.clone()).unwrap();

                // Attempt to truncate the file
                let result = truncate_file(&temp_path, true, 5);
                assert!(result.is_ok());

                // Restore permissions for cleanup
                permissions.set_mode(0o644);
                set_permissions(&temp_path, permissions).unwrap();
            }
        }
    }

    #[cfg(test)]
    mod truncate_mode_to_size_tests {
        use crate::{TruncateMode, TruncateSizeError};

        #[test]
        fn test_truncate_mode_to_size() {
            // Absolute mode
            assert_eq!(TruncateMode::Absolute(100).to_size(50), Ok(100));

            // Extend mode
            assert_eq!(TruncateMode::Extend(50).to_size(100), Ok(150));

            // Reduce mode
            assert_eq!(TruncateMode::Reduce(50).to_size(100), Ok(50));
            assert_eq!(TruncateMode::Reduce(150).to_size(100), Ok(0));

            // AtMost mode
            assert_eq!(TruncateMode::AtMost(75).to_size(100), Ok(75));
            assert_eq!(TruncateMode::AtMost(150).to_size(100), Ok(100));

            // AtLeast mode
            assert_eq!(TruncateMode::AtLeast(150).to_size(100), Ok(150));
            assert_eq!(TruncateMode::AtLeast(75).to_size(100), Ok(100));

            // RoundDown mode
            assert_eq!(TruncateMode::RoundDown(50).to_size(123), Ok(100));
            assert_eq!(TruncateMode::RoundDown(1).to_size(123), Ok(123)); // Edge case

            // RoundUp mode
            assert_eq!(TruncateMode::RoundUp(50).to_size(123), Ok(150));
            assert_eq!(TruncateMode::RoundUp(1).to_size(123), Ok(123)); // Edge case
        }

        #[test]
        fn test_to_size() {
            assert_eq!(TruncateMode::Extend(5).to_size(10), Ok(15));
            assert_eq!(TruncateMode::Reduce(5).to_size(10), Ok(5));
            assert_eq!(TruncateMode::Reduce(5).to_size(3), Ok(0));
        }

        #[test]
        fn test_to_size_rejects_extension_above_off_t_max() {
            assert_eq!(
                TruncateMode::Extend(i64::MAX as u64).to_size(1),
                Err(TruncateSizeError::ExtendOverflow)
            );
        }

        #[test]
        fn test_to_block_size_rejects_product_above_off_t_max() {
            assert_eq!(
                TruncateMode::Absolute(1_152_921_504_606_846_976).to_block_size(0, 4096),
                Err(TruncateSizeError::BlockOverflow {
                    blocks: "1152921504606846976".to_string(),
                    block_size: 4096,
                })
            );
        }

        #[test]
        fn test_to_block_size_reports_off_t_min_as_negative() {
            assert_eq!(
                TruncateMode::Reduce(9_223_372_036_854_775_808).to_block_size(0, 4096),
                Err(TruncateSizeError::BlockOverflow {
                    blocks: "-9223372036854775808".to_string(),
                    block_size: 4096,
                })
            );
        }

        #[test]
        fn test_to_block_size_uses_block_bytes_for_relative_modes() {
            assert_eq!(TruncateMode::Extend(1).to_block_size(4096, 4096), Ok(8192));
            assert_eq!(TruncateMode::Reduce(1).to_block_size(8192, 4096), Ok(4096));
        }
    }
    #[cfg(test)]
    mod parse_mode_and_size_tests {
        use crate::TruncateMode;
        use crate::truncate_parse_mode_and_size;

        use super::*;

        #[test]
        fn test_truncate_parse_mode_and_size_rejects_dd_block_suffix() {
            assert!(truncate_parse_mode_and_size("1b").is_err());
        }

        #[test]
        fn test_truncate_parse_mode_and_size_rejects_hexadecimal_values() {
            assert_eq!(
                truncate_parse_mode_and_size("0x10"),
                Err(ParseSizeError::ParseFailure("'0x10'".to_string()))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_rejects_unsupported_lowercase_units() {
            for size in [
                "1p", "1pB", "1piB", "1e", "1eB", "1eiB", "1z", "1zB", "1ziB", "1y", "1yB", "1yiB",
            ] {
                assert_eq!(
                    truncate_parse_mode_and_size(size),
                    Err(ParseSizeError::ParseFailure(format!("'{size}'")))
                );
            }
        }

        #[test]
        fn test_truncate_parse_mode_and_size_treats_leading_zeroes_as_decimal() {
            assert_eq!(
                truncate_parse_mode_and_size("010K"),
                Ok(TruncateMode::Absolute(10 * 1024))
            );
            assert_eq!(
                truncate_parse_mode_and_size("00K"),
                Ok(TruncateMode::Absolute(0))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_rejects_unit_only_relative_sizes() {
            for size in ["+K", "-K", "+kB", "-KiB"] {
                assert_eq!(
                    truncate_parse_mode_and_size(size),
                    Err(ParseSizeError::ParseFailure(format!("'{size}'")))
                );
            }
        }

        #[test]
        fn test_truncate_parse_mode_and_size_uses_gnu_whitespace_rules() {
            assert_eq!(
                truncate_parse_mode_and_size("+1 "),
                Err(ParseSizeError::ParseFailure("'+1 '".to_string()))
            );
            assert_eq!(
                truncate_parse_mode_and_size("< 1"),
                Ok(TruncateMode::AtMost(1))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_rejects_values_above_off_t_max() {
            assert!(truncate_parse_mode_and_size("9223372036854775808").is_err());
        }

        #[test]
        fn test_truncate_parse_mode_and_size_accepts_off_t_min_for_reduction() {
            assert_eq!(
                truncate_parse_mode_and_size("-9223372036854775808"),
                Ok(TruncateMode::Reduce(9_223_372_036_854_775_808))
            );
        }

        #[test]
        fn test_parse_mode_and_size() {
            assert_eq!(
                truncate_parse_mode_and_size("10"),
                Ok(TruncateMode::Absolute(10))
            );
            assert_eq!(
                truncate_parse_mode_and_size("+10"),
                Ok(TruncateMode::Extend(10))
            );
            assert_eq!(
                truncate_parse_mode_and_size("-10"),
                Ok(TruncateMode::Reduce(10))
            );
            assert_eq!(
                truncate_parse_mode_and_size("<10"),
                Ok(TruncateMode::AtMost(10))
            );
            assert_eq!(
                truncate_parse_mode_and_size(">10"),
                Ok(TruncateMode::AtLeast(10))
            );
            assert_eq!(
                truncate_parse_mode_and_size("/10"),
                Ok(TruncateMode::RoundDown(10))
            );
            assert_eq!(
                truncate_parse_mode_and_size("%10"),
                Ok(TruncateMode::RoundUp(10))
            );
        }
        #[test]
        fn test_truncate_parse_mode_and_size_absolute() {
            assert_eq!(
                truncate_parse_mode_and_size("100"),
                Ok(TruncateMode::Absolute(100))
            );
            assert_eq!(
                truncate_parse_mode_and_size("0"),
                Ok(TruncateMode::Absolute(0))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_extend() {
            assert_eq!(
                truncate_parse_mode_and_size("+50"),
                Ok(TruncateMode::Extend(50))
            );
            assert_eq!(
                truncate_parse_mode_and_size("+0"),
                Ok(TruncateMode::Extend(0))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_reduce() {
            assert_eq!(
                truncate_parse_mode_and_size("-30"),
                Ok(TruncateMode::Reduce(30))
            );
            assert_eq!(
                truncate_parse_mode_and_size("-0"),
                Ok(TruncateMode::Reduce(0))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_at_most() {
            assert_eq!(
                truncate_parse_mode_and_size("<200"),
                Ok(TruncateMode::AtMost(200))
            );
            assert_eq!(
                truncate_parse_mode_and_size("<0"),
                Ok(TruncateMode::AtMost(0))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_at_least() {
            assert_eq!(
                truncate_parse_mode_and_size(">300"),
                Ok(TruncateMode::AtLeast(300))
            );
            assert_eq!(
                truncate_parse_mode_and_size(">0"),
                Ok(TruncateMode::AtLeast(0))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_round_down() {
            assert_eq!(
                truncate_parse_mode_and_size("/4"),
                Ok(TruncateMode::RoundDown(4))
            );
            assert_eq!(
                truncate_parse_mode_and_size("/1"),
                Ok(TruncateMode::RoundDown(1))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_round_up() {
            assert_eq!(
                truncate_parse_mode_and_size("%5"),
                Ok(TruncateMode::RoundUp(5))
            );
            assert_eq!(
                truncate_parse_mode_and_size("%1"),
                Ok(TruncateMode::RoundUp(1))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_invalid() {
            assert_eq!(
                truncate_parse_mode_and_size("invalid"),
                Err(ParseSizeError::ParseFailure("'invalid'".to_string()))
            );
            assert_eq!(
                truncate_parse_mode_and_size("+invalid"),
                Err(ParseSizeError::ParseFailure("'+invalid'".to_string()))
            );
            assert_eq!(
                truncate_parse_mode_and_size(""),
                Err(ParseSizeError::ParseFailure("".to_string()))
            );
            assert_eq!(
                truncate_parse_mode_and_size("/0"),
                Ok(TruncateMode::RoundDown(0))
            );
            assert_eq!(
                truncate_parse_mode_and_size("%0"),
                Ok(TruncateMode::RoundUp(0))
            );
        }

        #[test]
        fn test_truncate_parse_mode_and_size_edge_cases() {
            // 边界条件测试
            assert_eq!(
                truncate_parse_mode_and_size(" "),
                Err(ParseSizeError::ParseFailure("".to_string()))
            );
            assert_eq!(
                truncate_parse_mode_and_size("+ "),
                Err(ParseSizeError::ParseFailure("'+ '".to_string()))
            );
            assert_eq!(
                truncate_parse_mode_and_size(" 100"),
                Ok(TruncateMode::Absolute(100))
            );
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::ffi::OsString;
        use std::io::Write;
        #[cfg(unix)]
        use std::os::unix::ffi::OsStringExt;

        use tempfile::tempdir;

        use super::*;

        // #[test]
        // fn test_truncate_main_size_short_10_gb() {
        //     let file = "test_truncate_main_size_short_10_gb";
        //     let dir = tempdir().unwrap();
        //     let file_path = dir.path().join(file);
        //     let mut tmp_file = File::create(&file_path).unwrap();
        //     writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
        //     let file_name = file_path.to_str().unwrap();
        //     let args = vec![ctcore::ct_util_name(), "-s", "10GB", file_name];
        //     let result = truncate_main(args.iter().map(|s| OsString::from(s)));
        //     assert!(result.is_ok());
        // }
        #[cfg(unix)]
        #[test]
        fn test_truncate_main_accepts_non_utf8_file_name() {
            let dir = tempdir().unwrap();
            let file_name = OsString::from_vec(b"raw-\xff".to_vec());
            let file_path = dir.path().join(&file_name);
            File::create(&file_path).unwrap();
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("-s"),
                OsString::from("7"),
                file_path.clone().into_os_string(),
            ];

            truncate_main(args.into_iter()).unwrap();

            assert_eq!(std::fs::metadata(file_path).unwrap().len(), 7);
        }

        #[test]
        fn test_truncate_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()]; // 缺少任何参数
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_truncate_main_reports_missing_file_operand() {
            let args = [ctcore::ct_util_name(), "-s", "0"];
            let error = truncate_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(error.to_string(), "missing file operand");
        }

        #[test]
        fn test_truncate_main_io_blocks_long() {
            let file = "test_truncate_main_io_blocks_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--io-blocks", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_truncate_main_io_blocks_short() {
            let file = "test_truncate_main_io_blocks_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-o", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_truncate_main_no_create_long() {
            let file = "test_truncate_main_no_create_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--no-create", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_truncate_main_no_create_short() {
            let file = "test_truncate_main_no_create_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-c", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_truncate_main_repeated_size_uses_last_value() {
            let file = "test_truncate_main_repeated_size";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "abcdef").unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--size", "1", "-s", "2", file_name];
            let result = truncate_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
            assert_eq!(metadata(&file_path).unwrap().len(), 2);
        }

        #[test]
        fn test_truncate_main_reference_long() {
            let file = "test_truncate_main_reference_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let reference_file = "test_truncate_main_reference_long_reference_file";
            let reference_file_path = dir.path().join(reference_file);
            let mut tmp_reference_file = File::create(&reference_file_path).unwrap();
            writeln!(
                tmp_reference_file,
                "tmp_reference_file test\nctyunos\nhello\nworld\n"
            )
            .unwrap();
            let reference_file_name = reference_file_path.to_str().unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--reference",
                reference_file_name,
                file_name,
            ];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_reference_short() {
            let file = "test_truncate_main_reference_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let reference_file = "test_truncate_main_reference_short_reference_file";
            let reference_file_path = dir.path().join(reference_file);
            let mut tmp_reference_file = File::create(&reference_file_path).unwrap();
            writeln!(
                tmp_reference_file,
                "tmp_reference_file test\nctyunos\nhello\nworld\n"
            )
            .unwrap();
            let reference_file_name = reference_file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-r", reference_file_name, file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_io_blocks_long_reference_short() {
            let file = "test_truncate_main_io_blocks_long_reference_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let reference_file = "reference_file";
            let reference_file_path = dir.path().join(reference_file);
            let mut tmp_reference_file = File::create(&reference_file_path).unwrap();
            writeln!(
                tmp_reference_file,
                "tmp_reference_file test\nctyunos\nhello\nworld\n"
            )
            .unwrap();
            let reference_file_name = reference_file_path.to_str().unwrap();

            let args = [
                ctcore::ct_util_name(),
                "-r",
                reference_file_name,
                "--io-blocks",
                file_name,
            ];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_truncate_main_io_blocks_short_reference_short() {
            let file = "test_truncate_main_io_blocks_short_reference_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let reference_file = "reference_file";
            let reference_file_path = dir.path().join(reference_file);
            let mut tmp_reference_file = File::create(&reference_file_path).unwrap();
            writeln!(
                tmp_reference_file,
                "tmp_reference_file test\nctyunos\nhello\nworld\n"
            )
            .unwrap();
            let reference_file_name = reference_file_path.to_str().unwrap();

            let args = [
                ctcore::ct_util_name(),
                "-r",
                reference_file_name,
                "-o",
                file_name,
            ];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_truncate_main_io_blocks_with_reference_requires_size() {
            let dir = tempdir().unwrap();
            let reference_path = dir.path().join("reference");
            let target_path = dir.path().join("target");
            std::fs::write(&reference_path, b"reference").unwrap();
            std::fs::write(&target_path, b"target").unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--io-blocks",
                "--reference",
                reference_path.to_str().unwrap(),
                target_path.to_str().unwrap(),
            ];
            let error = truncate_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(
                error.to_string(),
                "'--io-blocks' was specified but '--size' was not"
            );
            assert_eq!(metadata(&target_path).unwrap().len(), 6);
        }
        #[test]
        fn test_truncate_main_no_create_long_reference_short() {
            let file = "test_truncate_main_no_create_long_reference_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let reference_file = "reference_file";
            let reference_file_path = dir.path().join(reference_file);
            let mut tmp_reference_file = File::create(&reference_file_path).unwrap();
            writeln!(
                tmp_reference_file,
                "tmp_reference_file test\nctyunos\nhello\nworld\n"
            )
            .unwrap();
            let reference_file_name = reference_file_path.to_str().unwrap();

            let args = [
                ctcore::ct_util_name(),
                "-r",
                reference_file_name,
                "--no-create",
                file_name,
            ];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_no_create_short_reference_short() {
            let file = "test_truncate_main_no_create_short_reference_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let reference_file = "reference_file";
            let reference_file_path = dir.path().join(reference_file);
            let mut tmp_reference_file = File::create(&reference_file_path).unwrap();
            writeln!(
                tmp_reference_file,
                "tmp_reference_file test\nctyunos\nhello\nworld\n"
            )
            .unwrap();
            let reference_file_name = reference_file_path.to_str().unwrap();

            let args = [
                ctcore::ct_util_name(),
                "-r",
                reference_file_name,
                "-c",
                file_name,
            ];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_default_1000() {
            let file = "test_truncate_main_size_long_default_1000";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "1000", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_10_kb() {
            let file = "test_truncate_main_size_long_10_KB";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "10KB", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_10_k() {
            let file = "test_truncate_main_size_long_10_k";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "10K", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_10_mb() {
            let file = "test_truncate_main_size_long_10_MB";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "10MB", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_10_m() {
            let file = "test_truncate_main_size_long_10_M";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "10M", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_10_gb() {
            let file = "test_truncate_main_size_long_10_gb";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "10GB", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_10_g() {
            let file = "test_truncate_main_size_long_10_g";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "10G", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_extend_by_100() {
            let file = "test_truncate_main_size_long_extend_by_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "+100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_reduce_by_100() {
            let file = "test_truncate_main_size_long_reduce_by_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size=-100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_at_most_100() {
            let file = "test_truncate_main_size_long_extend_by_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "<100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_at_least_100() {
            let file = "test_truncate_main_size_long_at_least_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", ">100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_round_down_100() {
            let file = "test_truncate_main_size_long_round_down_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "/100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_long_round_up_100() {
            let file = "test_truncate_main_size_long_round_up_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--size", "%100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_default_1000() {
            let file = "test_truncate_main_size_short_default_1000";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "1000", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_10_kb() {
            let file = "test_truncate_main_size_short_10_KB";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "10KB", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_10_k() {
            let file = "test_truncate_main_size_short_10_k";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "10K", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_10_mb() {
            let file = "test_truncate_main_size_short_10_MB";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "10MB", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_10_m() {
            let file = "test_truncate_main_size_short_10_M";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "10M", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_10_gb() {
            let file = "test_truncate_main_size_short_10_gb";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "10GB", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_10_g() {
            let file = "test_truncate_main_size_short_10_g";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "10G", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_extend_by_100() {
            let file = "test_truncate_main_size_short_extend_by_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "+100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_reduce_by_100() {
            let file = "test_truncate_main_size_short_reduce_by_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s=-100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_at_most_100() {
            let file = "test_truncate_main_size_short_extend_by_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "<100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_at_least_100() {
            let file = "test_truncate_main_size_short_at_least_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", ">100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_round_down_100() {
            let file = "test_truncate_main_size_short_round_down_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "/100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_truncate_main_size_short_round_up_100() {
            let file = "test_truncate_main_size_short_round_up_100";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-s", "%100", file_name];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_truncate_main_execution_version() {
            let args_vec = [ctcore::ct_util_name(), "--version"];
            let args = args_vec.iter().map(OsString::from);
            let result = truncate_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_truncate_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = truncate_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_truncate_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_truncate_main_execution_help_short() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_truncate_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_truncate_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = truncate_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // truncate 接口: truncate [OPTION]... [FILE]...
        //
        // Arguments:
        //   <FILE>...
        //
        // Options:
        //   -o, --io-blocks          treat SIZE as the number of I/O blocks of the file rather than bytes (NOT IMPLEMENTED)
        //   -c, --no-create          do not create files that do not exist
        //   -r, --reference <RFILE>  base the size of each file on the size of RFILE
        //   -s, --size <SIZE>        set or adjust the size of each file according to SIZE, which is in bytes unless --io-blocks is specified
        //   -h, --help               Print help
        //   -V, --version            Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_help_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_io_blocks_long() {
            let command = ct_app();
            let file = "test_ct_app_io_blocks_long";
            let args = vec![ctcore::ct_util_name(), "--io-blocks", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_io_blocks_short() {
            let command = ct_app();
            let file = "test_ct_app_io_blocks_long";
            let args = vec![ctcore::ct_util_name(), "-o", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_io_blocks_allows_repeated_flags() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-o",
                "--io-blocks",
                "-s",
                "1",
                "target",
            ];
            let matches = command.try_get_matches_from(args).unwrap();

            assert!(matches.get_flag(truncate_flags::TRUNCATE_IO_BLOCKS));
        }

        #[test]
        fn test_ct_app_no_create_long() {
            let command = ct_app();
            let file = "test_ct_app_io_blocks_long";
            let args = vec![ctcore::ct_util_name(), "--no-create", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_create_short() {
            let command = ct_app();
            let file = "test_ct_app_io_blocks_long";
            let args = vec![ctcore::ct_util_name(), "-c", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_create_allows_repeated_flags() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-c",
                "--no-create",
                "-s",
                "0",
                "target",
            ];
            let matches = command.try_get_matches_from(args).unwrap();

            assert!(matches.get_flag(truncate_flags::TRUNCATE_NO_CREATE));
        }

        #[test]
        fn test_ct_app_reference_long() {
            let command = ct_app();
            let file = "test_ct_app_reference_long";
            let reference_file = "test_ct_app_reference_long_reference_file";
            let args = vec![ctcore::ct_util_name(), "--reference", reference_file, file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_reference_short() {
            let command = ct_app();
            let file = "test_ct_app_reference_short";
            let reference_file = "test_ct_app_reference_short_reference_file";
            let args = vec![ctcore::ct_util_name(), "-r", reference_file, file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_reference_allows_repeated_values_with_last_value() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--reference",
                "first-reference",
                "-r",
                "last-reference",
                "target",
            ];
            let matches = command.try_get_matches_from(args).unwrap();

            assert_eq!(
                matches.get_one::<OsString>(truncate_flags::TRUNCATE_REFERENCE),
                Some(&OsString::from("last-reference"))
            );
        }

        #[test]
        fn test_ct_app_io_blocks_long_reference_short() {
            let command = ct_app();
            let file = "test_ct_app_io_blocks_long";
            let reference_file = "reference_file";
            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                reference_file,
                "--io-blocks",
                file,
            ];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_io_blocks_short_reference_short() {
            let command = ct_app();
            let file = "test_ct_app_io_blocks_long";
            let reference_file = "reference_file";
            let args = vec![ctcore::ct_util_name(), "-r", reference_file, "-o", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_create_long_reference_short() {
            let command = ct_app();
            let file = "test_ct_app_io_blocks_long";
            let reference_file = "reference_file";
            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                reference_file,
                "--no-create",
                file,
            ];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_create_short_reference_short() {
            let command = ct_app();
            let file = "test_ct_app_io_blocks_long";
            let reference_file = "reference_file";
            let args = vec![ctcore::ct_util_name(), "-r", reference_file, "-c", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_default_1000() {
            let command = ct_app();
            let file = "test_ct_app_size_long_default_1000";
            let args = vec![ctcore::ct_util_name(), "--size", "1000", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_allows_repeated_size_with_last_value() {
            let command = ct_app();
            let file = "test_ct_app_size_allows_repeated_size";
            let args = vec![ctcore::ct_util_name(), "--size", "1", "-s", "2", file];
            let matches = command.try_get_matches_from(args).unwrap();

            assert_eq!(
                matches.get_one::<String>(truncate_flags::TRUNCATE_SIZE),
                Some(&"2".to_string())
            );
        }

        #[test]
        fn test_ct_app_size_long_10_kb() {
            let command = ct_app();
            let file = "test_ct_app_size_long_10_KB";
            let args = vec![ctcore::ct_util_name(), "--size", "10KB", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_10_k() {
            let command = ct_app();
            let file = "test_ct_app_size_long_10_k";
            let args = vec![ctcore::ct_util_name(), "--size", "10K", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_10_mb() {
            let command = ct_app();
            let file = "test_ct_app_size_long_10_MB";
            let args = vec![ctcore::ct_util_name(), "--size", "10MB", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_10_m() {
            let command = ct_app();
            let file = "test_ct_app_size_long_10_M";
            let args = vec![ctcore::ct_util_name(), "--size", "10M", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_10_gb() {
            let command = ct_app();
            let file = "test_ct_app_size_long_10_gb";
            let args = vec![ctcore::ct_util_name(), "--size", "10GB", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_10_g() {
            let command = ct_app();
            let file = "test_ct_app_size_long_10_g";
            let args = vec![ctcore::ct_util_name(), "--size", "10G", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_extend_by_100() {
            let command = ct_app();
            let file = "test_ct_app_size_long_extend_by_100";
            let args = vec![ctcore::ct_util_name(), "--size", "+100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_reduce_by_100() {
            let command = ct_app();
            let file = "test_ct_app_size_long_reduce_by_100";
            let args = vec![ctcore::ct_util_name(), "--size=-100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_at_most_100() {
            let command = ct_app();
            let file = "test_ct_app_size_long_extend_by_100";
            let args = vec![ctcore::ct_util_name(), "--size", "<100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_at_least_100() {
            let command = ct_app();
            let file = "test_ct_app_size_long_at_least_100";
            let args = vec![ctcore::ct_util_name(), "--size", ">100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_round_down_100() {
            let command = ct_app();
            let file = "test_ct_app_size_long_round_down_100";
            let args = vec![ctcore::ct_util_name(), "--size", "/100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_long_round_up_100() {
            let command = ct_app();
            let file = "test_ct_app_size_long_round_up_100";
            let args = vec![ctcore::ct_util_name(), "--size", "%100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_default_1000() {
            let command = ct_app();
            let file = "test_ct_app_size_short_default_1000";
            let args = vec![ctcore::ct_util_name(), "-s", "1000", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_10_kb() {
            let command = ct_app();
            let file = "test_ct_app_size_short_10_KB";
            let args = vec![ctcore::ct_util_name(), "-s", "10KB", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_10_k() {
            let command = ct_app();
            let file = "test_ct_app_size_short_10_k";
            let args = vec![ctcore::ct_util_name(), "-s", "10K", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_10_mb() {
            let command = ct_app();
            let file = "test_ct_app_size_short_10_MB";
            let args = vec![ctcore::ct_util_name(), "-s", "10MB", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_10_m() {
            let command = ct_app();
            let file = "test_ct_app_size_short_10_M";
            let args = vec![ctcore::ct_util_name(), "-s", "10M", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_10_gb() {
            let command = ct_app();
            let file = "test_ct_app_size_short_10_gb";
            let args = vec![ctcore::ct_util_name(), "-s", "10GB", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_10_g() {
            let command = ct_app();
            let file = "test_ct_app_size_short_10_g";
            let args = vec![ctcore::ct_util_name(), "-s", "10G", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_extend_by_100() {
            let command = ct_app();
            let file = "test_ct_app_size_short_extend_by_100";
            let args = vec![ctcore::ct_util_name(), "-s", "+100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_reduce_by_100() {
            let command = ct_app();
            let file = "test_ct_app_size_short_reduce_by_100";
            let args = vec![ctcore::ct_util_name(), "-s=-100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_at_most_100() {
            let command = ct_app();
            let file = "test_ct_app_size_short_extend_by_100";
            let args = vec![ctcore::ct_util_name(), "-s", "<100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_at_least_100() {
            let command = ct_app();
            let file = "test_ct_app_size_short_at_least_100";
            let args = vec![ctcore::ct_util_name(), "-s", ">100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_round_down_100() {
            let command = ct_app();
            let file = "test_ct_app_size_short_round_down_100";
            let args = vec![ctcore::ct_util_name(), "-s", "/100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_size_short_round_up_100() {
            let command = ct_app();
            let file = "test_ct_app_size_short_round_up_100";
            let args = vec![ctcore::ct_util_name(), "-s", "%100", file];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
