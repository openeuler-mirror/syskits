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
use ctcore::ct_error::{CTError, CTResult, CTsageError, set_ct_exit_code};
use ctcore::ct_posix::GnuGetoptCommandExt;
use ctcore::ct_quoting_style::gnu_quote_shell_bytes;
use std::ffi::{CString, OsStr, OsString};
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use sys_locale::get_locale;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn mbrtowc(
        wide: *mut libc::wchar_t,
        bytes: *const libc::c_char,
        length: usize,
        state: *mut libc::mbstate_t,
    ) -> usize;
    fn iswprint(wide: libc::c_uint) -> libc::c_int;
}

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
const PATHCHK_PATH_MAX_MINIMUM: usize = PATHCHK_POSIX_PATH_MAX;
const PATHCHK_NAME_MAX_MINIMUM: usize = PATHCHK_POSIX_NAME_MAX;
const PATHCHK_SHORT_OPTIONS: &[u8] = b"pP";
const PATHCHK_LONG_OPTIONS: &[&str] = &["portability", "help", "version"];

#[derive(Debug)]
struct PathchkUsageError {
    message: Vec<u8>,
}

impl PathchkUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

impl std::fmt::Display for PathchkUsageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl std::error::Error for PathchkUsageError {}

impl CTError for PathchkUsageError {
    fn diagnostic_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(&self.message)
    }

    fn code(&self) -> i32 {
        1
    }

    fn usage(&self) -> bool {
        true
    }
}

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
    let matches = parse_pathchk_args(args)?;
    // 解析参数到 PathchkFlags 结构体
    let flags = PathchkFlags::new(&matches)?;

    pathchk_exec(writer, &flags)
}

