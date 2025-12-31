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

//! truncate 是一个 Linux 命令，用于修改文件的大小，它可以将文件的大小缩小或扩展到指定的大小。

use std::fs::{metadata, OpenOptions};
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

use clap::{crate_version, Arg, ArgAction, Command};

use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_parse_size::{parse_size_u64, ParseSizeError};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};

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
    /// assert_eq!(mode.to_size(fsize), 15);
    /// ```
    ///
    /// 如果减小的字节数超过文件的大小，结果将为0：
    ///
    /// ```rust,ignore
    /// let mode = TruncateMode::Reduce(5);
    /// let fsize = 3;
    /// assert_eq!(mode.to_size(fsize), 0);
    /// ```
    fn to_size(&self, fsize: u64) -> u64 {
        match self {
            Self::Absolute(size) => *size,
            Self::Extend(size) => fsize + size,
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
            Self::RoundUp(size) => fsize + fsize % size,
        }
    }
}

const TRUNCATE_ABOUT: &str = ct_help_about!("truncate.md");
const TRUNCATE_AFTER_HELP: &str = ct_help_section!("after help", "truncate.md");
const TRUNCATE_USAGE: &str = ct_help_usage!("truncate.md");

pub mod truncate_flags {
    pub const TRUNCATE_IO_BLOCKS: &str = "io-blocks";
    pub const TRUNCATE_NO_CREATE: &str = "no-create";
    pub const TRUNCATE_REFERENCE: &str = "reference";
    pub const TRUNCATE_SIZE: &str = "size";
    pub const TRUNCATE_ARG_FILES: &str = "files";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    truncate_main(args)
}
pub fn truncate_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args).map_err(|e| {
        e.print().expect("Error writing clap::Error");
        match e.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => 0,
            _ => 1,
        }
    })?;

    let files: Vec<String> = matches
        .get_many::<String>(truncate_flags::TRUNCATE_ARG_FILES)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    if files.is_empty() {
        Err(CTsageError::new(1, "missing file operand"))
    } else {
        let is_io_blocks = matches.get_flag(truncate_flags::TRUNCATE_IO_BLOCKS);
        let is_no_create = matches.get_flag(truncate_flags::TRUNCATE_NO_CREATE);
        let reference = matches
            .get_one::<String>(truncate_flags::TRUNCATE_REFERENCE)
            .map(String::from);
        let size = matches
            .get_one::<String>(truncate_flags::TRUNCATE_SIZE)
            .map(String::from);
        truncate(is_no_create, is_io_blocks, reference, size, &files)
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = TRUNCATE_ABOUT;
    let usage_description = ct_format_usage(TRUNCATE_USAGE);
    let args = vec![
        Arg::new(truncate_flags::TRUNCATE_IO_BLOCKS)
            .short('o')
            .long(truncate_flags::TRUNCATE_IO_BLOCKS)
            .help(
                "treat SIZE as the number of I/O blocks of the file rather than bytes \
            (NOT IMPLEMENTED)",
            )
            .action(ArgAction::SetTrue),
        Arg::new(truncate_flags::TRUNCATE_NO_CREATE)
            .short('c')
            .long(truncate_flags::TRUNCATE_NO_CREATE)
            .help("do not create files that do not exist")
            .action(ArgAction::SetTrue),
        Arg::new(truncate_flags::TRUNCATE_REFERENCE)
            .short('r')
            .long(truncate_flags::TRUNCATE_REFERENCE)
            .required_unless_present(truncate_flags::TRUNCATE_SIZE)
            .help("base the size of each file on the size of RFILE")
            .value_name("RFILE")
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(truncate_flags::TRUNCATE_SIZE)
            .short('s')
            .long(truncate_flags::TRUNCATE_SIZE)
            .required_unless_present(truncate_flags::TRUNCATE_REFERENCE)
            .help(
                "set or adjust the size of each file according to SIZE, which is in \
            bytes unless --io-blocks is specified",
            )
            .value_name("SIZE"),
        Arg::new(truncate_flags::TRUNCATE_ARG_FILES)
            .value_name("FILE")
            .action(ArgAction::Append)
            .required(true)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .after_help(TRUNCATE_AFTER_HELP)
        .args(args)
}

/// 将指定文件截断到给定的大小。
///
/// 如果 `create` 为真，那么如果文件尚不存在，文件将会被创建。如果 `size` 大于文件中的字节数，文件将用零填充。
/// 如果 `size` 小于文件的字节数，文件将被截断，`size` 之后的任何字节都将丢失。
///
/// # 错误
///
/// 如果文件无法被打开，或者设置文件大小时出现错误。
fn truncate_file(filename: &str, create: bool, size: u64) -> CTResult<()> {
    #[cfg(unix)]
    if let Ok(md) = std::fs::metadata(filename) {
        if md.file_type().is_fifo() {
            let err_massage = format!(
                "cannot open {} for writing: No such device or address",
                filename.quote()
            );
            return Err(CtSimpleError::new(1, err_massage));
        }
    }
    let path = Path::new(filename);
    match OpenOptions::new().write(true).create(create).open(path) {
        Ok(file) => file.set_len(size),
        Err(e) => {
            if e.kind() == ErrorKind::NotFound && !create {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
    .map_err_context(|| format!("cannot open {} for writing", filename.quote()))
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
fn truncate_reference_and_size(
    r_file_name: &str,
    size_string: &str,
    filenames: &[String],
    is_create: bool,
) -> CTResult<()> {
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
    let md = metadata(r_file_name).map_err(|e| match e.kind() {
        ErrorKind::NotFound => {
            let err_massage = format!(
                "cannot stat {}: No such file or directory",
                r_file_name.quote()
            );
            CtSimpleError::new(1, err_massage)
        }
        _ => e.map_err_context(String::new),
    })?;

    let md_size = md.len();
    let t_size = truncate_mode.to_size(md_size);
    for filename in filenames {
        truncate_file(filename, is_create, t_size)?;
    }
    Ok(())
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
fn truncate_reference_file_only(
    r_file_name: &str,
    filenames: &[String],
    is_create: bool,
) -> CTResult<()> {
    let md = metadata(r_file_name).map_err(|e| match e.kind() {
        ErrorKind::NotFound => {
            let err_massage = format!(
                "cannot stat {}: No such file or directory",
                r_file_name.quote()
            );
            CtSimpleError::new(1, err_massage)
        }
        _ => e.map_err_context(String::new),
    })?;
    let t_size = md.len();
    for filename in filenames {
        truncate_file(filename, is_create, t_size)?;
    }
    Ok(())
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
fn truncate_size_only(size_string: &str, filenames: &[String], is_create: bool) -> CTResult<()> {
    let truncate_mode = truncate_parse_mode_and_size(size_string)
        .map_err(|e| CtSimpleError::new(1, format!("Invalid number: {e}")))?;
    if let TruncateMode::RoundDown(0) | TruncateMode::RoundUp(0) = truncate_mode {
        return Err(CtSimpleError::new(1, "division by zero"));
    }
    for filename in filenames {
        let f_size = match metadata(filename) {
            Ok(md) => {
                #[cfg(unix)]
                if md.file_type().is_fifo() {
                    let err_massage = format!(
                        "cannot open {} for writing: No such device or address",
                        filename.quote()
                    );
                    return Err(CtSimpleError::new(1, err_massage));
                }
                md.len()
            }
            Err(_) => 0,
        };
        let t_size = truncate_mode.to_size(f_size);
        // TODO: 修复对stat的重复调用
        truncate_file(filename, is_create, t_size)?;
    }
    Ok(())
}

fn truncate(
    is_no_create: bool,
    _: bool,
    reference: Option<String>,
    size: Option<String>,
    filenames: &[String],
) -> CTResult<()> {
    let is_create = !is_no_create;
    // 存在四种可能的情况：
    // - 已给出参考文件且已给出大小，
    // - 已给出参考文件但未给出大小，
    // - 未给出参考文件但已给出大小，
    // - 既未给出参考文件也未给出大小，
    match (reference, size) {
        (Some(r_file_name), Some(size_string)) => {
            truncate_reference_and_size(&r_file_name, &size_string, filenames, is_create)
        }
        (Some(r_file_name), None) => {
            truncate_reference_file_only(&r_file_name, filenames, is_create)
        }
        (None, Some(size_string)) => truncate_size_only(&size_string, filenames, is_create),
        (None, None) => unreachable!(), // 这种情况现在不可能发生，因为它已经被clap处理了
    }
}

/// 判断一个字符是否是大小修饰符，如 '+' 或 '<'。
fn is_modifier(c: char) -> bool {
    c == '+' || c == '-' || c == '<' || c == '>' || c == '/' || c == '%'
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
    // 删除任何空白字符。
    let mut size_string = size_string.trim();

    // 从大小字符串中获取任何存在的修饰符字符。例如，如果参数是 "+123"，那么修饰符就是 '+'。
    if let Some(c) = size_string.chars().next() {
        if is_modifier(c) {
            size_string = &size_string[1..];
        }
        parse_size_u64(size_string).map(match c {
            '+' => TruncateMode::Extend,
            '-' => TruncateMode::Reduce,
            '<' => TruncateMode::AtMost,
            '>' => TruncateMode::AtLeast,
            '/' => TruncateMode::RoundDown,
            '%' => TruncateMode::RoundUp,
            _ => TruncateMode::Absolute,
        })
    } else {
        Err(ParseSizeError::ParseFailure(size_string.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

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
                Some(reference_file_path.clone()),
                Some("+5".to_string()),
                &target_files,
            )
            .unwrap();
            assert_eq!(metadata(&target_file_path).unwrap().len(), 19); // "Hello, world!" 长度 + 5

            truncate(
                false,
                false,
                Some(reference_file_path.clone()),
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
                Some(reference_file_path.clone()),
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
                Some("test_truncate_errors2".to_string()),
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
            truncate_size_only("+5", &target_files, true).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 19);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 19);

            // 测试相对调整大小：Reduce
            truncate_size_only("-3", &target_files, true).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 16);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 16);

            // 测试相对调整大小：AtMost
            truncate_size_only("<8", &target_files, true).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 8);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 8);

            // 测试相对调整大小：AtLeast
            truncate_size_only(">20", &target_files, true).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 20);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 20);

            // 测试相对调整大小：RoundDown
            truncate_size_only("/4", &target_files, true).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 20); // 20 already multiple of 4
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 20); // 20 already multiple of 4

            // 测试相对调整大小：RoundUp
            truncate_size_only("%3", &target_files, true).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 22); // next multiple of 3
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 22); // next multiple of 3
        }

        #[test]
        fn test_truncate_size_only_errors() {
            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 测试无效大小字符串
            let result = truncate_size_only("invalid", &target_files, true);
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("Invalid number"));

            // 测试除以零的情况
            let result = truncate_size_only("/0", &target_files, true);
            assert!(result.is_err());
            let error_message = format!("{}", result.unwrap_err());
            assert!(error_message.contains("division by zero"));

            let result = truncate_size_only("%0", &target_files, true);
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
            let result = truncate_size_only("+5", &target_files, false);
            assert!(result.is_ok());
            assert!(!std::path::Path::new(non_existent_file).exists());

            // 设置 create 为 true
            truncate_size_only("+5", &target_files, true).unwrap();
            assert_eq!(metadata(non_existent_file).unwrap().len(), 5);

            // 清理
            std::fs::remove_file(non_existent_file).unwrap();
        }

        #[test]
        fn test_truncate_size_only_zero_length() {
            // 创建目标文件并写入一些数据
            let mut target_file = NamedTempFile::new().unwrap();
            writeln!(target_file, "Target file").unwrap();
            let target_file_path = target_file.path().to_str().unwrap().to_string();

            let target_files = vec![target_file_path.clone()];

            // 使用绝对大小进行调整（零长度）
            truncate_size_only("0", &target_files, true).unwrap();
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
            truncate_size_only("10", &target_files, true).unwrap();
            assert_eq!(metadata(&target_file1_path).unwrap().len(), 10);
            assert_eq!(metadata(&target_file2_path).unwrap().len(), 10);
        }
    }

    #[cfg(test)]
    mod truncate_reference_file_only_tests {
        use std::fs::metadata;
        use std::io::Write;

        use tempfile::NamedTempFile;

        use super::*;

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
}