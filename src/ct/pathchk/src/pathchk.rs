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

//！ pathchk判断无效或未移植的文件名。

extern crate rust_i18n;
use clap::ArgMatches;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, set_ct_exit_code};
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Write};
use sys_locale::get_locale;

// operating mode
enum PathchkMode {
    Default, // use filesystem to determine information and limits
    Basic,   // check basic compatibility with POSIX
    Extra,   // check for leading dashes and empty names
    Both,    // a combination of `Basic` and `Extra`
}

pub mod pathchk_flags {
    pub const PATHCHK_POSIX: &str = "posix";
    pub const PATHCHK_POSIX_SPECIAL: &str = "posix-special";
    pub const PATHCHK_PORTABILITY: &str = "portability";
    pub const PATHCHK_PATH: &str = "path";
}

// a few global constants as used in the GNU implementation
const PATHCHK_POSIX_PATH_MAX: usize = 256;
const PATHCHK_POSIX_NAME_MAX: usize = 14;

/// PathchkFlags 结构体用于存储和管理 pathchk 命令的运行参数
struct PathchkFlags {
    mode: PathchkMode,  // 检查模式
    paths: Vec<String>, // 需要检查的路径列表
}

impl PathchkFlags {
    /// 从命令行参数创建 PathchkFlags 实例
    ///
    /// # 参数
    /// * `matches` - 解析后的命令行参数
    ///
    /// # 返回
    /// * `CTResult<Self>` - 成功则返回 PathchkFlags 实例，失败则返回错误
    fn new(matches: &ArgMatches) -> CTResult<Self> {
        // 获取路径参数
        let paths = matches
            .get_many::<String>(pathchk_flags::PATHCHK_PATH)
            .ok_or_else(|| CTsageError::new(1, "missing operand"))?
            .cloned()
            .collect();

        // 设置工作模式
        let mode = {
            let is_posix = matches.get_flag(pathchk_flags::PATHCHK_POSIX);
            let is_posix_special = matches.get_flag(pathchk_flags::PATHCHK_POSIX_SPECIAL);
            let is_portability = matches.get_flag(pathchk_flags::PATHCHK_PORTABILITY);

            if (is_posix && is_posix_special) || is_portability {
                PathchkMode::Both
            } else if is_posix {
                PathchkMode::Basic
            } else if is_posix_special {
                PathchkMode::Extra
            } else {
                PathchkMode::Default
            }
        };

        Ok(Self { mode, paths })
    }
}

/// 程序入口点，设置标准错误输出并调用主函数
///
/// # 参数
/// * `args` - 命令行参数
///
/// # 返回
/// * `CTResult<()>` - 执行结果
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let stdout = std::io::stderr();
    let mut out = stdout.lock();

    pathchk_main(&mut out, args)
}

/// pathchk 命令的主要实现函数
///
/// # 参数
/// * `writer` - 输出写入器
/// * `args` - 命令行参数
///
/// # 返回
/// * `CTResult<()>` - 执行结果
pub fn pathchk_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 尝试解析命令行参数
    let matches = ct_app().try_get_matches_from(args)?;
    // 解析参数到 PathchkFlags 结构体
    let flags = PathchkFlags::new(&matches)?;

    pathchk_exec(writer, &flags)
}

/// 执行路径检查的核心函数
///
/// # 参数
/// * `writer` - 输出写入器
/// * `flags` - 解析后的命令行参数
///
/// # 返回
/// * `CTResult<()>` - 执行结果
fn pathchk_exec<W: Write>(writer: &mut W, flags: &PathchkFlags) -> CTResult<()> {
    let mut is_success = true;
    for path in &flags.paths {
        let path_segments: Vec<String> = path.split('/').map(String::from).collect();
        is_success &= check_path(writer, &flags.mode, &path_segments)?;
    }

    if !is_success {
        set_ct_exit_code(1);
    }
    Ok(())
}

