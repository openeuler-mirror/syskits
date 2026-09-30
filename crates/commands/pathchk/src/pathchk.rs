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
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, ArgAction, Command, builder::OsStringValueParser, crate_version};
use ctcore::Tool;
use ctcore::ct_display::locale_quote_marks;
use ctcore::ct_error::{CTResult, CTsageError, set_ct_exit_code};
use ctcore::ct_posix::GnuGetoptCommandExt;
use ctcore::ct_quoting_style::gnu_quote_shell_bytes;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use sys_locale::get_locale;

// operating mode
#[derive(Clone, Copy)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathchkRow {
    pub path: String,
    pub ok: bool,
    pub diagnostic_kind: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathchkSemantic {
    pub rows: Vec<PathchkRow>,
    pub stderr_text: String,
    pub exit_code: i32,
}

// a few global constants as used in the GNU implementation
const PATHCHK_POSIX_PATH_MAX: usize = 256;
const PATHCHK_POSIX_NAME_MAX: usize = 14;

fn initialize_locale() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }
}

/// PathchkFlags 结构体用于存储和管理 pathchk 命令的运行参数
struct PathchkFlags {
    mode: PathchkMode,    // 检查模式
    paths: Vec<OsString>, // 需要检查的路径列表
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
            .get_many::<OsString>(pathchk_flags::PATHCHK_PATH)
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
    initialize_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 尝试解析命令行参数
    let matches = ct_app().try_get_matches_from(args)?;
    // 解析参数到 PathchkFlags 结构体
    let flags = PathchkFlags::new(&matches)?;

    pathchk_exec(writer, &flags)
}

pub fn pathchk_native_semantic(args: impl ctcore::Args) -> CTResult<PathchkSemantic> {
    initialize_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;
    let flags = PathchkFlags::new(&matches)?;

    let mut rows = Vec::with_capacity(flags.paths.len());
    let mut stderr_text = String::new();
    let mut exit_code = 0;

    for path in &flags.paths {
        let mut diagnostic = Vec::new();
        let ok = check_path(&mut diagnostic, &flags.mode, path.as_os_str())?;
        let message_raw = String::from_utf8_lossy(&diagnostic).into_owned();
        if !ok {
            exit_code = 1;
            stderr_text.push_str(&message_raw);
        }

        let message =
            (!message_raw.is_empty()).then_some(message_raw.trim_end_matches('\n').to_string());
        let diagnostic_kind = message
            .as_deref()
            .map(pathchk_diagnostic_kind)
            .map(str::to_string);

        rows.push(PathchkRow {
            path: path.to_string_lossy().into_owned(),
            ok,
            diagnostic_kind,
            message,
        });
    }

    Ok(PathchkSemantic {
        rows,
        stderr_text,
        exit_code,
    })
}