pub fn pathchk_native_semantic(args: impl ctcore::Args) -> CTResult<PathchkSemantic> {
    initialize_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = parse_pathchk_args(args)?;
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

fn parse_pathchk_args(args: impl ctcore::Args) -> CTResult<ArgMatches> {
    let args = args.collect::<Vec<_>>();
    validate_pathchk_options(&args)?;
    Ok(ct_app().try_get_matches_from(args)?)
}

fn validate_pathchk_options(args: &[OsString]) -> CTResult<()> {
    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if bytes == b"--" || bytes.len() <= 1 || bytes[0] != b'-' {
            return Ok(());
        }

        if bytes.starts_with(b"--") {
            let long = &bytes[2..];
            let separator = long.iter().position(|byte| *byte == b'=');
            let name = &long[..separator.unwrap_or(long.len())];
            let canonical = (!name.is_empty()).then(|| {
                PATHCHK_LONG_OPTIONS
                    .iter()
                    .copied()
                    .find(|option| option.as_bytes().starts_with(name))
            });
            let Some(canonical) = canonical.flatten() else {
                let mut message = b"unrecognized option '".to_vec();
                message.extend_from_slice(bytes);
                message.push(b'\'');
                return Err(PathchkUsageError::boxed(message));
            };

            if separator.is_some() {
                return Err(PathchkUsageError::boxed(
                    format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
                ));
            }
            if matches!(canonical, "help" | "version") {
                return Ok(());
            }
        } else if let Some(unknown) = bytes[1..]
            .iter()
            .find(|option| !PATHCHK_SHORT_OPTIONS.contains(option))
        {
            let mut message = b"invalid option -- '".to_vec();
            message.push(*unknown);
            message.push(b'\'');
            return Err(PathchkUsageError::boxed(message));
        }
    }

    Ok(())
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
            .action(ArgAction::SetTrue)
            .overrides_with(pathchk_flags::PATHCHK_POSIX),
        Arg::new(pathchk_flags::PATHCHK_POSIX_SPECIAL)
            .short('P')
            .help(r#"check for empty names and leading "-""#)
            .action(ArgAction::SetTrue)
            .overrides_with(pathchk_flags::PATHCHK_POSIX_SPECIAL),
        Arg::new(pathchk_flags::PATHCHK_PORTABILITY)
            .long(pathchk_flags::PATHCHK_PORTABILITY)
            .help(t!("pathchk.clap.pathchk_portability"))
            .action(ArgAction::SetTrue)
            .overrides_with(pathchk_flags::PATHCHK_PORTABILITY),
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

    match fs::symlink_metadata(Path::new(path)) {
        Ok(_) => return Ok(true),
        Err(error) if error.raw_os_error() == Some(libc::ENOENT) && !path_bytes.is_empty() => {}
        Err(error) => return write_path_error(writer, path, &error),
    }

    check_default_limits(writer, path, &mut system_pathconf)
}

fn check_default_limits<W: Write, F>(
    writer: &mut W,
    path: &OsStr,
    pathconf: &mut F,
) -> CTResult<bool>
where
    F: FnMut(&OsStr, libc::c_int) -> std::io::Result<Option<usize>>,
{
    let path_bytes = path.as_bytes();
    let total_len = path_bytes.len();

    if PATHCHK_PATH_MAX_MINIMUM <= total_len {
        let directory = if path_bytes.starts_with(b"/") {
            OsStr::new("/")
        } else {
            OsStr::new(".")
        };
        let maximum = match pathconf(directory, libc::_PC_PATH_MAX) {
            Ok(Some(limit)) => limit.min(isize::MAX as usize) as isize,
            // GNU preserves pathconf's -1 return here, even when errno is
            // zero.  Its signed comparison then reports a limit of -2.
            Ok(None) => -1,
            Err(error) => return write_pathconf_error(writer, directory, &error),
        };

        if maximum <= total_len as isize {
            write!(
                writer,
                "pathchk: limit {} exceeded by length {total_len} of file name ",
                maximum - 1
            )?;
            write_shell_quoted_path(writer, path, true)?;
            writer.write_all(b"\n")?;
            return Ok(false);
        }
    }

    if !path_components(path_bytes).any(|component| component.len() > PATHCHK_NAME_MAX_MINIMUM) {
        return Ok(true);
    }

    let mut offset = 0;
    let mut name_max = PATHCHK_NAME_MAX_MINIMUM;
    let mut known_name_max = None;
    while offset < path_bytes.len() {
        while offset < path_bytes.len() && path_bytes[offset] == b'/' {
            offset += 1;
        }
        if offset == path_bytes.len() {
            break;
        }

        let component_start = offset;
        while offset < path_bytes.len() && path_bytes[offset] != b'/' {
            offset += 1;
        }
        let component = &path_bytes[component_start..offset];

        if let Some(limit) = known_name_max {
            name_max = limit;
        } else {
            let directory = if component_start == 0 {
                OsStr::new(".")
            } else {
                OsStr::from_bytes(&path_bytes[..component_start])
            };
            match pathconf(directory, libc::_PC_NAME_MAX) {
                Ok(Some(limit)) => name_max = limit,
                Ok(None) => name_max = usize::MAX,
                Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
                    known_name_max = Some(name_max);
                }
                Err(error) => return write_path_error(writer, directory, &error),
            }
        }

        if name_max < component.len() {
            write!(
                writer,
                "pathchk: limit {name_max} exceeded by length {} of file name component ",
                component.len()
            )?;
            writer.write_all(&locale_quote_bytes(component))?;
            writer.write_all(b"\n")?;
            return Ok(false);
        }
    }

    Ok(true)
}

fn system_pathconf(path: &OsStr, variable: libc::c_int) -> std::io::Result<Option<usize>> {
    let path = CString::new(path.as_bytes()).expect("Unix paths cannot contain NUL bytes");
    unsafe {
        *libc::__errno_location() = 0;
        let limit = libc::pathconf(path.as_ptr(), variable);
        if limit >= 0 {
            Ok(Some(limit as usize))
        } else {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(0) {
                Ok(None)
            } else {
                Err(error)
            }
        }
    }
}