/// 创建命令行参数解析器
///
/// # 返回
/// * `Command` - clap 命令行解析器实例
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("pathchk.about");
    let usage_description = t!("pathchk.usage");
    let args = vec![
        Arg::new(pathchk_flags::PATHCHK_POSIX)
            .short('p')
            .help(t!("pathchk.clap.pathchk_posix"))
            .action(ArgAction::SetTrue),
        Arg::new(pathchk_flags::PATHCHK_POSIX_SPECIAL)
            .short('P')
            .help(r#"check for empty names and leading "-""#)
            .action(ArgAction::SetTrue),
        Arg::new(pathchk_flags::PATHCHK_PORTABILITY)
            .long(pathchk_flags::PATHCHK_PORTABILITY)
            .help(t!("pathchk.clap.pathchk_portability"))
            .action(ArgAction::SetTrue),
        Arg::new(pathchk_flags::PATHCHK_PATH)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

/// 根据指定的模式检查路径
///
/// # 参数
/// * `writer` - 输出写入器
/// * `mode` - 检查模式
/// * `path` - 待检查的路径组件
///
/// # 返回
/// * `CTResult<bool>` - 检查结果，true 表示通过检查
fn check_path<W: Write>(writer: &mut W, mode: &PathchkMode, path: &[String]) -> CTResult<bool> {
    let result = match *mode {
        PathchkMode::Basic => check_basic(writer, path)?,
        PathchkMode::Extra => check_default(writer, path)? && check_extra(writer, path)?,
        PathchkMode::Both => check_basic(writer, path)? && check_extra(writer, path)?,
        _ => check_default(writer, path)?,
    };
    Ok(result)
}

/// 执行 POSIX 基本兼容性检查
///
/// # 参数
/// * `writer` - 输出写入器
/// * `path` - 待检查的路径组件
///
/// # 返回
/// * `CTResult<bool>` - 检查结果，true 表示通过检查
fn check_basic<W: Write>(writer: &mut W, path: &[String]) -> CTResult<bool> {
    let joined_path = path.join("/");
    let total_len = joined_path.len();

    // First check empty path
    if total_len == 0 {
        writeln!(writer, "pathchk: empty file name")?;
        return Ok(false);
    }

    // Then check portable characters for each component
    for p in path {
        if !check_portable_chars(writer, p)? {
            return Ok(false);
        }

        // Only check length after character validation
        let component_len = p.len();
        if component_len > PATHCHK_POSIX_NAME_MAX {
            writeln!(
                writer,
                "pathchk: limit {} exceeded by length {} of file name component ‘{}’",
                PATHCHK_POSIX_NAME_MAX, component_len, p
            )?;
            return Ok(false);
        }
    }

    // Finally check total path length
    if total_len > PATHCHK_POSIX_PATH_MAX {
        writeln!(
            writer,
            "pathchk: limit {} exceeded by length {} of file name '{}'",
            PATHCHK_POSIX_PATH_MAX, total_len, joined_path
        )?;
        return Ok(false);
    }

    // permission checks
    check_searchable(writer, &joined_path)
}

/// 执行额外的兼容性检查（空名称和前导连字符）
///
/// # 参数
/// * `writer` - 输出写入器
/// * `path` - 待检查的路径组件
///
/// # 返回
/// * `CTResult<bool>` - 检查结果，true 表示通过检查
fn check_extra<W: Write>(writer: &mut W, path: &[String]) -> CTResult<bool> {
    // components: leading hyphens
    for p in path {
        if p.starts_with('-') {
            writeln!(
                writer,
                "pathchk: leading '-' in a component of file name {}",
                p.quote()
            )?;
            return Ok(false);
        }
    }
    // path length
    if path.join("/").is_empty() {
        writeln!(writer, "pathchk: empty file name")?;
        return Ok(false);
    }
    Ok(true)
}

/// 使用文件系统执行默认检查
///
/// # 参数
/// * `writer` - 输出写入器
/// * `path` - 待检查的路径组件
///
/// # 返回
/// * `CTResult<bool>` - 检查结果，true 表示通过检查
fn check_default<W: Write>(writer: &mut W, path: &[String]) -> CTResult<bool> {
    let joined_path = path.join("/");
    let total_len = joined_path.len();

    // First check empty path
    if total_len == 0 {
        writeln!(writer, "pathchk: empty file name")?;
        return Ok(false);
    }

    // Then check path length
    if total_len > libc::PATH_MAX as usize {
        writeln!(
            writer,
            "pathchk: limit {} exceeded by length {} of file name ‘{}’",
            libc::PATH_MAX,
            total_len,
            joined_path
        )?;
        return Ok(false);
    }

    // Check components length
    for p in path {
        let component_len = p.len();
        if component_len > libc::FILENAME_MAX as usize {
            writeln!(
                writer,
                "pathchk: limit {} exceeded by length {} of file name component ‘{}’",
                libc::FILENAME_MAX,
                component_len,
                p
            )?;
            return Ok(false);
        }
    }

    // Finally do permission checks
    check_searchable(writer, &joined_path)
}

/// 检查路径是否可搜索或是否存在其他问题
///
/// # 参数
/// * `writer` - 输出写入器
/// * `path` - 待检查的路径
///
/// # 返回
/// * `CTResult<bool>` - 检查结果，true 表示通过检查
fn check_searchable<W: Write>(writer: &mut W, path: &str) -> CTResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) => {
            if e.kind() == ErrorKind::NotFound {
                Ok(true)
            } else if e.raw_os_error() == Some(36) {
                // ENAMETOOLONG
                writeln!(writer, "pathchk: {}: File name too long", path)?;
                Ok(false)
            } else {
                writeln!(writer, "pathchk: {}: {}", path, e)?;
                Ok(false)
            }
        }
    }
}