fn pathchk_diagnostic_kind(message: &str) -> &'static str {
    if message.contains("empty file name") {
        "empty_name"
    } else if message.contains("non-portable character")
        || message.contains("nonportable character")
    {
        "non_portable_character"
    } else if message.contains("leading '-'") {
        "leading_dash"
    } else if message.contains("component") && message.contains("exceeded") {
        "component_too_long"
    } else if message.contains("file name") && message.contains("exceeded") {
        "path_too_long"
    } else if message.contains("File name too long") {
        "os_path_too_long"
    } else {
        "path_error"
    }
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
    set_ct_exit_code(0);
    let mut is_success = true;
    for path in &flags.paths {
        is_success &= check_path(writer, &flags.mode, path.as_os_str())?;
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
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        // GNU pathchk uses getopt with a leading '+' in its option string,
        // so option parsing always stops at the first path operand.
        .gnu_getopt_with_mode(true)
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
fn check_path<W: Write>(writer: &mut W, mode: &PathchkMode, path: &OsStr) -> CTResult<bool> {
    let result = match *mode {
        PathchkMode::Basic => check_basic(writer, path)?,
        PathchkMode::Extra => check_extra(writer, path)? && check_default(writer, path)?,
        PathchkMode::Both => check_extra(writer, path)? && check_basic(writer, path)?,
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
fn check_basic<W: Write>(writer: &mut W, path: &OsStr) -> CTResult<bool> {
    let path_bytes = path.as_bytes();
    let total_len = path_bytes.len();

    // First check empty path
    if total_len == 0 {
        writeln!(writer, "pathchk: empty file name")?;
        return Ok(false);
    }

    if !check_portable_chars(writer, path)? {
        return Ok(false);
    }

    // POSIX permits at most PATHCHK_POSIX_PATH_MAX - 1 bytes in a path.
    if total_len >= PATHCHK_POSIX_PATH_MAX {
        write!(
            writer,
            "pathchk: limit {} exceeded by length {total_len} of file name ",
            PATHCHK_POSIX_PATH_MAX - 1
        )?;
        write_shell_quoted_path(writer, path, true)?;
        writer.write_all(b"\n")?;
        return Ok(false);
    }

    // Then check the portable component length limit.
    for component in path_components(path_bytes) {
        // Only check length after character validation
        let component_len = component.len();
        if component_len > PATHCHK_POSIX_NAME_MAX {
            write!(
                writer,
                "pathchk: limit {PATHCHK_POSIX_NAME_MAX} exceeded by length {component_len} of file name component "
            )?;
            writer.write_all(&locale_quote_bytes(component))?;
            writer.write_all(b"\n")?;
            return Ok(false);
        }
    }

    Ok(true)
}

/// 执行额外的兼容性检查（空名称和前导连字符）
///
/// # 参数
/// * `writer` - 输出写入器
/// * `path` - 待检查的路径组件
///
/// # 返回
/// * `CTResult<bool>` - 检查结果，true 表示通过检查
fn check_extra<W: Write>(writer: &mut W, path: &OsStr) -> CTResult<bool> {
    // components: leading hyphens
    for component in path_components(path.as_bytes()) {
        if component.starts_with(b"-") {
            writer.write_all(b"pathchk: leading '-' in a component of file name ")?;
            write_shell_quoted_path(writer, path, true)?;
            writer.write_all(b"\n")?;
            return Ok(false);
        }
    }
    // path length
    if path.as_bytes().is_empty() {
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
fn check_default<W: Write>(writer: &mut W, path: &OsStr) -> CTResult<bool> {
    let path_bytes = path.as_bytes();
    let total_len = path_bytes.len();

    // Then check path length
    if total_len > libc::PATH_MAX as usize {
        write!(
            writer,
            "pathchk: limit {} exceeded by length {} of file name ",
            libc::PATH_MAX,
            total_len
        )?;
        write_shell_quoted_path(writer, path, true)?;
        writer.write_all(b"\n")?;
        return Ok(false);
    }

    // Check components length
    for component in path_components(path_bytes) {
        let component_len = component.len();
        if component_len > libc::FILENAME_MAX as usize {
            write!(
                writer,
                "pathchk: limit {} exceeded by length {} of file name component ",
                libc::FILENAME_MAX,
                component_len
            )?;
            writer.write_all(&locale_quote_bytes(component))?;
            writer.write_all(b"\n")?;
            return Ok(false);
        }
    }

    // Finally do permission checks
    check_searchable(writer, path)
}

/// 检查路径是否可搜索或是否存在其他问题
///
/// # 参数
/// * `writer` - 输出写入器
/// * `path` - 待检查的路径
///
/// # 返回
/// * `CTResult<bool>` - 检查结果，true 表示通过检查
fn check_searchable<W: Write>(writer: &mut W, path: &OsStr) -> CTResult<bool> {
    match fs::symlink_metadata(Path::new(path)) {
        Ok(_) => Ok(true),
        Err(e) => {
            if e.kind() == ErrorKind::NotFound && !path.as_bytes().is_empty() {
                Ok(true)
            } else if e.raw_os_error() == Some(36) {
                // ENAMETOOLONG
                writer.write_all(b"pathchk: ")?;
                write_shell_quoted_path(writer, path, true)?;
                writer.write_all(b": File name too long\n")?;
                Ok(false)
            } else {
                let message = e.to_string();
                let message = if let Some(pos) = message.find(" (os error ") {
                    &message[..pos]
                } else {
                    message.as_str()
                };
                writer.write_all(b"pathchk: ")?;
                write_shell_quoted_path(writer, path, true)?;
                writeln!(writer, ": {message}")?;
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
fn check_portable_chars<W: Write>(writer: &mut W, path: &OsStr) -> CTResult<bool> {
    const VALID_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-";
    for (index, byte) in path.as_bytes().iter().enumerate() {
        if *byte != b'/' && !VALID_CHARS.contains(byte) {
            let invalid =
                &path.as_bytes()[index..index + locale_character_len(&path.as_bytes()[index..])];
            writer.write_all(b"pathchk: non-portable character ")?;
            writer.write_all(&locale_quote_bytes(invalid))?;
            writer.write_all(b" in file name ")?;
            write_shell_quoted_path(writer, path, true)?;
            writer.write_all(b"\n")?;
            return Ok(false);
        }
    }
    Ok(true)
}

fn path_components(path: &[u8]) -> impl Iterator<Item = &[u8]> {
    path.split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
}

fn write_shell_quoted_path<W: Write>(
    writer: &mut W,
    path: &OsStr,
    always_quote: bool,
) -> std::io::Result<()> {
    writer.write_all(&gnu_quote_shell_bytes(path, always_quote))
}

fn locale_quote_bytes(bytes: &[u8]) -> Vec<u8> {
    let (left_quote, right_quote) = locale_quote_marks();
    let mut quoted = Vec::with_capacity(bytes.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(left_quote.as_bytes());

    if let Ok(value) = std::str::from_utf8(bytes) {
        if (left_quote, right_quote) == ("‘", "’") {
            for character in value.chars() {
                match character {
                    '\x07' => quoted.extend_from_slice(b"\\a"),
                    '\x08' => quoted.extend_from_slice(b"\\b"),
                    '\t' => quoted.extend_from_slice(b"\\t"),
                    '\n' => quoted.extend_from_slice(b"\\n"),
                    '\x0b' => quoted.extend_from_slice(b"\\v"),
                    '\x0c' => quoted.extend_from_slice(b"\\f"),
                    '\r' => quoted.extend_from_slice(b"\\r"),
                    '\\' => quoted.extend_from_slice(b"\\\\"),
                    '’' => quoted.extend_from_slice(b"\\\xE2\x80\x99"),
                    _ => {
                        let mut encoded = [0; 4];
                        quoted.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
                    }
                }
            }
            quoted.extend_from_slice(right_quote.as_bytes());
            return quoted;
        }
    }

    for byte in bytes {
        match *byte {
            b'\x07' => quoted.extend_from_slice(b"\\a"),
            b'\x08' => quoted.extend_from_slice(b"\\b"),
            b'\t' => quoted.extend_from_slice(b"\\t"),
            b'\n' => quoted.extend_from_slice(b"\\n"),
            b'\x0b' => quoted.extend_from_slice(b"\\v"),
            b'\x0c' => quoted.extend_from_slice(b"\\f"),
            b'\r' => quoted.extend_from_slice(b"\\r"),
            b'\\' => quoted.extend_from_slice(b"\\\\"),
            b'\'' if right_quote == "'" => quoted.extend_from_slice(b"\\'"),
            byte if byte.is_ascii_graphic() || byte == b' ' => quoted.push(byte),
            byte => quoted.extend(format!("\\{byte:03o}").bytes()),
        }
    }
    quoted.extend_from_slice(right_quote.as_bytes());
    quoted
}

fn locale_character_len(bytes: &[u8]) -> usize {
    if bytes.first().is_some_and(u8::is_ascii) {
        return 1;
    }

    match std::str::from_utf8(bytes) {
        Ok(value) => value.chars().next().map_or(1, char::len_utf8),
        Err(error) => error.error_len().unwrap_or(1),
    }
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
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

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

        #[test]
        fn test_flags_stop_parsing_options_after_first_path() {
            let args = vec![ctcore::ct_util_name(), "name#", "-p"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = PathchkFlags::new(&matches).unwrap();

            assert!(matches!(flags.mode, PathchkMode::Default));
            assert_eq!(flags.paths, vec!["name#", "-p"]);
        }
    }

    mod pathchk_execution_tests {
        use super::*;

        #[test]
        fn test_nonportable_character() {
            let args = [ctcore::ct_util_name(), "-p", "special#file"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(
                output_str
                    .contains("pathchk: non-portable character ‘#’ in file name 'special#file'")
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_non_utf8_path_is_checked_as_portability_violation() {
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("-p"),
                OsString::from_vec(vec![0xff]),
            ];
            let mut output = Cursor::new(Vec::new());

            let result = pathchk_main(&mut output, args.into_iter());

            assert!(result.is_ok(), "non-UTF-8 path was rejected: {result:?}");
            assert!(
                output
                    .into_inner()
                    .windows(b"non-portable character".len())
                    .any(|window| window == b"non-portable character"),
                "expected a portability diagnostic"
            );
        }

        #[test]
        fn test_component_too_long() {
            let long_name = "a".repeat(15);
            let args = [ctcore::ct_util_name(), "-p", &long_name];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("limit 14 exceeded by length 15"));
        }

        #[test]
        fn test_posix_total_length_limit_precedes_component_limit() {
            let path = "a".repeat(PATHCHK_POSIX_PATH_MAX);
            let mut output = Cursor::new(Vec::new());

            assert!(!check_basic(&mut output, OsStr::new(&path)).unwrap());

            let output = String::from_utf8(output.into_inner()).unwrap();
            assert!(
                output.contains("limit 255 exceeded by length 256 of file name"),
                "unexpected diagnostic: {output}"
            );
        }

        #[test]
        fn test_leading_hyphen() {
            let args = [ctcore::ct_util_name(), "-P", "-"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("leading '-' in a component of file name '-'"));
        }

        #[test]
        fn test_portability_mode_checks_leading_hyphen_before_portable_characters() {
            let args = [ctcore::ct_util_name(), "-p", "-P", "--", "-#"];
            let mut output = Cursor::new(Vec::new());

            assert!(pathchk_main(&mut output, args.iter().map(OsString::from)).is_ok());

            let output = String::from_utf8(output.into_inner()).unwrap();
            assert!(output.contains("leading '-' in a component of file name '-#'"));
            assert!(!output.contains("non-portable character"));
        }

        #[test]
        fn test_empty_filename() {
            let args = [ctcore::ct_util_name(), "-P", ""];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();

            assert!(output_str.contains("pathchk: empty file name"));
        }

        #[test]
        fn test_default_mode_empty_path_reports_lstat_error() {
            let args = [ctcore::ct_util_name(), ""];
            let mut output = Cursor::new(Vec::new());

            assert!(pathchk_main(&mut output, args.iter().map(OsString::from)).is_ok());

            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "pathchk: '': No such file or directory\n"
            );
        }

        #[test]
        fn test_filename_too_long() {
            let long_name = "a".repeat(300);
            let args = [ctcore::ct_util_name(), "-P", &long_name];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("File name too long"));
            assert!(output_str.contains(&long_name));
        }

        #[test]
        fn test_path_with_multiple_components() {
            let args = [ctcore::ct_util_name(), "-p", "dir1/dir2/file"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_posix_mode_does_not_check_path_searchability() {
            let name = format!("pchk{}", std::process::id());
            std::fs::write(&name, b"regular file").unwrap();
            let path = format!("{name}/child");
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("-p"),
                OsString::from(&path),
            ];
            let mut output = Cursor::new(Vec::new());

            let result = pathchk_main(&mut output, args.into_iter());

            std::fs::remove_file(&name).unwrap();
            assert!(result.is_ok());
            assert!(output.into_inner().is_empty());
        }

        #[test]
        fn test_multiple_paths() {
            let args = [ctcore::ct_util_name(), "-p", "file1", "file2"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_path_with_dots() {
            let args = [ctcore::ct_util_name(), "-p", "../file"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_path_with_special_chars() {
            let args = [ctcore::ct_util_name(), "-p", "file@name"];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("pathchk: non-portable character ‘@’"));
        }

        #[test]
        fn test_path_with_long_component() {
            let long_component = format!("dir/{}", "a".repeat(15));
            let args = [ctcore::ct_util_name(), "-p", &long_component];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("limit 14 exceeded"));
        }

        #[test]
        fn test_path_with_long_total_length() {
            let long_path = format!("{}/file", "a".repeat(255));
            let args = [ctcore::ct_util_name(), "-p", &long_path];
            let mut output = Cursor::new(Vec::new());
            let result = pathchk_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();

            assert!(output_str.contains("limit 255 exceeded by length 260 of file name"));
        }
    }

    mod check_functions_tests {
        use super::*;

        #[test]
        fn test_check_portable_chars() {
            let mut output = Cursor::new(Vec::new());
            assert!(check_portable_chars(&mut output, OsStr::new("valid-name.txt")).unwrap());
            assert!(!check_portable_chars(&mut output, OsStr::new("invalid#name")).unwrap());
            assert!(!check_portable_chars(&mut output, OsStr::new("name@domain")).unwrap());
        }

        #[test]
        fn test_check_searchable() {
            let mut output = Cursor::new(Vec::new());
            assert!(check_searchable(&mut output, OsStr::new(".")).unwrap());
            assert!(check_searchable(&mut output, OsStr::new("nonexistent_file")).unwrap());
        }

        #[test]
        fn test_check_extra_with_multiple_components() {
            let mut output = Cursor::new(Vec::new());
            assert!(!check_extra(&mut output, OsStr::new("-bad/name")).unwrap());
        }

        #[test]
        fn test_check_basic_with_valid_path() {
            let mut output = Cursor::new(Vec::new());
            assert!(check_basic(&mut output, OsStr::new("valid/path.txt")).unwrap());
        }

        #[test]
        fn test_check_default_with_valid_path() {
            let mut output = Cursor::new(Vec::new());
            assert!(check_default(&mut output, OsStr::new("valid/path.txt")).unwrap());
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
            let tool = Pathchk;

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