fn write_pathconf_error<W: Write>(
    writer: &mut W,
    directory: &OsStr,
    error: &std::io::Error,
) -> CTResult<bool> {
    let message = error.to_string();
    let message = if let Some(pos) = message.find(" (os error ") {
        &message[..pos]
    } else {
        message.as_str()
    };
    writer.write_all(b"pathchk: ")?;
    write_shell_quoted_path(writer, directory, false)?;
    writeln!(
        writer,
        ": unable to determine maximum file name length: {message}"
    )?;
    Ok(false)
}

fn write_path_error<W: Write>(
    writer: &mut W,
    path: &OsStr,
    error: &std::io::Error,
) -> CTResult<bool> {
    if error.raw_os_error() == Some(libc::ENAMETOOLONG) {
        writer.write_all(b"pathchk: ")?;
        write_shell_quoted_path(writer, path, false)?;
        writer.write_all(b": File name too long\n")?;
    } else {
        let message = error.to_string();
        let message = if let Some(pos) = message.find(" (os error ") {
            &message[..pos]
        } else {
            message.as_str()
        };
        writer.write_all(b"pathchk: ")?;
        write_shell_quoted_path(writer, path, false)?;
        writeln!(writer, ": {message}")?;
    }
    Ok(false)
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
    let (left_quote, right_quote) = pathchk_locale_quote_marks();
    let mut quoted = Vec::with_capacity(bytes.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(left_quote);

    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'\x07' => quoted.extend_from_slice(b"\\a"),
            b'\x08' => quoted.extend_from_slice(b"\\b"),
            b'\t' => quoted.extend_from_slice(b"\\t"),
            b'\n' => quoted.extend_from_slice(b"\\n"),
            b'\x0b' => quoted.extend_from_slice(b"\\v"),
            b'\x0c' => quoted.extend_from_slice(b"\\f"),
            b'\r' => quoted.extend_from_slice(b"\\r"),
            b'\\' => quoted.extend_from_slice(b"\\\\"),
            byte if byte.is_ascii() => {
                if byte == right_quote.first().copied().unwrap_or_default()
                    && right_quote.len() == 1
                {
                    quoted.push(b'\\');
                }
                if byte.is_ascii_graphic() || byte == b' ' {
                    quoted.push(byte);
                } else {
                    quoted.extend(format!("\\{byte:03o}").bytes());
                }
            }
            _ => {
                let (length, printable) = locale_character(bytes, index);
                let character = &bytes[index..index + length];
                if printable {
                    if character == right_quote {
                        quoted.push(b'\\');
                    }
                    quoted.extend_from_slice(character);
                } else {
                    for byte in character {
                        quoted.extend(format!("\\{byte:03o}").bytes());
                    }
                }
                index += length;
                continue;
            }
        }
        index += 1;
    }
    quoted.extend_from_slice(right_quote);
    quoted
}

fn locale_character_len(bytes: &[u8]) -> usize {
    locale_character(bytes, 0).0
}

fn locale_character(bytes: &[u8], start: usize) -> (usize, bool) {
    if bytes.get(start).is_some_and(u8::is_ascii) {
        return (1, false);
    }

    #[cfg(target_os = "linux")]
    unsafe {
        let remaining = &bytes[start..];
        let mut state: libc::mbstate_t = std::mem::zeroed();
        let mut wide = 0 as libc::wchar_t;
        let length = mbrtowc(
            &mut wide,
            remaining.as_ptr().cast(),
            remaining.len(),
            &mut state,
        );
        if length == usize::MAX || length == usize::MAX - 1 || length == 0 {
            return (1, false);
        }
        (
            length.min(remaining.len()),
            iswprint(wide as libc::c_uint) != 0,
        )
    }

    #[cfg(not(target_os = "linux"))]
    {
        (1, false)
    }
}