/// 检查路径段是否只包含有效（可移植）字符
///
/// # 参数
/// * `writer` - 输出写入器
/// * `path_segment` - 待检查的路径段
///
/// # 返回
/// * `CTResult<bool>` - 检查结果，true 表示通过检查
fn check_portable_chars<W: Write>(writer: &mut W, path_segment: &str) -> CTResult<bool> {
    const VALID_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-";
    for (i, ch) in path_segment.as_bytes().iter().enumerate() {
        if !VALID_CHARS.contains(ch) {
            let invalid = path_segment[i..].chars().next().unwrap();
            writeln!(
                writer,
                "pathchk: nonportable character ‘{}’ in file name '{}'",
                invalid, path_segment
            )?;
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Default)]
pub struct Pathchk;
impl Tool for Pathchk {
    fn name(&self) -> &'static str {
        "pathchk"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let stdout = std::io::stderr();
        let mut out = stdout.lock();
        pathchk_main(&mut out, args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::io::Cursor;

    mod pathchk_flags_tests {
        use super::*;

        #[test]
        fn test_flags_basic() {
            let args = vec![ctcore::ct_util_name(), "-p", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = PathchkFlags::new(&matches).unwrap();

            assert!(matches!(flags.mode, PathchkMode::Basic));
            assert_eq!(flags.paths, vec!["file.txt"]);
        }

        #[test]
        fn test_flags_extra() {
            let args = vec![ctcore::ct_util_name(), "-P", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = PathchkFlags::new(&matches).unwrap();

            assert!(matches!(flags.mode, PathchkMode::Extra));
        }

        #[test]
        fn test_flags_both() {
            let args = vec![ctcore::ct_util_name(), "--portability", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = PathchkFlags::new(&matches).unwrap();

            assert!(matches!(flags.mode, PathchkMode::Both));
        }

        #[test]
        fn test_flags_missing_path() {
            let args = vec![ctcore::ct_util_name(), "-p"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = PathchkFlags::new(&matches);
            assert!(result.is_err());
        }

        #[test]
        fn test_flags_default_mode() {
            let args = vec![ctcore::ct_util_name(), "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = PathchkFlags::new(&matches).unwrap();
            assert!(matches!(flags.mode, PathchkMode::Default));
        }

        #[test]
        fn test_flags_both_with_posix_and_special() {
            let args = vec![ctcore::ct_util_name(), "-p", "-P", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = PathchkFlags::new(&matches).unwrap();
            assert!(matches!(flags.mode, PathchkMode::Both));
        }
    }

    mod pathchk_execution_tests {
        use super::*;

        #[test]
        fn test_nonportable_character() {
            let args = vec![ctcore::ct_util_name(), "-p", "special#file"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(
                output_str
                    .contains("pathchk: nonportable character ‘#’ in file name 'special#file'")
            );
        }

        #[test]
        fn test_component_too_long() {
            let long_name = "a".repeat(15);
            let args = vec![ctcore::ct_util_name(), "-p", &long_name];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("limit 14 exceeded by length 15"));
        }

        #[test]
        fn test_leading_hyphen() {
            let args = vec![ctcore::ct_util_name(), "-P", "-"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("leading '-' in a component of file name '-'"));
        }

        #[test]
        fn test_empty_filename() {
            let args = vec![ctcore::ct_util_name(), "-P", ""];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();

            assert!(output_str.contains("pathchk: empty file name"));
        }

        #[test]
        fn test_filename_too_long() {
            let long_name = "a".repeat(300);
            let args = vec![ctcore::ct_util_name(), "-P", &long_name];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("File name too long"));
            assert!(output_str.contains(&long_name));
        }

        #[test]
        fn test_path_with_multiple_components() {
            let args = vec![ctcore::ct_util_name(), "-p", "dir1/dir2/file"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_multiple_paths() {
            let args = vec![ctcore::ct_util_name(), "-p", "file1", "file2"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_path_with_dots() {
            let args = vec![ctcore::ct_util_name(), "-p", "../file"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_path_with_special_chars() {
            let args = vec![ctcore::ct_util_name(), "-p", "file@name"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("pathchk: nonportable character ‘@’"));
        }

        #[test]
        fn test_path_with_long_component() {
            let long_component = format!("dir/{}", "a".repeat(15));
            let args = vec![ctcore::ct_util_name(), "-p", &long_component];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("limit 14 exceeded"));
        }

        #[test]
        fn test_path_with_long_total_length() {
            let long_path = format!("{}/file", "a".repeat(255));
            let args = vec![ctcore::ct_util_name(), "-p", &long_path];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();

            assert!(output_str.contains("exceeded by length 255 of file"));
        }
    }

    mod check_functions_tests {
        use super::*;

        #[test]
        fn test_check_portable_chars() {
            let mut output = Cursor::new(Vec::new());
            assert!(check_portable_chars(&mut output, "valid-name.txt").unwrap());
            assert!(!check_portable_chars(&mut output, "invalid#name").unwrap());
            assert!(!check_portable_chars(&mut output, "name@domain").unwrap());
        }

        #[test]
        fn test_check_searchable() {
            let mut output = Cursor::new(Vec::new());
            assert!(check_searchable(&mut output, ".").unwrap());
            assert!(check_searchable(&mut output, "nonexistent_file").unwrap());
        }

        #[test]
        fn test_check_extra_with_multiple_components() {
            let mut output = Cursor::new(Vec::new());
            let path = vec!["-bad".to_string(), "name".to_string()];
            assert!(!check_extra(&mut output, &path).unwrap());
        }

        #[test]
        fn test_check_basic_with_valid_path() {
            let mut output = Cursor::new(Vec::new());
            let path = vec!["valid".to_string(), "path.txt".to_string()];
            assert!(check_basic(&mut output, &path).unwrap());
        }

        #[test]
        fn test_check_default_with_valid_path() {
            let mut output = Cursor::new(Vec::new());
            let path = vec!["valid".to_string(), "path.txt".to_string()];
            assert!(check_default(&mut output, &path).unwrap());
        }
    }

    mod ct_app_tests {
        use super::*;
        use clap::error::ErrorKind;

        #[test]
        fn test_app_all_options() {
            let args = vec![
                ctcore::ct_util_name(),
                "-p",
                "-P",
                "--portability",
                "file.txt",
            ];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_app_minimal_options() {
            let args = vec![ctcore::ct_util_name(), "file.txt"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_app_invalid_option() {
            let args = vec![ctcore::ct_util_name(), "--invalid-option", "file.txt"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
    }

    mod tests_tool_implementation {
        use crate::Pathchk;
        use ctcore::Tool;
        use std::ffi::OsString;

        #[test]
        fn test_tool_implementation() {
            let tool = Pathchk::default();

            // 测试 name 方法
            assert_eq!(tool.name(), "pathchk");

            // 测试 command 方法
            let command = tool.command();
            assert!(command.get_name().contains("pathchk"));

            // 测试 execute 方法
            let args = vec![OsString::from("pathchk"), OsString::from("--help")];
            assert!(tool.execute(&args).is_err()); // --help参数通常会返回错误
        }
    }
}