fn pathchk_locale_quote_marks() -> (&'static [u8], &'static [u8]) {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
        .unwrap_or_default();
    let locale = locale
        .to_string_lossy()
        .replace('-', "_")
        .to_ascii_lowercase();
    if locale.starts_with("zh_cn") {
        return (b"\"", b"\"");
    }

    let (left_quote, right_quote) = locale_quote_marks();
    (left_quote.as_bytes(), right_quote.as_bytes())
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
        fn test_flags_accept_repeated_boolean_options() {
            for args in [
                vec![ctcore::ct_util_name(), "-pp", "file.txt"],
                vec![ctcore::ct_util_name(), "-PP", "file.txt"],
                vec![
                    ctcore::ct_util_name(),
                    "--portability",
                    "--portability",
                    "file.txt",
                ],
            ] {
                let matches = ct_app().try_get_matches_from(args).unwrap();
                let flags = PathchkFlags::new(&matches).unwrap();
                assert_eq!(flags.paths, vec!["file.txt"]);
            }
        }

        #[test]
        fn test_short_option_cluster_uses_gnu_invalid_option_diagnostic() {
            let mut output = Cursor::new(Vec::new());
            let error = pathchk_main(
                &mut output,
                [ctcore::ct_util_name(), "-pz"]
                    .map(OsString::from)
                    .into_iter(),
            )
            .unwrap_err();

            assert_eq!(error.diagnostic_bytes().as_ref(), b"invalid option -- 'z'");
        }

        #[test]
        fn test_long_option_value_uses_canonical_gnu_diagnostic() {
            let mut output = Cursor::new(Vec::new());
            let error = pathchk_main(
                &mut output,
                [ctcore::ct_util_name(), "--p=value"]
                    .map(OsString::from)
                    .into_iter(),
            )
            .unwrap_err();

            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"option '--portability' doesn't allow an argument"
            );
        }

        #[test]
        fn test_empty_long_option_name_is_unrecognized() {
            let mut output = Cursor::new(Vec::new());
            let error = pathchk_main(
                &mut output,
                [ctcore::ct_util_name(), "--=value"]
                    .map(OsString::from)
                    .into_iter(),
            )
            .unwrap_err();

            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"unrecognized option '--=value'"
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_non_utf8_short_option_preserves_original_byte() {
            let mut output = Cursor::new(Vec::new());
            let error = pathchk_main(
                &mut output,
                [
                    OsString::from(ctcore::ct_util_name()),
                    OsString::from_vec(vec![b'-', 0xff]),
                ]
                .into_iter(),
            )
            .unwrap_err();

            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"invalid option -- '\xff'"
            );
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
        fn test_default_mode_reports_lstat_error_before_static_length_limit() {
            let path = "a".repeat((libc::PATH_MAX as usize) + 1);
            let mut output = Cursor::new(Vec::new());

            assert!(!check_default(&mut output, OsStr::new(&path)).unwrap());

            let output = String::from_utf8(output.into_inner()).unwrap();
            assert!(output.ends_with(": File name too long\n"));
            assert!(!output.contains("limit"));
        }

        #[test]
        fn test_default_mode_lstat_error_uses_non_always_shell_quoting() {
            let name = format!("pchk{}", std::process::id());
            std::fs::write(&name, b"regular file").unwrap();
            let path = format!("{name}/child");
            let mut output = Cursor::new(Vec::new());

            assert!(!check_default(&mut output, OsStr::new(&path)).unwrap());

            std::fs::remove_file(&name).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                format!("pathchk: {path}: Not a directory\n")
            );
        }

        #[test]
        fn test_default_limits_use_pathconf_path_max() {
            let component = "a".repeat(PATHCHK_POSIX_NAME_MAX);
            let mut components = vec![component; 17];
            components.push("a".to_string());
            let path = components.join("/");
            assert_eq!(path.len(), PATHCHK_POSIX_PATH_MAX);
            let mut output = Cursor::new(Vec::new());

            assert!(
                !check_default_limits(&mut output, OsStr::new(&path), &mut |_, variable| {
                    if variable == libc::_PC_PATH_MAX {
                        Ok(Some(32))
                    } else {
                        Ok(Some(PATHCHK_POSIX_NAME_MAX))
                    }
                },)
                .unwrap()
            );

            let output = String::from_utf8(output.into_inner()).unwrap();
            assert!(output.contains("limit 31 exceeded by length 256 of file name"));
        }

        #[test]
        fn test_default_limits_preserve_unlimited_path_max_as_gnu_negative_limit() {
            let component = "a".repeat(PATHCHK_POSIX_NAME_MAX);
            let mut components = vec![component; 17];
            components.push("a".to_string());
            let path = components.join("/");
            assert_eq!(path.len(), PATHCHK_POSIX_PATH_MAX);
            let mut output = Cursor::new(Vec::new());

            assert!(
                !check_default_limits(&mut output, OsStr::new(&path), &mut |_, variable| {
                    if variable == libc::_PC_PATH_MAX {
                        Ok(None)
                    } else {
                        Ok(Some(PATHCHK_POSIX_NAME_MAX))
                    }
                },)
                .unwrap()
            );

            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                format!("pathchk: limit -2 exceeded by length 256 of file name '{path}'\n")
            );
        }

        #[test]
        fn test_default_limits_use_pathconf_name_max() {
            let path = "a".repeat(PATHCHK_POSIX_NAME_MAX + 1);
            let mut output = Cursor::new(Vec::new());

            assert!(
                !check_default_limits(&mut output, OsStr::new(&path), &mut |_, variable| {
                    if variable == libc::_PC_NAME_MAX {
                        Ok(Some(PATHCHK_POSIX_NAME_MAX))
                    } else {
                        Ok(Some(usize::MAX))
                    }
                },)
                .unwrap()
            );

            let output = String::from_utf8(output.into_inner()).unwrap();
            assert!(output.contains("limit 14 exceeded by length 15 of file name component"));
        }

        #[test]
        fn test_default_limits_name_max_pathconf_error_is_plain_directory_error() {
            let path = "a".repeat(PATHCHK_POSIX_NAME_MAX + 1);
            let mut output = Cursor::new(Vec::new());

            assert!(
                !check_default_limits(&mut output, OsStr::new(&path), &mut |_, variable| {
                    if variable == libc::_PC_NAME_MAX {
                        Err(std::io::Error::from_raw_os_error(libc::EACCES))
                    } else {
                        Ok(Some(4096))
                    }
                },)
                .unwrap()
            );

            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "pathchk: .: Permission denied\n"
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

        #[cfg(target_os = "linux")]
        #[test]
        fn test_locale_quote_bytes_preserves_printable_gbk_character() {
            let locale_name = CString::new("zh_CN.gbk").unwrap();
            let locale = unsafe {
                libc::newlocale(
                    libc::LC_CTYPE_MASK,
                    locale_name.as_ptr(),
                    std::ptr::null_mut(),
                )
            };
            assert!(!locale.is_null(), "zh_CN.gbk locale must be available");

            let previous = unsafe { libc::uselocale(locale) };
            let quoted = locale_quote_bytes(b"\xd6\xd0");
            unsafe {
                libc::uselocale(previous);
                libc::freelocale(locale);
            }

            assert!(
                quoted.windows(2).any(|bytes| bytes == b"\xd6\xd0"),
                "valid GBK bytes were escaped instead of retained: {quoted:?}"
            );
        }

        #[test]
        fn test_check_portable_chars() {
            let mut output = Cursor::new(Vec::new());
            assert!(check_portable_chars(&mut output, OsStr::new("valid-name.txt")).unwrap());
            assert!(!check_portable_chars(&mut output, OsStr::new("invalid#name")).unwrap());
            assert!(!check_portable_chars(&mut output, OsStr::new("name@domain")).unwrap());
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
