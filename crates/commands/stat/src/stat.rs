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

extern crate rust_i18n;
use chrono::{DateTime, Local};
use clap::builder::ValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::ct_fs::display_permissions;
use ctcore::ct_fsext::{CtBirthTime, FsMeta, pretty_filetype, pretty_fstype, read_fs_list, statfs};
use ctcore::ct_quoting_style::{CtQuotes, CtQuotingStyle, escape_name};
use ctcore::libc::{self, mode_t};
use ctcore::{ct_entries, ct_show_error, ct_show_warning};
use rustix::fs::{AtFlags, StatxFlags, major, minor, statx};
use std::borrow::Cow;
use std::ffi::{CStr, OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::os::fd::FromRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::prelude::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

// 声明 i18n 宏和初始化函数
rust_i18n::i18n!("locales", fallback = "en-US");
use rust_i18n::t;
use sys_locale::get_locale;

mod stat_options {
    pub const STAT_DEREFERENCE: &str = "dereference";
    pub const STAT_FILE_SYSTEM: &str = "file-system";
    pub const STAT_FORMAT: &str = "format";
    pub const STAT_PRINTF: &str = "printf";
    pub const STAT_TERSE: &str = "terse";
    pub const STAT_FILES: &str = "files";
    pub const STAT_HELP: &str = "help";
    pub const STAT_VERSION: &str = "version";
    pub const STAT_ABOUT: &str = "about";
    pub const STAT_USAGE: &str = "usage";
    pub const STAT_LONG_USAGE: &str = "long_usage";
    pub const STAT_CACHED: &str = "cached";
}

// 添加缓存模式的枚举类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CachedMode {
    Default,
    Never,
    Always,
}

impl CachedMode {
    fn from_str(s: &str) -> Result<Self, String> {
        if !s.is_empty() && "default".starts_with(s) {
            Ok(CachedMode::Default)
        } else if !s.is_empty() && "never".starts_with(s) {
            Ok(CachedMode::Never)
        } else if !s.is_empty() && "always".starts_with(s) {
            Ok(CachedMode::Always)
        } else {
            Err(format!("invalid cached mode: {s}"))
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq, Clone, Copy)]
struct StatFlags {
    is_alter: bool,
    is_zero: bool,
    is_left: bool,
    is_space: bool,
    is_sign: bool,
    is_group: bool,
    is_locale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatQuotingStyle {
    Common(CtQuotingStyle),
    Shell { escape: bool, always_quote: bool },
    CMaybe,
    Escape,
    Locale { clocale: bool },
}

impl StatQuotingStyle {
    fn parse(style: &str) -> Option<Self> {
        let common = |style| Self::Common(style);
        match style {
            "literal" => Some(common(CtQuotingStyle::Literal { show_control: true })),
            "shell" => Some(Self::Shell {
                escape: false,
                always_quote: false,
            }),
            "shell-always" => Some(Self::Shell {
                escape: false,
                always_quote: true,
            }),
            "shell-escape" => Some(Self::Shell {
                escape: true,
                always_quote: false,
            }),
            "shell-escape-always" => Some(Self::shell_escape_always()),
            "c" => Some(common(CtQuotingStyle::C {
                quotes: CtQuotes::Double,
            })),
            "c-maybe" => Some(Self::CMaybe),
            "escape" => Some(Self::Escape),
            "locale" => Some(Self::Locale { clocale: false }),
            "clocale" => Some(Self::Locale { clocale: true }),
            _ => None,
        }
    }

    fn literal() -> Self {
        Self::Common(CtQuotingStyle::Literal { show_control: true })
    }

    fn shell_escape_always() -> Self {
        Self::Shell {
            escape: true,
            always_quote: true,
        }
    }

    fn quote(self, name: &str) -> String {
        match self {
            Self::Common(style) => escape_name(OsStr::new(name), &style),
            Self::Shell {
                escape,
                always_quote,
            } => {
                let style = CtQuotingStyle::Shell {
                    escape,
                    always_quote,
                    show_control: true,
                };
                let escaped = escape_name(OsStr::new(name), &style);
                if !escape && !always_quote && escaped == name && name.chars().any(char::is_control)
                {
                    format!("'{escaped}'")
                } else {
                    escaped
                }
            }
            Self::CMaybe => {
                let escaped = escape_quoting_component(name, Some('"'));
                if escaped == name {
                    name.to_string()
                } else {
                    format!("\"{escaped}\"")
                }
            }
            Self::Escape => escape_quoting_component(name, None),
            Self::Locale { clocale } => {
                let (left, right) = locale_quoting_marks(clocale);
                let escaped = escape_quoting_component(name, right.chars().next());
                format!("{left}{escaped}{right}")
            }
        }
    }
}

fn escape_quoting_component(name: &str, quote_to_escape: Option<char>) -> String {
    let mut escaped = String::with_capacity(name.len());
    for character in name.chars() {
        match character {
            '\x07' => escaped.push_str("\\a"),
            '\x08' => escaped.push_str("\\b"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\x0b' => escaped.push_str("\\v"),
            '\x0c' => escaped.push_str("\\f"),
            '\r' => escaped.push_str("\\r"),
            '\\' => escaped.push_str("\\\\"),
            c if Some(c) == quote_to_escape => {
                escaped.push('\\');
                escaped.push(c);
            }
            c => escaped.push(c),
        }
    }
    escaped
}

fn locale_quoting_marks(clocale: bool) -> (&'static str, &'static str) {
    let message_locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default()
        .to_ascii_uppercase();
    if message_locale.starts_with("ZH_CN") {
        return ("\"", "\"");
    }

    // SAFETY: nl_langinfo returns a process-owned NUL-terminated string after setlocale.
    let codeset = unsafe { CStr::from_ptr(libc::nl_langinfo(libc::CODESET)) }
        .to_string_lossy()
        .to_ascii_uppercase();
    if matches!(codeset.as_str(), "UTF-8" | "UTF8") {
        ("‘", "’")
    } else if clocale {
        ("\"", "\"")
    } else {
        ("'", "'")
    }
}

fn device_major(device: u64) -> u64 {
    u64::from(major(device))
}

fn device_minor(device: u64) -> u64 {
    u64::from(minor(device))
}

fn metadata_for_fd(fd: libc::c_int) -> std::io::Result<fs::Metadata> {
    // SAFETY: fcntl only reads the supplied descriptor and returns a new owned descriptor.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: duplicate is a fresh descriptor owned by this function.
    let file = unsafe { fs::File::from_raw_fd(duplicate) };
    file.metadata()
}

static STDIN_WAS_CLOSED_AT_STARTUP: AtomicBool = AtomicBool::new(false);

extern "C" fn record_initial_stdin_state() {
    // SAFETY: F_GETFD only queries the descriptor and runs before Rust initializes stdio.
    let result = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFD) };
    if result < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EBADF) {
        STDIN_WAS_CLOSED_AT_STARTUP.store(true, Ordering::Relaxed);
    }
}

#[used]
#[unsafe(link_section = ".init_array")]
static RECORD_INITIAL_STDIN_STATE: extern "C" fn() = record_initial_stdin_state;

fn metadata_for_stdin() -> std::io::Result<fs::Metadata> {
    if STDIN_WAS_CLOSED_AT_STARTUP.load(Ordering::Relaxed) {
        return Err(std::io::Error::from_raw_os_error(libc::EBADF));
    }
    metadata_for_fd(libc::STDIN_FILENO)
}

#[derive(Debug)]
pub enum StatOutputType {
    Str(String),
    Integer(i64),
    Unsigned(u64),
    UnsignedHex(u64),
    UnsignedOct(u32),
    Timestamp(i64, i64),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatRowKind {
    File,
    Filesystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatSemanticField {
    Name,
    QuotedName,
    Size,
    Blocks,
    BlockSizeReported,
    IoBlock,
    RawModeHex,
    FileType,
    Uid,
    User,
    Gid,
    Group,
    Device,
    DeviceHex,
    DeviceMajor,
    DeviceMinor,
    DeviceMajorHex,
    DeviceMinorHex,
    DeviceType,
    DeviceTypeHex,
    DeviceTypeMajor,
    DeviceTypeMinor,
    DeviceTypeMajorHex,
    DeviceTypeMinorHex,
    Inode,
    Links,
    MountPoint,
    AccessRightsOctal,
    AccessRightsHuman,
    Context,
    AccessTime,
    AccessEpoch,
    ModifyTime,
    ModifyEpoch,
    ChangeTime,
    ChangeEpoch,
    BirthTime,
    BirthEpoch,
    FilesystemIdHex,
    NameMax,
    FilesystemTypeHex,
    FilesystemType,
    BlockSize,
    FundamentalBlockSize,
    TotalBlocks,
    FreeBlocks,
    AvailableBlocks,
    TotalFileNodes,
    FreeFileNodes,
    Formatted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatSemanticValue {
    String(String),
    Int(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatSemanticRow {
    pub row_kind: StatRowKind,
    pub fields: Vec<(StatSemanticField, StatSemanticValue)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatSemantic {
    pub rows: Vec<StatSemanticRow>,
    pub selected_fields: Vec<StatSemanticField>,
    pub classic_text: String,
}

#[derive(Debug, PartialEq, Eq)]
enum StatToken {
    Char(char),
    Byte(u8),
    IgnoredDirective,
    InvalidDirective(String),
    Directive {
        flag: StatFlags,
        width: usize,
        precision: Option<i32>,
        modifier: Option<char>,
        format: char,
    },
}

#[derive(Debug)]
enum StatExecutionError {
    Write(io::Error),
    InvalidDirective(String),
}

impl From<io::Error> for StatExecutionError {
    fn from(error: io::Error) -> Self {
        Self::Write(error)
    }
}

trait ScanUtil {
    fn scan_char(&self, radix: u32) -> Option<(u8, usize)>;
}

impl ScanUtil for str {
    fn scan_char(&self, radix: u32) -> Option<(u8, usize)> {
        let count = match radix {
            8 => 3,
            16 => 2,
            _ => return None,
        };
        let chars = self.chars().enumerate();
        let mut res = 0;
        let mut offset = 0;
        for (i, c) in chars {
            if i >= count {
                break;
            }
            match c.to_digit(radix) {
                Some(digit) => {
                    res = res * radix + digit;
                }
                None => break,
            }
            offset = i + 1;
        }
        if offset > 0 {
            Some((res as u8, offset))
        } else {
            None
        }
    }
}

fn group_num(s: &str) -> Cow<str> {
    let is_negative = s.starts_with('-');
    assert!(is_negative || s.chars().take(1).all(|c| c.is_ascii_digit()));
    assert!(s.chars().skip(1).all(|c| c.is_ascii_digit()));
    let locale = NumericLocale::current();
    if locale.thousands_separator.is_empty()
        || locale.grouping.is_empty()
        || matches!(locale.grouping[0], 0 | 127 | u8::MAX)
    {
        return s.into();
    }

    let (sign, digits) = if is_negative { ("-", &s[1..]) } else { ("", s) };
    if digits.is_empty() {
        return s.into();
    }

    let mut groups = Vec::new();
    let mut end = digits.len();
    let mut pattern_index = 0;
    let mut group_size = locale.grouping[0] as usize;
    while end > group_size {
        groups.push(&digits[end - group_size..end]);
        end -= group_size;
        if let Some(&next) = locale.grouping.get(pattern_index + 1) {
            match next {
                0 => {}
                127 | u8::MAX => break,
                value => {
                    pattern_index += 1;
                    group_size = value as usize;
                }
            }
        }
    }
    groups.push(&digits[..end]);
    groups.reverse();

    let mut result = String::from(sign);
    result.push_str(&groups.join(&locale.thousands_separator));
    Cow::Owned(result)
}

struct NumericLocale {
    decimal_point: String,
    thousands_separator: String,
    grouping: Vec<u8>,
}

impl NumericLocale {
    fn current() -> Self {
        unsafe {
            let locale = libc::localeconv();
            if locale.is_null() {
                return Self::c();
            }

            let decimal_point = copy_locale_string((*locale).decimal_point, ".");
            let thousands_separator = copy_locale_string((*locale).thousands_sep, "");
            let mut grouping = Vec::new();
            if !(*locale).grouping.is_null() {
                for index in 0..16 {
                    let value = *(*locale).grouping.add(index) as u8;
                    grouping.push(value);
                    if matches!(value, 0 | 127 | u8::MAX) {
                        break;
                    }
                }
            }

            Self {
                decimal_point,
                thousands_separator,
                grouping,
            }
        }
    }

    fn c() -> Self {
        Self {
            decimal_point: ".".into(),
            thousands_separator: String::new(),
            grouping: Vec::new(),
        }
    }
}

unsafe fn copy_locale_string(value: *const libc::c_char, fallback: &str) -> String {
    if value.is_null() {
        fallback.to_string()
    } else {
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    }
}

struct Stater {
    is_follow: bool,
    is_show_fs: bool,
    is_from_user: bool,
    cached_mode: CachedMode,
    files: Vec<OsString>,
    mount_list: Option<Vec<String>>,
    default_tokens: Vec<StatToken>,
    default_dev_tokens: Vec<StatToken>,
    quoting_style: StatQuotingStyle,
}

/// Prints a formatted output based on the provided output type, flags, width, and precision.
///
/// # Arguments
///
/// * `output` - A reference to the OutputType enum containing the value to be printed.
/// * `flags` - A Flags struct containing formatting flags.
/// * `width` - The width of the field for the printed output.
/// * `precision` - An Option containing the precision value.
///
/// This function delegates the printing process to more specialized functions depending on the output type.
fn print_it<W: Write>(
    writer: &mut W,
    output: &StatOutputType,
    flags: StatFlags,
    width: usize,
    precision: Option<i32>,
) -> io::Result<()> {
    // If the precision is given as just '.', the precision is taken to be zero.
    // A negative precision is taken as if the precision were omitted.
    // This gives the minimum number of digits to appear for d, i, o, u, x, and X conversions,
    // the maximum number of characters to be printed from a string for s and S conversions.

    // #
    // The value should be converted to an "alternate form".
    // For o conversions, the first character of the output string  is made  zero  (by  prefixing  a 0 if it was not zero already).
    // For x and X conversions, a nonzero result has the string "0x" (or "0X" for X conversions) prepended to it.

    // 0
    // The value should be zero padded.
    // For d, i, o, u, x, X, a, A, e, E, f, F, g, and G conversions, the converted value is padded on the left with zeros rather than blanks.
    // If the 0 and - flags both appear, the 0 flag is ignored.
    // If a precision  is  given with a numeric conversion (d, i, o, u, x, and X), the 0 flag is ignored.
    // For other conversions, the behavior is undefined.

    // -
    // The converted value is to be left adjusted on the field boundary.  (The default is right justification.)
    // The  converted  value  is padded on the right with blanks, rather than on the left with blanks or zeros.
    // A - overrides a 0 if both are given.

    // ' ' (a space)
    // A blank should be left before a positive number (or empty string) produced by a signed conversion.

    // +
    // A sign (+ or -) should always be placed before a number produced by a signed conversion.
    // By default, a sign  is  used only for negative numbers.
    // A + overrides a space if both are used.
    writer.write_all(render_output(output, flags, width, precision).as_bytes())
}

impl Stater {
    fn handle_percent_case(chars: &[char], i: &mut usize, bound: usize) -> StatToken {
        let old = *i;

        *i += 1;
        if *i >= bound {
            return StatToken::Char('%');
        }
        if chars[*i] == '%' {
            *i += 1;
            return StatToken::Char('%');
        }

        let mut flag = StatFlags::default();

        while *i < bound {
            match chars[*i] {
                '#' => flag.is_alter = true,
                '0' => flag.is_zero = true,
                '-' => flag.is_left = true,
                ' ' => flag.is_space = true,
                '+' => flag.is_sign = true,
                '\'' => flag.is_group = true,
                'I' => flag.is_locale = true, // 【修复】：不再 panic，正确识别 I 标志
                _ => break,
            }
            *i += 1;
        }

        let mut width = 0usize;
        let mut field_too_large = false;
        let mut precision = None;
        let mut j = *i;

        while j < bound && chars[j].is_ascii_digit() {
            let digit = chars[j].to_digit(10).unwrap() as usize;
            width = width
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .unwrap_or_else(|| {
                    field_too_large = true;
                    usize::MAX
                });
            j += 1;
        }
        field_too_large |= width > i32::MAX as usize;

        if j < bound && chars[j] == '.' {
            j += 1;
            let mut prec = 0u64;
            let mut has_precision = false;
            while j < bound && chars[j].is_ascii_digit() {
                let digit = u64::from(chars[j].to_digit(10).unwrap());
                prec = prec
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(digit))
                    .unwrap_or_else(|| {
                        field_too_large = true;
                        u64::MAX
                    });
                has_precision = true;
                j += 1;
            }
            if has_precision {
                field_too_large |= prec > i32::MAX as u64;
                precision = Some(prec.min(i32::MAX as u64) as i32);
            } else {
                precision = Some(-1);
            }
        }

        *i = j;
        // 使用单引号包裹错误信息
        if *i >= bound {
            let directive = chars[old..].iter().collect::<String>();
            return StatToken::InvalidDirective(directive);
        }

        let mut modifier = None;
        if (chars[*i] == 'H' || chars[*i] == 'L')
            && *i + 1 < bound
            && (chars[*i + 1] == 'd' || chars[*i + 1] == 'r')
        {
            modifier = Some(chars[*i]);
            *i += 1;
        }

        // 如果跟在修饰符后的是 '%'，直接拦截报错，而不是把它当成合法的 format 指令
        if chars[*i] == '%' {
            let directive = chars[old..=*i].iter().collect::<String>();
            return StatToken::InvalidDirective(directive);
        }

        if field_too_large {
            return StatToken::IgnoredDirective;
        }

        StatToken::Directive {
            width,
            flag,
            precision,
            modifier,
            format: chars[*i],
        }
    }

    fn handle_escape_sequences(chars: &[char], i: &mut usize, bound: usize) -> StatToken {
        *i += 1;
        if *i >= bound {
            ct_show_warning!("backslash at end of format");
            return StatToken::Char('\\');
        }
        match chars[*i] {
            // 精确区分 \x 是不完整还是无法识别，并使用裸字节
            'x' => {
                if *i + 1 < bound {
                    let digits = chars[*i + 1..].iter().take(2).collect::<String>();
                    if let Some((b, offset)) = digits.scan_char(16) {
                        *i += offset;
                        StatToken::Byte(b)
                    } else {
                        ct_show_warning!("unrecognized escape '\\x'");
                        StatToken::Char('x')
                    }
                } else {
                    ct_show_warning!("unrecognized escape '\\x'");
                    StatToken::Char('x')
                }
            }
            // 八进制直接生成裸字节
            '0'..='7' => {
                let digits = chars[*i..].iter().take(3).collect::<String>();
                let (b, offset) = digits.scan_char(8).unwrap();
                *i += offset - 1;
                StatToken::Byte(b)
            }
            '"' => StatToken::Char('"'),
            '\\' => StatToken::Char('\\'),
            'a' => StatToken::Byte(b'\x07'),
            'b' => StatToken::Byte(b'\x08'),
            'e' => StatToken::Byte(b'\x1B'),
            'f' => StatToken::Byte(b'\x0C'),
            'n' => StatToken::Byte(b'\n'),
            'r' => StatToken::Byte(b'\r'),
            't' => StatToken::Byte(b'\t'),
            'v' => StatToken::Byte(b'\x0B'),
            c => {
                ct_show_warning!("unrecognized escape '\\{}'", c);
                StatToken::Char(c)
            }
        }
    }

    fn generate_tokens(format_str: &str, use_printf: bool) -> CTResult<Vec<StatToken>> {
        let mut tokens = Vec::new();
        let chars = format_str.chars().collect::<Vec<char>>();
        let bound = chars.len();
        let mut i = 0;
        while i < bound {
            let token = match chars[i] {
                '%' => Self::handle_percent_case(&chars, &mut i, bound),
                '\\' => {
                    if use_printf {
                        Self::handle_escape_sequences(&chars, &mut i, bound)
                    } else {
                        StatToken::Char('\\')
                    }
                }
                c => StatToken::Char(c),
            };
            let invalid = matches!(token, StatToken::InvalidDirective(_));
            tokens.push(token);
            if invalid {
                break;
            }
            i += 1;
        }
        if !use_printf
            && !format_str.ends_with('\n')
            && !matches!(tokens.last(), Some(StatToken::InvalidDirective(_)))
        {
            tokens.push(StatToken::Char('\n'));
        }
        Ok(tokens)
    }

    fn new(matches: &ArgMatches) -> CTResult<Self> {
        // Get files
        let files = Self::get_files(matches)?;

        // Get format configuration
        let (default_tokens, default_dev_tokens) = Self::configure_format(matches)?;

        let is_show_fs = matches.get_flag(stat_options::STAT_FILE_SYSTEM);
        let is_from_user = matches.contains_id(stat_options::STAT_FORMAT)
            || matches.contains_id(stat_options::STAT_PRINTF);
        let requests_mount_point = default_tokens
            .iter()
            .chain(
                (!is_from_user)
                    .then_some(default_dev_tokens.iter())
                    .into_iter()
                    .flatten(),
            )
            .any(|token| matches!(token, StatToken::Directive { format: 'm', .. }));
        let uses_quoting_environment = is_from_user
            && default_tokens.iter().any(|token| {
                matches!(
                    token,
                    StatToken::Directive {
                        flag,
                        width: 0,
                        precision: None,
                        modifier: None,
                        format: 'N',
                    } if *flag == StatFlags::default()
                )
            });
        let quoting_style = if uses_quoting_environment {
            match std::env::var_os("QUOTING_STYLE") {
                Some(value) => {
                    let value = value.to_string_lossy();
                    StatQuotingStyle::parse(&value).unwrap_or_else(|| {
                        ct_show_error!(
                            "ignoring invalid value of environment variable QUOTING_STYLE: {}",
                            value.as_ref().quote()
                        );
                        StatQuotingStyle::shell_escape_always()
                    })
                }
                None => StatQuotingStyle::shell_escape_always(),
            }
        } else {
            StatQuotingStyle::literal()
        };

        let mount_list = if is_show_fs || !requests_mount_point {
            None
        } else {
            Self::get_mount_list()?
        };

        // 处理 --cached 选项
        let cached_mode =
            if let Some(mode_str) = matches.get_one::<String>(stat_options::STAT_CACHED) {
                match CachedMode::from_str(mode_str) {
                    Ok(mode) => mode,
                    Err(err) => return Err(CtSimpleError::new(1, err)),
                }
            } else {
                CachedMode::Default
            };

        Ok(Self {
            is_follow: matches.get_flag(stat_options::STAT_DEREFERENCE),
            is_show_fs,
            is_from_user,
            cached_mode,
            files,
            mount_list,
            default_tokens,
            default_dev_tokens,
            quoting_style,
        })
    }

    fn get_files(matches: &ArgMatches) -> CTResult<Vec<OsString>> {
        matches
            .get_many::<OsString>(stat_options::STAT_FILES)
            .map(|v| v.map(OsString::from).collect())
            .filter(|files: &Vec<OsString>| !files.is_empty())
            .ok_or_else(|| {
                CtSimpleError::new(
                    1,
                    "missing operand\nTry 'stat --help' for more information.".to_string(),
                )
            })
    }

    fn configure_format(matches: &ArgMatches) -> CTResult<(Vec<StatToken>, Vec<StatToken>)> {
        let format_str = if matches.contains_id(stat_options::STAT_PRINTF) {
            Some(
                matches
                    .get_one::<String>(stat_options::STAT_PRINTF)
                    .expect("Invalid format string")
                    .as_str(),
            )
        } else {
            matches
                .get_one::<String>(stat_options::STAT_FORMAT)
                .map(|s| s.as_str())
        };

        let use_printf = matches.contains_id(stat_options::STAT_PRINTF);
        let terse = matches.get_flag(stat_options::STAT_TERSE);
        let show_fs = matches.get_flag(stat_options::STAT_FILE_SYSTEM);

        let default_tokens = if let Some(format_str) = format_str {
            Self::generate_tokens(format_str, use_printf)?
        } else {
            Self::generate_tokens(&Self::default_format(show_fs, terse, false), false)?
        };

        let default_dev_tokens =
            Self::generate_tokens(&Self::default_format(show_fs, terse, true), use_printf)?;

        Ok((default_tokens, default_dev_tokens))
    }

    fn get_mount_list() -> CTResult<Option<Vec<String>>> {
        let mut mount_list = read_fs_list()
            .map_err_context(|| "cannot read table of mounted file systems".into())?
            .iter()
            .map(|mi| mi.mount_dir.clone())
            .collect::<Vec<String>>();

        // Reverse sort. The longer comes first.
        mount_list.sort();
        mount_list.reverse();

        Ok(Some(mount_list))
    }

    fn find_mount_point<P: AsRef<Path>>(&self, p: P) -> Option<String> {
        let input = p.as_ref();
        let path = if !self.is_follow
            && fs::symlink_metadata(input).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            let parent = input
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            parent.canonicalize().ok()?.join(input.file_name()?)
        } else {
            input.canonicalize().ok()?
        };

        for root in self.mount_list.as_ref()? {
            if path.starts_with(root) {
                return Some(root.clone());
            }
        }
        None
    }

    fn exec<W: Write>(&self, writer: &mut W) -> Result<i32, StatExecutionError> {
        let mut stdin_is_fifo = false;
        if cfg!(unix) {
            if let Ok(md) = fs::metadata("/dev/stdin") {
                stdin_is_fifo = md.file_type().is_fifo();
            }
        }

        let mut ret = 0;
        for f in &self.files {
            ret |= self.do_stat(f, stdin_is_fifo, writer)?;
        }
        Ok(ret)
    }

    fn do_stat<W: Write>(
        &self,
        file: &OsStr,
        stdin_is_fifo: bool,
        writer: &mut W,
    ) -> Result<i32, StatExecutionError> {
        let display_name = file.to_string_lossy();

        // Handle file path resolution
        let file = match self.resolve_file_path(display_name.as_ref(), stdin_is_fifo) {
            Ok(path) => path,
            Err(status) => return Ok(status),
        };

        // Process based on mode (filesystem or file)
        if self.is_show_fs {
            self.handle_filesystem_stat(&file, display_name.as_ref(), writer)
        } else {
            self.handle_file_stat(&file, display_name.as_ref(), stdin_is_fifo, writer)
        }
    }

    fn resolve_file_path(&self, display_name: &str, _stdin_is_fifo: bool) -> Result<OsString, i32> {
        if !cfg!(unix) || display_name != "-" {
            return Ok(OsString::from(display_name));
        }

        if self.is_show_fs {
            ct_show_error!("using '-' to denote standard input does not work in file system mode");
            return Err(1);
        }

        Ok(OsString::from("-"))
    }

    fn handle_filesystem_stat<W: Write>(
        &self,
        file: &OsStr,
        display_name: &str,
        writer: &mut W,
    ) -> Result<i32, StatExecutionError> {
        #[cfg(unix)]
        let path = file.as_bytes();
        #[cfg(not(unix))]
        let path = file.to_string_lossy();

        match statfs(path) {
            Ok(meta) => {
                self.print_filesystem_info(&meta, &self.default_tokens, display_name, writer)?;
                Ok(0)
            }
            Err(e) => {
                // statfs 返回的是 String 类型的错误，直接使用
                // 尝试提取错误消息，去除可能的错误代码
                let error_description = if e.contains("(os error ") {
                    if let Some(pos) = e.find("(os error ") {
                        e[..pos].trim().to_string()
                    } else {
                        e
                    }
                } else {
                    e
                };

                ct_show_error!(
                    "cannot read file system information for {}: {}",
                    display_name.quote(),
                    error_description
                );
                Ok(1)
            }
        }
    }

    fn handle_file_stat<W: Write>(
        &self,
        file: &OsStr,
        display_name: &str,
        _stdin_is_fifo: bool,
        writer: &mut W,
    ) -> Result<i32, StatExecutionError> {
        let result = if display_name == "-" {
            metadata_for_stdin()
        } else {
            get_metadata(file, self.is_follow, self.cached_mode)
        };

        match result {
            Ok(meta) => {
                let tokens = self.select_tokens(&meta);
                self.print_file_info(&meta, tokens, file, display_name, writer)
            }
            Err(e) => {
                // 提取错误描述，不包含错误代码
                let error_description = match e.kind() {
                    std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
                    std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
                    std::io::ErrorKind::ConnectionRefused => "Connection refused".to_string(),
                    std::io::ErrorKind::ConnectionReset => "Connection reset".to_string(),
                    std::io::ErrorKind::ConnectionAborted => "Connection aborted".to_string(),
                    std::io::ErrorKind::NotConnected => "Not connected".to_string(),
                    std::io::ErrorKind::AddrInUse => "Address in use".to_string(),
                    std::io::ErrorKind::AddrNotAvailable => "Address not available".to_string(),
                    std::io::ErrorKind::BrokenPipe => "Broken pipe".to_string(),
                    std::io::ErrorKind::AlreadyExists => "File exists".to_string(),
                    std::io::ErrorKind::WouldBlock => {
                        "Resource temporarily unavailable".to_string()
                    }
                    std::io::ErrorKind::InvalidInput => "Invalid argument".to_string(),
                    std::io::ErrorKind::InvalidData => "Invalid data".to_string(),
                    std::io::ErrorKind::TimedOut => "Timed out".to_string(),
                    std::io::ErrorKind::WriteZero => "Write zero".to_string(),
                    std::io::ErrorKind::Interrupted => "Interrupted system call".to_string(),
                    std::io::ErrorKind::Unsupported => "Operation not supported".to_string(),
                    std::io::ErrorKind::UnexpectedEof => "Unexpected end of file".to_string(),
                    std::io::ErrorKind::OutOfMemory => "Out of memory".to_string(),
                    _ => {
                        // 尝试提取错误消息，去除可能的错误代码
                        let err_string = e.to_string();
                        if let Some(pos) = err_string.find("(os error ") {
                            err_string[..pos].trim().to_string()
                        } else {
                            err_string
                        }
                    }
                };
                if display_name == "-" {
                    ct_show_error!("cannot stat standard input: {}", error_description);
                } else {
                    ct_show_error!(
                        "cannot statx {}: {}",
                        display_name.quote(),
                        error_description
                    );
                }
                Ok(1)
            }
        }
    }

    fn select_tokens(&self, meta: &fs::Metadata) -> &[StatToken] {
        if self.is_from_user
            || !(meta.file_type().is_char_device() || meta.file_type().is_block_device())
        {
            &self.default_tokens
        } else {
            &self.default_dev_tokens
        }
    }

    fn print_filesystem_info<W: Write>(
        &self,
        meta: &impl FsMeta,
        tokens: &[StatToken],
        display_name: &str,
        writer: &mut W,
    ) -> Result<(), StatExecutionError> {
        for token in tokens {
            match token {
                StatToken::Char(c) => write!(writer, "{c}")?,
                StatToken::Byte(b) => writer.write_all(&[*b])?,
                StatToken::IgnoredDirective => {}
                StatToken::InvalidDirective(directive) => {
                    return Err(StatExecutionError::InvalidDirective(format!(
                        "'{directive}': invalid directive"
                    )));
                }
                StatToken::Directive {
                    flag,
                    width,
                    precision,
                    modifier: _,
                    format,
                } => {
                    let output = self.get_filesystem_output(meta, *format, display_name);
                    print_it(writer, &output, *flag, *width, *precision)?;
                }
            }
        }
        Ok(())
    }

    fn print_file_info(
        &self,
        meta: &fs::Metadata,
        tokens: &[StatToken],
        file: &OsStr,
        display_name: &str,
        writer: &mut impl Write,
    ) -> Result<i32, StatExecutionError> {
        let mut status = 0;

        for token in tokens {
            match token {
                StatToken::Char(c) => write!(writer, "{c}")?,
                StatToken::Byte(b) => writer.write_all(&[*b])?,
                StatToken::IgnoredDirective => {}
                StatToken::InvalidDirective(directive) => {
                    return Err(StatExecutionError::InvalidDirective(format!(
                        "'{directive}': invalid directive"
                    )));
                }
                StatToken::Directive {
                    flag,
                    width,
                    precision,
                    modifier,
                    format,
                } => {
                    let (output, output_status) = self.get_file_output_with_status(
                        meta,
                        *format,
                        file,
                        display_name,
                        *modifier,
                    );
                    status |= output_status;
                    print_it(writer, &output, *flag, *width, *precision)?;
                }
            }
        }

        Ok(status)
    }

    fn get_filesystem_output(
        &self,
        meta: &impl FsMeta,
        format: char,
        display_name: &str,
    ) -> StatOutputType {
        match format {
            // free blocks available to non-superuser
            'a' => StatOutputType::Integer(meta.avail_blocks() as i64),
            // total data blocks in file system
            'b' => StatOutputType::Integer(meta.total_blocks() as i64),
            // total file nodes in file system
            'c' => StatOutputType::Unsigned(meta.total_file_nodes()),
            // free file nodes in file system
            'd' => StatOutputType::Integer(meta.free_file_nodes() as i64),
            // free blocks in file system
            'f' => StatOutputType::Integer(meta.free_blocks() as i64),
            // file system ID in hex
            'i' => StatOutputType::UnsignedHex(meta.fsid()),
            // maximum length of filenames
            'l' => StatOutputType::Unsigned(meta.namelen()),
            // file name
            'n' => StatOutputType::Str(display_name.to_string()),
            // block size (for faster transfers)
            's' => StatOutputType::Unsigned(meta.io_size()),
            // fundamental block size (for block counts)
            'S' => StatOutputType::Unsigned(meta.block_size() as u64),
            // file system type in hex
            't' => StatOutputType::UnsignedHex(meta.fs_type() as u64),
            // file system type in human readable form
            'T' => StatOutputType::Str(pretty_fstype(meta.fs_type()).into()),
            _ => StatOutputType::Unknown,
        }
    }

    fn get_file_output(
        &self,
        meta: &fs::Metadata,
        format: char,
        file: &OsStr,
        display_name: &str,
        modifier: Option<char>,
    ) -> StatOutputType {
        self.get_file_output_without_diagnostics(meta, format, file, display_name, modifier)
    }

    fn get_file_output_with_status(
        &self,
        meta: &fs::Metadata,
        format: char,
        file: &OsStr,
        display_name: &str,
        modifier: Option<char>,
    ) -> (StatOutputType, i32) {
        if format == 'C' {
            return self.get_file_context_output(file, true);
        }

        (
            self.get_file_output_without_diagnostics(meta, format, file, display_name, modifier),
            0,
        )
    }

    fn get_file_output_without_diagnostics(
        &self,
        meta: &fs::Metadata,
        format: char,
        file: &OsStr,
        display_name: &str,
        modifier: Option<char>,
    ) -> StatOutputType {
        let file_type = meta.file_type();

        match format {
            // access rights in octal
            'a' => StatOutputType::UnsignedOct(0o7777 & meta.mode()),
            // access rights in human readable form
            'A' => StatOutputType::Str(display_permissions(meta, true)),
            // number of blocks allocated (see %B)
            'b' => StatOutputType::Unsigned(meta.blocks()),
            // The size in bytes of each block reported by %b
            // FIXME: blocksize differs on various platform
            // See coreutils/gnulib/lib/stat-size.h ST_NBLOCKSIZE // spell-checker:disable-line
            'B' => StatOutputType::Unsigned(512),

            //SELinux security context string
            'C' => self.get_file_context_output(file, false).0,
            // device number - handle modifier for major/minor separation
            'd' => match modifier {
                Some('H') => StatOutputType::Unsigned(device_major(meta.dev())),
                Some('L') => StatOutputType::Unsigned(device_minor(meta.dev())),
                _ => StatOutputType::Unsigned(meta.dev()),
            },
            'D' => match modifier {
                Some('H') => StatOutputType::UnsignedHex(device_major(meta.dev())),
                Some('L') => StatOutputType::UnsignedHex(device_minor(meta.dev())),
                _ => StatOutputType::UnsignedHex(meta.dev()),
            },
            // raw mode in hex
            'f' => StatOutputType::UnsignedHex(meta.mode() as u64),
            // file type (localized)
            'F' => StatOutputType::Str(localized_filetype(meta.mode() as mode_t, meta.len())),
            // group ID of owner
            'g' => StatOutputType::Unsigned(meta.gid() as u64),
            // group name of owner
            'G' => {
                let group_name =
                    ct_entries::gid2grp(meta.gid()).unwrap_or_else(|_| "UNKNOWN".to_owned());
                StatOutputType::Str(group_name)
            }
            // number of hard links
            'h' => StatOutputType::Unsigned(meta.nlink()),
            // inode number
            'i' => StatOutputType::Unsigned(meta.ino()),
            // mount point
            'm' => StatOutputType::Str(self.find_mount_point(file).unwrap_or_default()),
            // file name
            'n' => StatOutputType::Str(display_name.to_string()),
            // quoted file name with dereference if symbolic link
            'N' => {
                let format_quote = |s: &str| self.quoting_style.quote(s);

                let file_name = if file_type.is_symlink() {
                    // 读取符号链接应该用真实解析出的 file，而不是字面量 display_name
                    let dst = match fs::read_link(file) {
                        Ok(path) => path,
                        Err(e) => {
                            println!("{e}");
                            return StatOutputType::Unknown;
                        }
                    };
                    format!(
                        "{} -> {}",
                        format_quote(display_name), // 输出原始名称
                        format_quote(&dst.to_string_lossy())
                    )
                } else {
                    format_quote(display_name) // 输出原始名称
                };
                StatOutputType::Str(file_name)
            }
            // optimal I/O transfer size hint
            'o' => StatOutputType::Unsigned(meta.blksize()),
            // total size, in bytes
            's' => StatOutputType::Unsigned(meta.len()),
            't' => match modifier {
                Some('H') => StatOutputType::UnsignedHex(device_major(meta.rdev())),
                Some('L') => StatOutputType::UnsignedHex(device_minor(meta.rdev())),
                _ => StatOutputType::UnsignedHex(device_major(meta.rdev())),
            },
            'T' => match modifier {
                Some('H') => StatOutputType::UnsignedHex(device_major(meta.rdev())),
                Some('L') => StatOutputType::UnsignedHex(device_minor(meta.rdev())),
                _ => StatOutputType::UnsignedHex(device_minor(meta.rdev())),
            },
            'r' => match modifier {
                Some('H') => StatOutputType::Unsigned(device_major(meta.rdev())),
                Some('L') => StatOutputType::Unsigned(device_minor(meta.rdev())),
                _ => StatOutputType::Unsigned(meta.rdev()),
            },
            'R' => match modifier {
                Some('H') => StatOutputType::UnsignedHex(device_major(meta.rdev())),
                Some('L') => StatOutputType::UnsignedHex(device_minor(meta.rdev())),
                _ => StatOutputType::UnsignedHex(meta.rdev()),
            },
            // user ID of owner
            'u' => StatOutputType::Unsigned(meta.uid() as u64),
            // user name of owner
            'U' => {
                let user_name =
                    ct_entries::uid2usr(meta.uid()).unwrap_or_else(|_| "UNKNOWN".to_owned());
                StatOutputType::Str(user_name)
            }

            // time of file birth, human-readable; - if unknown
            'w' => StatOutputType::Str(
                meta.birth()
                    .map(|(sec, nsec)| pretty_time(sec as i64, nsec as i64))
                    .unwrap_or_else(|| "-".to_string()),
            ),
            // time of file birth, seconds since Epoch; 0 if unknown
            'W' => {
                let (sec, nsec) = meta.birth().unwrap_or_default();
                StatOutputType::Timestamp(sec as i64, nsec as i64)
            }

            // time of last access, human-readable
            'x' => StatOutputType::Str(pretty_time(meta.atime(), meta.atime_nsec())),
            // time of last access, seconds since Epoch
            'X' => StatOutputType::Timestamp(meta.atime(), meta.atime_nsec()),
            // time of last data modification, human-readable
            'y' => StatOutputType::Str(pretty_time(meta.mtime(), meta.mtime_nsec())),
            // time of last data modification, seconds since Epoch
            'Y' => StatOutputType::Timestamp(meta.mtime(), meta.mtime_nsec()),
            // time of last status change, human-readable
            'z' => StatOutputType::Str(pretty_time(meta.ctime(), meta.ctime_nsec())),
            // time of last status change, seconds since Epoch
            'Z' => StatOutputType::Timestamp(meta.ctime(), meta.ctime_nsec()),

            _ => StatOutputType::Unknown,
        }
    }

    fn get_file_context_output(
        &self,
        file: &OsStr,
        emit_diagnostic: bool,
    ) -> (StatOutputType, i32) {
        if file.is_empty() {
            return (StatOutputType::Str(String::new()), 0);
        }

        match selinux::SecurityContext::of_path(file, false, false) {
            Err(err) => {
                if emit_diagnostic {
                    ct_show_error!(
                        "failed to get security context of {}: {}",
                        file.quote(),
                        security_context_error_description(&err)
                    );
                }
                (StatOutputType::Str("?".to_string()), 1)
            }
            Ok(None) => {
                if emit_diagnostic {
                    ct_show_error!(
                        "failed to get security context of {}: No data available",
                        file.quote()
                    );
                }
                (StatOutputType::Str("?".to_string()), 1)
            }
            Ok(Some(context)) => {
                let context = context.as_bytes();
                let context_strip_suffix = context.strip_suffix(&[0]).unwrap_or(context);
                let context_str =
                    String::from_utf8(context_strip_suffix.to_vec()).unwrap_or_else(|e| {
                        if emit_diagnostic {
                            ct_show_warning!(
                                "getting security context of {}: {}",
                                file.quote(),
                                e.to_string()
                            );
                        }
                        String::from_utf8_lossy(context_strip_suffix).into_owned()
                    });
                (StatOutputType::Str(context_str), 0)
            }
        }
    }

    fn default_format(show_fs: bool, terse: bool, show_dev_type: bool) -> String {
        if show_fs {
            if terse {
                t!("default_format.fs_terse")
            } else {
                t!("default_format.fs_normal")
            }
        } else if terse {
            default_file_terse_format(
                selinux::kernel_support() != selinux::KernelSupport::Unsupported,
            )
        } else {
            let part2 = if show_dev_type {
                t!("default_format.file_part2_dev")
            } else {
                t!("default_format.file_part2_no_dev")
            };
            format!(
                "{}{}{}{}",
                t!("default_format.file_part1"),
                part2,
                t!("default_format.file_part3"),
                t!("default_format.file_part4"),
            )
        }
    }
}

fn default_file_terse_format(include_context: bool) -> String {
    let mut format = t!("default_format.file_terse");
    if include_context {
        if format.ends_with('\n') {
            format.pop();
            format.push_str(" %C\n");
        } else {
            format.push_str(" %C");
        }
    }
    format
}

fn security_context_error_description(err: &impl std::fmt::Display) -> String {
    let text = err.to_string();
    if let Some(pos) = text.find("(os error ") {
        text[..pos].trim().trim_end_matches('.').to_string()
    } else {
        text.trim_end_matches('.').to_string()
    }
}

/// \u8fd4\u56de\u672c\u5730\u5316\u7684\u6587\u4ef6\u7c7b\u578b\u5b57\u7b26\u4e32\uff08\u901a\u8fc7 t!() \u5b8f\u8fdb\u884c\u7ffb\u8bd1\uff09
fn localized_filetype(mode: mode_t, size: u64) -> String {
    match pretty_filetype(mode, size) {
        "regular empty file" => t!("file_type.regular_empty_file"),
        "regular file" => t!("file_type.regular_file"),
        "directory" => t!("file_type.directory"),
        "symbolic link" => t!("file_type.symbolic_link"),
        "character special file" => t!("file_type.character_special_file"),
        "block special file" => t!("file_type.block_special_file"),
        "fifo" => t!("file_type.fifo"),
        "socket" => t!("file_type.socket"),
        _ => t!("file_type.weird_file"),
    }
}

// \u6dfb\u52a0\u4e00\u4e2a\u51fd\u6570\uff0c\u76f4\u63a5\u4f7f\u7528 rustix \u7684 statx \u83b7\u53d6\u6587\u4ef6\u4fe1\u606f\u5e76\u8f6c\u6362\u4e3a fs::Metadata
fn get_metadata(
    file: &OsStr,
    follow_links: bool,
    cached_mode: CachedMode,
) -> std::io::Result<fs::Metadata> {
    // 设置基本标志
    let mut flags = AtFlags::empty();

    // 如果不跟随符号链接
    if !follow_links {
        flags |= AtFlags::SYMLINK_NOFOLLOW;
    }

    // 根据缓存模式设置标志
    match cached_mode {
        CachedMode::Always => flags |= AtFlags::STATX_DONT_SYNC,
        CachedMode::Never => flags |= AtFlags::STATX_FORCE_SYNC,
        CachedMode::Default => {} // 不设置特殊标志
    }

    // 如果不是强制同步，添加 AT_NO_AUTOMOUNT 标志
    if cached_mode != CachedMode::Never {
        flags |= AtFlags::NO_AUTOMOUNT;
    }

    // 尝试使用 rustix 的 statx 获取文件信息
    // 请求所有可能需要的字段
    let statx_result = statx(rustix::fs::CWD, file.as_bytes(), flags, StatxFlags::ALL);

    match statx_result {
        Ok(_statx_data) => {
            // 理想情况下，我们应该直接将 statx 数据转换为 fs::Metadata
            // 但由于 rustix 和标准库之间可能没有直接的转换接口，
            // 我们可能需要自己实现这个转换
            //
            // 这里我们仍然使用标准库的方法，但在实际项目中，
            // 可以考虑实现一个 statx_to_metadata 函数，类似于 statx.h 中的 statx_to_stat
            if follow_links {
                fs::metadata(file)
            } else {
                fs::symlink_metadata(file)
            }
        }
        Err(_e) => {
            // 如果 statx 失败，回退到标准库的方法
            if follow_links {
                fs::metadata(file)
            } else {
                fs::symlink_metadata(file)
            }
        }
    }

    // 注意：要完全模拟 stat.c 的行为，我们需要实现一个 statx_to_metadata 转换函数
    // 类似于 statx.h 中的 statx_to_stat 函数，将 rustix::fs::Statx 转换为 std::fs::Metadata
    //
    // 这需要更深入地了解 rustix 和 std::fs::Metadata 的内部实现
    // 以及可能需要使用 unsafe 代码来构造 Metadata
    //
    // 一个完整的实现可能类似于：
    //
    // fn statx_to_metadata(statx_data: rustix::fs::Statx) -> std::fs::Metadata {
    //     // 构造 Metadata 对象
    //     // 这可能需要使用 std::fs::Metadata 的内部构造函数或其他方法
    // }
}

#[derive(Default)]
pub struct Stat;
impl Tool for Stat {
    fn name(&self) -> &'static str {
        "stat"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        stat_main(args.iter().cloned())
    }
}

pub fn stat_main(args: impl ctcore::Args) -> CTResult<()> {
    init_stat_locale();

    let matches = ct_app()
        .after_help(rust_i18n::t!(stat_options::STAT_LONG_USAGE))
        .try_get_matches_from(args)?;

    let stater = Stater::new(&matches)?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let status = match stater.exec(&mut output) {
        Ok(status) => status,
        Err(StatExecutionError::Write(error)) => {
            return Err(error).map_err_context(|| String::from("write error"));
        }
        Err(StatExecutionError::InvalidDirective(message)) => {
            output
                .flush()
                .map_err_context(|| String::from("write error"))?;
            return Err(CtSimpleError::new(1, message));
        }
    };
    output
        .flush()
        .map_err_context(|| String::from("write error"))?;

    // Convert non-zero exit status to error
    match status {
        0 => Ok(()),
        status => Err(status.into()),
    }
}

pub fn stat_native_semantic(args: impl ctcore::Args) -> CTResult<StatSemantic> {
    init_stat_locale();

    let matches = ct_app()
        .after_help(rust_i18n::t!(stat_options::STAT_LONG_USAGE))
        .try_get_matches_from(args)?;
    let stater = Stater::new(&matches)?;
    if let Some(directive) = stater.default_tokens.iter().find_map(|token| match token {
        StatToken::InvalidDirective(directive) => Some(directive),
        _ => None,
    }) {
        return Err(CtSimpleError::new(
            1,
            format!("'{directive}': invalid directive"),
        ));
    }
    let selected_fields = semantic_selected_fields(&stater, &matches);

    let mut stdin_is_fifo = false;
    if cfg!(unix)
        && let Ok(md) = fs::metadata("/dev/stdin")
    {
        stdin_is_fifo = md.file_type().is_fifo();
    }

    let mut rows = Vec::with_capacity(stater.files.len());
    let mut classic_text = String::new();

    for file in &stater.files {
        let display_name = file.to_string_lossy().to_string();
        let resolved = match stater.resolve_file_path(display_name.as_ref(), stdin_is_fifo) {
            Ok(path) => path,
            Err(status) => {
                return Err(CtSimpleError::new(
                    status,
                    "using '-' to denote standard input does not work in file system mode",
                ));
            }
        };

        if stater.is_show_fs {
            #[cfg(unix)]
            let path = resolved.as_bytes();
            #[cfg(not(unix))]
            let path = resolved.to_string_lossy();

            let meta = statfs(path).map_err(|e| {
                CtSimpleError::new(
                    1,
                    format!(
                        "cannot read file system information for {}: {}",
                        display_name.quote(),
                        filesystem_error_description(&e)
                    ),
                )
            })?;

            let rendered =
                render_filesystem_tokens(&stater, &meta, &stater.default_tokens, &display_name);
            classic_text.push_str(&rendered);
            rows.push(build_filesystem_semantic_row(
                &stater,
                &meta,
                &display_name,
                &selected_fields,
                &rendered,
            ));
        } else {
            let meta = (if display_name == "-" {
                metadata_for_stdin()
            } else {
                get_metadata(&resolved, stater.is_follow, stater.cached_mode)
            })
            .map_err(|e| {
                CtSimpleError::new(
                    1,
                    format!(
                        "cannot statx {}: {}",
                        display_name.quote(),
                        file_error_description(&e)
                    ),
                )
            })?;

            let tokens = stater.select_tokens(&meta);
            let rendered = normalize_default_file_classic_text(
                render_file_tokens(&stater, &meta, tokens, &resolved, &display_name),
                &stater,
                &matches,
            );
            classic_text.push_str(&rendered);
            rows.push(build_file_semantic_row(
                &stater,
                &meta,
                &resolved,
                &display_name,
                &selected_fields,
                &rendered,
            ));
        }
    }

    if classic_text.ends_with('\n') {
        classic_text.pop();
    }

    Ok(StatSemantic {
        rows,
        selected_fields,
        classic_text,
    })
}

fn init_stat_locale() {
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
}

fn normalize_default_file_classic_text(
    rendered: String,
    stater: &Stater,
    matches: &ArgMatches,
) -> String {
    if stater.is_show_fs || stater.is_from_user || matches.get_flag(stat_options::STAT_TERSE) {
        return rendered;
    }

    let unknown_context_line = rust_i18n::t!("default_format.file_part_context").replace("%C", "?");
    rendered.replacen(&unknown_context_line, "", 1)
}

fn semantic_selected_fields(stater: &Stater, matches: &ArgMatches) -> Vec<StatSemanticField> {
    if stater.is_from_user {
        let mut fields = token_semantic_fields(stater.is_show_fs, &stater.default_tokens);
        push_unique_field(&mut fields, StatSemanticField::Formatted);
        return fields;
    }

    if stater.is_show_fs {
        if matches.get_flag(stat_options::STAT_TERSE) {
            terse_filesystem_fields()
        } else {
            rich_filesystem_fields()
        }
    } else if matches.get_flag(stat_options::STAT_TERSE) {
        terse_file_fields()
    } else {
        rich_file_fields()
    }
}

fn rich_file_fields() -> Vec<StatSemanticField> {
    vec![
        StatSemanticField::Name,
        StatSemanticField::QuotedName,
        StatSemanticField::Size,
        StatSemanticField::Blocks,
        StatSemanticField::IoBlock,
        StatSemanticField::FileType,
        StatSemanticField::DeviceMajor,
        StatSemanticField::DeviceMinor,
        StatSemanticField::Inode,
        StatSemanticField::Links,
        StatSemanticField::AccessRightsOctal,
        StatSemanticField::AccessRightsHuman,
        StatSemanticField::Uid,
        StatSemanticField::User,
        StatSemanticField::Gid,
        StatSemanticField::Group,
        StatSemanticField::MountPoint,
        StatSemanticField::Context,
        StatSemanticField::AccessTime,
        StatSemanticField::ModifyTime,
        StatSemanticField::ChangeTime,
        StatSemanticField::BirthTime,
        StatSemanticField::DeviceTypeMajor,
        StatSemanticField::DeviceTypeMinor,
    ]
}

fn terse_file_fields() -> Vec<StatSemanticField> {
    let mut fields = vec![
        StatSemanticField::Name,
        StatSemanticField::Size,
        StatSemanticField::Blocks,
        StatSemanticField::RawModeHex,
        StatSemanticField::Uid,
        StatSemanticField::Gid,
        StatSemanticField::DeviceHex,
        StatSemanticField::Inode,
        StatSemanticField::Links,
        StatSemanticField::DeviceTypeMajorHex,
        StatSemanticField::DeviceTypeMinorHex,
        StatSemanticField::AccessEpoch,
        StatSemanticField::ModifyEpoch,
        StatSemanticField::ChangeEpoch,
        StatSemanticField::BirthEpoch,
        StatSemanticField::IoBlock,
    ];
    if selinux::kernel_support() != selinux::KernelSupport::Unsupported {
        fields.push(StatSemanticField::Context);
    }
    fields
}

fn rich_filesystem_fields() -> Vec<StatSemanticField> {
    vec![
        StatSemanticField::Name,
        StatSemanticField::FilesystemIdHex,
        StatSemanticField::NameMax,
        StatSemanticField::FilesystemType,
        StatSemanticField::FilesystemTypeHex,
        StatSemanticField::BlockSize,
        StatSemanticField::FundamentalBlockSize,
        StatSemanticField::TotalBlocks,
        StatSemanticField::FreeBlocks,
        StatSemanticField::AvailableBlocks,
        StatSemanticField::TotalFileNodes,
        StatSemanticField::FreeFileNodes,
    ]
}

fn terse_filesystem_fields() -> Vec<StatSemanticField> {
    vec![
        StatSemanticField::Name,
        StatSemanticField::FilesystemIdHex,
        StatSemanticField::NameMax,
        StatSemanticField::FilesystemTypeHex,
        StatSemanticField::BlockSize,
        StatSemanticField::FundamentalBlockSize,
        StatSemanticField::TotalBlocks,
        StatSemanticField::FreeBlocks,
        StatSemanticField::AvailableBlocks,
        StatSemanticField::TotalFileNodes,
        StatSemanticField::FreeFileNodes,
    ]
}

fn token_semantic_fields(is_show_fs: bool, tokens: &[StatToken]) -> Vec<StatSemanticField> {
    let mut fields = Vec::new();

    for token in tokens {
        let StatToken::Directive {
            modifier, format, ..
        } = token
        else {
            continue;
        };

        let mapped = if is_show_fs {
            filesystem_format_field(*format)
        } else {
            file_format_field(*format, *modifier)
        };

        if let Some(field) = mapped {
            push_unique_field(&mut fields, field);
        }
    }

    fields
}

fn filesystem_format_field(format: char) -> Option<StatSemanticField> {
    match format {
        'a' => Some(StatSemanticField::AvailableBlocks),
        'b' => Some(StatSemanticField::TotalBlocks),
        'c' => Some(StatSemanticField::TotalFileNodes),
        'd' => Some(StatSemanticField::FreeFileNodes),
        'f' => Some(StatSemanticField::FreeBlocks),
        'i' => Some(StatSemanticField::FilesystemIdHex),
        'l' => Some(StatSemanticField::NameMax),
        'n' => Some(StatSemanticField::Name),
        's' => Some(StatSemanticField::BlockSize),
        'S' => Some(StatSemanticField::FundamentalBlockSize),
        't' => Some(StatSemanticField::FilesystemTypeHex),
        'T' => Some(StatSemanticField::FilesystemType),
        _ => None,
    }
}

fn file_format_field(format: char, modifier: Option<char>) -> Option<StatSemanticField> {
    match (format, modifier) {
        ('a', _) => Some(StatSemanticField::AccessRightsOctal),
        ('A', _) => Some(StatSemanticField::AccessRightsHuman),
        ('b', _) => Some(StatSemanticField::Blocks),
        ('B', _) => Some(StatSemanticField::BlockSizeReported),
        ('C', _) => Some(StatSemanticField::Context),
        ('d', Some('H')) => Some(StatSemanticField::DeviceMajor),
        ('d', Some('L')) => Some(StatSemanticField::DeviceMinor),
        ('d', _) => Some(StatSemanticField::Device),
        ('D', Some('H')) => Some(StatSemanticField::DeviceMajorHex),
        ('D', Some('L')) => Some(StatSemanticField::DeviceMinorHex),
        ('D', _) => Some(StatSemanticField::DeviceHex),
        ('f', _) => Some(StatSemanticField::RawModeHex),
        ('F', _) => Some(StatSemanticField::FileType),
        ('g', _) => Some(StatSemanticField::Gid),
        ('G', _) => Some(StatSemanticField::Group),
        ('h', _) => Some(StatSemanticField::Links),
        ('i', _) => Some(StatSemanticField::Inode),
        ('m', _) => Some(StatSemanticField::MountPoint),
        ('n', _) => Some(StatSemanticField::Name),
        ('N', _) => Some(StatSemanticField::QuotedName),
        ('o', _) => Some(StatSemanticField::IoBlock),
        ('r', Some('H')) => Some(StatSemanticField::DeviceTypeMajor),
        ('r', Some('L')) => Some(StatSemanticField::DeviceTypeMinor),
        ('r', _) => Some(StatSemanticField::DeviceType),
        ('R', Some('H')) => Some(StatSemanticField::DeviceTypeMajorHex),
        ('R', Some('L')) => Some(StatSemanticField::DeviceTypeMinorHex),
        ('R', _) => Some(StatSemanticField::DeviceTypeHex),
        ('s', _) => Some(StatSemanticField::Size),
        ('t', _) => Some(StatSemanticField::DeviceTypeMajorHex),
        ('T', _) => Some(StatSemanticField::DeviceTypeMinorHex),
        ('u', _) => Some(StatSemanticField::Uid),
        ('U', _) => Some(StatSemanticField::User),
        ('w', _) => Some(StatSemanticField::BirthTime),
        ('W', _) => Some(StatSemanticField::BirthEpoch),
        ('x', _) => Some(StatSemanticField::AccessTime),
        ('X', _) => Some(StatSemanticField::AccessEpoch),
        ('y', _) => Some(StatSemanticField::ModifyTime),
        ('Y', _) => Some(StatSemanticField::ModifyEpoch),
        ('z', _) => Some(StatSemanticField::ChangeTime),
        ('Z', _) => Some(StatSemanticField::ChangeEpoch),
        _ => None,
    }
}

fn push_unique_field(fields: &mut Vec<StatSemanticField>, field: StatSemanticField) {
    if !fields.contains(&field) {
        fields.push(field);
    }
}

fn push_unique_value(
    fields: &mut Vec<(StatSemanticField, StatSemanticValue)>,
    field: StatSemanticField,
    value: StatSemanticValue,
) {
    if !fields.iter().any(|(existing, _)| *existing == field) {
        fields.push((field, value));
    }
}

fn build_file_semantic_row(
    stater: &Stater,
    meta: &fs::Metadata,
    file: &OsStr,
    display_name: &str,
    selected_fields: &[StatSemanticField],
    rendered: &str,
) -> StatSemanticRow {
    let mut fields = Vec::new();

    for field in selected_fields {
        let value = match field {
            StatSemanticField::Formatted => Some(StatSemanticValue::String(rendered.to_string())),
            _ => file_field_value(stater, meta, file, display_name, *field),
        };

        if let Some(value) = value {
            push_unique_value(&mut fields, *field, value);
        }
    }

    StatSemanticRow {
        row_kind: StatRowKind::File,
        fields,
    }
}

fn build_filesystem_semantic_row(
    stater: &Stater,
    meta: &impl FsMeta,
    display_name: &str,
    selected_fields: &[StatSemanticField],
    rendered: &str,
) -> StatSemanticRow {
    let mut fields = Vec::new();

    for field in selected_fields {
        let value = match field {
            StatSemanticField::Formatted => Some(StatSemanticValue::String(rendered.to_string())),
            _ => filesystem_field_value(stater, meta, display_name, *field),
        };

        if let Some(value) = value {
            push_unique_value(&mut fields, *field, value);
        }
    }

    StatSemanticRow {
        row_kind: StatRowKind::Filesystem,
        fields,
    }
}

fn file_field_value(
    stater: &Stater,
    meta: &fs::Metadata,
    file: &OsStr,
    display_name: &str,
    field: StatSemanticField,
) -> Option<StatSemanticValue> {
    let is_device = meta.file_type().is_char_device() || meta.file_type().is_block_device();

    let output = match field {
        StatSemanticField::Name => stater.get_file_output(meta, 'n', file, display_name, None),
        StatSemanticField::QuotedName => {
            stater.get_file_output(meta, 'N', file, display_name, None)
        }
        StatSemanticField::Size => stater.get_file_output(meta, 's', file, display_name, None),
        StatSemanticField::Blocks => stater.get_file_output(meta, 'b', file, display_name, None),
        StatSemanticField::BlockSizeReported => {
            stater.get_file_output(meta, 'B', file, display_name, None)
        }
        StatSemanticField::IoBlock => stater.get_file_output(meta, 'o', file, display_name, None),
        StatSemanticField::RawModeHex => {
            stater.get_file_output(meta, 'f', file, display_name, None)
        }
        StatSemanticField::FileType => stater.get_file_output(meta, 'F', file, display_name, None),
        StatSemanticField::Uid => stater.get_file_output(meta, 'u', file, display_name, None),
        StatSemanticField::User => stater.get_file_output(meta, 'U', file, display_name, None),
        StatSemanticField::Gid => stater.get_file_output(meta, 'g', file, display_name, None),
        StatSemanticField::Group => stater.get_file_output(meta, 'G', file, display_name, None),
        StatSemanticField::Device => stater.get_file_output(meta, 'd', file, display_name, None),
        StatSemanticField::DeviceHex => stater.get_file_output(meta, 'D', file, display_name, None),
        StatSemanticField::DeviceMajor => {
            stater.get_file_output(meta, 'd', file, display_name, Some('H'))
        }
        StatSemanticField::DeviceMinor => {
            stater.get_file_output(meta, 'd', file, display_name, Some('L'))
        }
        StatSemanticField::DeviceMajorHex => {
            stater.get_file_output(meta, 'D', file, display_name, Some('H'))
        }
        StatSemanticField::DeviceMinorHex => {
            stater.get_file_output(meta, 'D', file, display_name, Some('L'))
        }
        StatSemanticField::DeviceType if is_device => {
            stater.get_file_output(meta, 'r', file, display_name, None)
        }
        StatSemanticField::DeviceTypeHex if is_device => {
            stater.get_file_output(meta, 'R', file, display_name, None)
        }
        StatSemanticField::DeviceTypeMajor if is_device => {
            stater.get_file_output(meta, 'r', file, display_name, Some('H'))
        }
        StatSemanticField::DeviceTypeMinor if is_device => {
            stater.get_file_output(meta, 'r', file, display_name, Some('L'))
        }
        StatSemanticField::DeviceTypeMajorHex if is_device => {
            stater.get_file_output(meta, 't', file, display_name, Some('H'))
        }
        StatSemanticField::DeviceTypeMinorHex if is_device => {
            stater.get_file_output(meta, 'T', file, display_name, Some('L'))
        }
        StatSemanticField::Inode => stater.get_file_output(meta, 'i', file, display_name, None),
        StatSemanticField::Links => stater.get_file_output(meta, 'h', file, display_name, None),
        StatSemanticField::MountPoint => {
            stater.get_file_output(meta, 'm', file, display_name, None)
        }
        StatSemanticField::AccessRightsOctal => {
            stater.get_file_output(meta, 'a', file, display_name, None)
        }
        StatSemanticField::AccessRightsHuman => {
            stater.get_file_output(meta, 'A', file, display_name, None)
        }
        StatSemanticField::Context => stater.get_file_output(meta, 'C', file, display_name, None),
        StatSemanticField::AccessTime => {
            stater.get_file_output(meta, 'x', file, display_name, None)
        }
        StatSemanticField::AccessEpoch => {
            stater.get_file_output(meta, 'X', file, display_name, None)
        }
        StatSemanticField::ModifyTime => {
            stater.get_file_output(meta, 'y', file, display_name, None)
        }
        StatSemanticField::ModifyEpoch => {
            stater.get_file_output(meta, 'Y', file, display_name, None)
        }
        StatSemanticField::ChangeTime => {
            stater.get_file_output(meta, 'z', file, display_name, None)
        }
        StatSemanticField::ChangeEpoch => {
            stater.get_file_output(meta, 'Z', file, display_name, None)
        }
        StatSemanticField::BirthTime => stater.get_file_output(meta, 'w', file, display_name, None),
        StatSemanticField::BirthEpoch => {
            stater.get_file_output(meta, 'W', file, display_name, None)
        }
        _ => return None,
    };

    semantic_value_from_output(field, output)
}

fn filesystem_field_value(
    stater: &Stater,
    meta: &impl FsMeta,
    display_name: &str,
    field: StatSemanticField,
) -> Option<StatSemanticValue> {
    let output = match field {
        StatSemanticField::Name => stater.get_filesystem_output(meta, 'n', display_name),
        StatSemanticField::FilesystemIdHex => stater.get_filesystem_output(meta, 'i', display_name),
        StatSemanticField::NameMax => stater.get_filesystem_output(meta, 'l', display_name),
        StatSemanticField::FilesystemTypeHex => {
            stater.get_filesystem_output(meta, 't', display_name)
        }
        StatSemanticField::FilesystemType => stater.get_filesystem_output(meta, 'T', display_name),
        StatSemanticField::BlockSize => stater.get_filesystem_output(meta, 's', display_name),
        StatSemanticField::FundamentalBlockSize => {
            stater.get_filesystem_output(meta, 'S', display_name)
        }
        StatSemanticField::TotalBlocks => stater.get_filesystem_output(meta, 'b', display_name),
        StatSemanticField::FreeBlocks => stater.get_filesystem_output(meta, 'f', display_name),
        StatSemanticField::AvailableBlocks => stater.get_filesystem_output(meta, 'a', display_name),
        StatSemanticField::TotalFileNodes => stater.get_filesystem_output(meta, 'c', display_name),
        StatSemanticField::FreeFileNodes => stater.get_filesystem_output(meta, 'd', display_name),
        _ => return None,
    };

    semantic_value_from_output(field, output)
}

fn semantic_value_from_output(
    field: StatSemanticField,
    output: StatOutputType,
) -> Option<StatSemanticValue> {
    match field {
        StatSemanticField::AccessRightsOctal => match output {
            StatOutputType::UnsignedOct(value) => {
                Some(StatSemanticValue::String(format!("{value:o}")))
            }
            _ => None,
        },
        StatSemanticField::RawModeHex
        | StatSemanticField::DeviceHex
        | StatSemanticField::DeviceMajorHex
        | StatSemanticField::DeviceMinorHex
        | StatSemanticField::DeviceTypeHex
        | StatSemanticField::DeviceTypeMajorHex
        | StatSemanticField::DeviceTypeMinorHex
        | StatSemanticField::FilesystemIdHex
        | StatSemanticField::FilesystemTypeHex => match output {
            StatOutputType::UnsignedHex(value) => {
                Some(StatSemanticValue::String(format!("{value:x}")))
            }
            _ => None,
        },
        StatSemanticField::Name
        | StatSemanticField::QuotedName
        | StatSemanticField::FileType
        | StatSemanticField::User
        | StatSemanticField::Group
        | StatSemanticField::MountPoint
        | StatSemanticField::AccessRightsHuman
        | StatSemanticField::Context
        | StatSemanticField::AccessTime
        | StatSemanticField::ModifyTime
        | StatSemanticField::ChangeTime
        | StatSemanticField::BirthTime
        | StatSemanticField::FilesystemType
        | StatSemanticField::Formatted => match output {
            StatOutputType::Str(value) if !value.is_empty() => {
                Some(StatSemanticValue::String(value))
            }
            StatOutputType::Str(_) => None,
            _ => None,
        },
        StatSemanticField::AccessEpoch
        | StatSemanticField::ModifyEpoch
        | StatSemanticField::ChangeEpoch
        | StatSemanticField::BirthEpoch => match output {
            StatOutputType::Timestamp(sec, _) => Some(StatSemanticValue::Int(sec)),
            StatOutputType::Integer(value) => Some(StatSemanticValue::Int(value)),
            StatOutputType::Unsigned(value) => {
                i64::try_from(value).ok().map(StatSemanticValue::Int)
            }
            _ => None,
        },
        _ => match output {
            StatOutputType::Integer(value) => Some(StatSemanticValue::Int(value)),
            StatOutputType::Unsigned(value) => {
                i64::try_from(value).ok().map(StatSemanticValue::Int)
            }
            _ => None,
        },
    }
}

fn render_filesystem_tokens(
    stater: &Stater,
    meta: &impl FsMeta,
    tokens: &[StatToken],
    display_name: &str,
) -> String {
    let mut text = String::new();

    for token in tokens {
        match token {
            StatToken::Char(c) => text.push(*c),
            StatToken::Byte(b) => text.push(char::from(*b)),
            StatToken::IgnoredDirective => {}
            StatToken::InvalidDirective(_) => break,
            StatToken::Directive {
                flag,
                width,
                precision,
                modifier: _,
                format,
            } => {
                let output = stater.get_filesystem_output(meta, *format, display_name);
                text.push_str(&render_output(&output, *flag, *width, *precision));
            }
        }
    }

    text
}

fn render_file_tokens(
    stater: &Stater,
    meta: &fs::Metadata,
    tokens: &[StatToken],
    file: &OsStr,
    display_name: &str,
) -> String {
    let mut text = String::new();

    for token in tokens {
        match token {
            StatToken::Char(c) => text.push(*c),
            StatToken::Byte(b) => text.push(char::from(*b)),
            StatToken::IgnoredDirective => {}
            StatToken::InvalidDirective(_) => break,
            StatToken::Directive {
                flag,
                width,
                precision,
                modifier,
                format,
            } => {
                let output = stater.get_file_output(meta, *format, file, display_name, *modifier);
                text.push_str(&render_output(&output, *flag, *width, *precision));
            }
        }
    }

    text
}

fn render_output(
    output: &StatOutputType,
    flags: StatFlags,
    width: usize,
    precision: Option<i32>,
) -> String {
    match output {
        StatOutputType::Str(value) => render_str(value, &flags, width, precision),
        StatOutputType::Integer(value) => render_integer(*value, &flags, width, precision),
        StatOutputType::Unsigned(value) => render_unsigned(*value, &flags, width, precision),
        StatOutputType::UnsignedOct(value) => render_unsigned_oct(*value, &flags, width, precision),
        StatOutputType::UnsignedHex(value) => render_unsigned_hex(*value, &flags, width, precision),
        StatOutputType::Timestamp(sec, nsec) => {
            render_timestamp(*sec, *nsec, &flags, width, precision)
        }
        StatOutputType::Unknown => "?".into(),
    }
}

fn render_str(s: &str, flags: &StatFlags, width: usize, precision: Option<i32>) -> String {
    let p = match precision {
        Some(-1) => 0,
        Some(p) => p as usize,
        None => usize::MAX,
    };
    let value = if p < s.len() { &s[..p] } else { s };
    if flags.is_left {
        format!("{value:<width$}")
    } else {
        format!("{value:>width$}")
    }
}

fn render_integer(num: i64, flags: &StatFlags, width: usize, precision: Option<i32>) -> String {
    let sign = if num < 0 {
        "-"
    } else if flags.is_sign {
        "+"
    } else if flags.is_space {
        " "
    } else {
        ""
    };
    let mut digits = decimal_digits(num.unsigned_abs(), precision);
    if flags.is_group {
        digits = group_num(&digits).into_owned();
    }
    render_numeric(sign, "", &digits, flags, width, precision)
}

fn render_unsigned(num: u64, flags: &StatFlags, width: usize, precision: Option<i32>) -> String {
    let mut digits = decimal_digits(num, precision);
    if flags.is_group {
        digits = group_num(&digits).into_owned();
    }
    render_numeric("", "", &digits, flags, width, precision)
}

fn render_unsigned_oct(
    num: u32,
    flags: &StatFlags,
    width: usize,
    precision: Option<i32>,
) -> String {
    let mut digits = if num == 0 && numeric_precision(precision) == Some(0) {
        String::new()
    } else {
        format!("{num:o}")
    };
    let mut minimum = numeric_precision(precision).unwrap_or(0);
    if flags.is_alter && !digits.starts_with('0') {
        minimum = minimum.max(digits.len() + 1);
    }
    if minimum > digits.len() {
        digits.insert_str(0, &"0".repeat(minimum - digits.len()));
    }
    render_numeric("", "", &digits, flags, width, precision)
}

fn render_unsigned_hex(
    num: u64,
    flags: &StatFlags,
    width: usize,
    precision: Option<i32>,
) -> String {
    let mut digits = if num == 0 && numeric_precision(precision) == Some(0) {
        String::new()
    } else {
        format!("{num:x}")
    };
    if let Some(minimum) = numeric_precision(precision)
        && minimum > digits.len()
    {
        digits.insert_str(0, &"0".repeat(minimum - digits.len()));
    }
    let prefix = if flags.is_alter && num != 0 { "0x" } else { "" };
    render_numeric("", prefix, &digits, flags, width, precision)
}

fn numeric_precision(precision: Option<i32>) -> Option<usize> {
    precision.map(|value| value.max(0) as usize)
}

fn decimal_digits(num: u64, precision: Option<i32>) -> String {
    let mut digits = if num == 0 && numeric_precision(precision) == Some(0) {
        String::new()
    } else {
        num.to_string()
    };
    if let Some(minimum) = numeric_precision(precision)
        && minimum > digits.len()
    {
        digits.insert_str(0, &"0".repeat(minimum - digits.len()));
    }
    digits
}

fn render_numeric(
    sign: &str,
    base_prefix: &str,
    digits: &str,
    flags: &StatFlags,
    width: usize,
    precision: Option<i32>,
) -> String {
    let value_len = sign.len() + base_prefix.len() + digits.len();
    let padding = width.saturating_sub(value_len);

    if flags.is_left {
        format!("{sign}{base_prefix}{digits}{}", " ".repeat(padding))
    } else if flags.is_zero && precision.is_none() {
        format!("{sign}{base_prefix}{}{digits}", "0".repeat(padding))
    } else {
        format!("{}{sign}{base_prefix}{digits}", " ".repeat(padding))
    }
}

fn render_timestamp(
    sec: i64,
    nsec: i64,
    flags: &StatFlags,
    width: usize,
    precision: Option<i32>,
) -> String {
    let fraction_precision = precision.map(|value| if value == -1 { 9 } else { value as usize });
    let stored_precision = fraction_precision.unwrap_or(0).min(9);
    let divisor = 10_i64.pow((9 - stored_precision) as u32);
    let mut fraction = nsec / divisor;
    let mut display_sec = sec;
    let mut negative_zero = false;

    if fraction_precision.is_some() && sec < 0 && nsec != 0 {
        let modulus = 1_000_000_000 / divisor;
        fraction = modulus - fraction - i64::from(nsec % divisor != 0);
        if fraction != 0 {
            display_sec += 1;
        }
        negative_zero = display_sec == 0;
    }

    let negative = display_sec < 0 || negative_zero;
    let mut digits = display_sec.unsigned_abs().to_string();
    if flags.is_group {
        digits = group_num(&digits).into_owned();
    }

    if let Some(fraction_precision) = fraction_precision
        && fraction_precision > 0
    {
        let mut fraction_text = format!("{fraction:0>stored_precision$}");
        fraction_text.push_str(&"0".repeat(fraction_precision - stored_precision));
        digits.push_str(&NumericLocale::current().decimal_point);
        digits.push_str(&fraction_text);
    }

    let sign = if negative {
        "-"
    } else if flags.is_sign {
        "+"
    } else if flags.is_space {
        " "
    } else {
        ""
    };

    render_numeric(sign, "", &digits, flags, width, None)
}

fn filesystem_error_description(err: &str) -> String {
    if let Some(pos) = err.find("(os error ") {
        err[..pos].trim().to_string()
    } else {
        err.to_string()
    }
}

fn file_error_description(err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
        std::io::ErrorKind::ConnectionRefused => "Connection refused".to_string(),
        std::io::ErrorKind::ConnectionReset => "Connection reset".to_string(),
        std::io::ErrorKind::ConnectionAborted => "Connection aborted".to_string(),
        std::io::ErrorKind::NotConnected => "Not connected".to_string(),
        std::io::ErrorKind::AddrInUse => "Address in use".to_string(),
        std::io::ErrorKind::AddrNotAvailable => "Address not available".to_string(),
        std::io::ErrorKind::BrokenPipe => "Broken pipe".to_string(),
        std::io::ErrorKind::AlreadyExists => "File exists".to_string(),
        std::io::ErrorKind::WouldBlock => "Resource temporarily unavailable".to_string(),
        std::io::ErrorKind::InvalidInput => "Invalid argument".to_string(),
        std::io::ErrorKind::InvalidData => "Invalid data".to_string(),
        std::io::ErrorKind::TimedOut => "Timed out".to_string(),
        std::io::ErrorKind::WriteZero => "Write zero".to_string(),
        std::io::ErrorKind::Interrupted => "Interrupted system call".to_string(),
        std::io::ErrorKind::Unsupported => "Operation not supported".to_string(),
        std::io::ErrorKind::UnexpectedEof => "Unexpected end of file".to_string(),
        std::io::ErrorKind::OutOfMemory => "Out of memory".to_string(),
        _ => {
            let text = err.to_string();
            if let Some(pos) = text.find("(os error ") {
                text[..pos].trim().to_string()
            } else {
                text
            }
        }
    }
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(stat_options::STAT_DEREFERENCE)
            .short('L')
            .long(stat_options::STAT_DEREFERENCE)
            .help(rust_i18n::t!(stat_options::STAT_DEREFERENCE))
            .action(ArgAction::SetTrue),
        Arg::new(stat_options::STAT_FILE_SYSTEM)
            .short('f')
            .long(stat_options::STAT_FILE_SYSTEM)
            .help(rust_i18n::t!(stat_options::STAT_FILE_SYSTEM))
            .action(ArgAction::SetTrue),
        Arg::new(stat_options::STAT_TERSE)
            .short('t')
            .long(stat_options::STAT_TERSE)
            .help(rust_i18n::t!(stat_options::STAT_TERSE))
            .action(ArgAction::SetTrue),
        Arg::new(stat_options::STAT_FORMAT)
            .short('c')
            .long(stat_options::STAT_FORMAT)
            .help(rust_i18n::t!(stat_options::STAT_FORMAT))
            .value_name("FORMAT")
            .overrides_with(stat_options::STAT_PRINTF),
        Arg::new(stat_options::STAT_PRINTF)
            .long(stat_options::STAT_PRINTF)
            .value_name("FORMAT")
            .help(rust_i18n::t!(stat_options::STAT_PRINTF))
            .overrides_with(stat_options::STAT_FORMAT),
        Arg::new(stat_options::STAT_CACHED)
            .long(stat_options::STAT_CACHED)
            .value_name("MODE")
            .help("specify how to use cached attributes; useful on remote file systems"),
        Arg::new(stat_options::STAT_FILES)
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(stat_options::STAT_HELP)
            .short('h')
            .long(stat_options::STAT_HELP)
            .help(rust_i18n::t!(stat_options::STAT_HELP))
            .action(ArgAction::Help),
        Arg::new(stat_options::STAT_VERSION)
            .short('v')
            .long(stat_options::STAT_VERSION)
            .help(rust_i18n::t!(stat_options::STAT_VERSION))
            .action(ArgAction::Version),
    ];
    Command::new(ctcore::ct_util_name())
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args_override_self(true)
        .version(crate_version!())
        .about(rust_i18n::t!(stat_options::STAT_ABOUT))
        .override_usage(rust_i18n::t!(stat_options::STAT_USAGE))
        .infer_long_args(true)
        .args(args)
}

const PRETTY_DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S.%f %z";

fn pretty_time(sec: i64, nsec: i64) -> String {
    // Return the date in UTC
    let tm = chrono::DateTime::from_timestamp(sec, nsec as u32).unwrap_or_default();
    let tm: DateTime<Local> = tm.into();

    tm.format(PRETTY_DATETIME_FORMAT).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from_raw_os_error(libc::ENOSPC))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn formatted_output_propagates_write_errors() {
        let error = print_it(
            &mut FailingWriter,
            &StatOutputType::Str("file".to_string()),
            StatFlags::default(),
            0,
            None,
        )
        .unwrap_err();

        assert_eq!(error.raw_os_error(), Some(libc::ENOSPC));
    }

    #[test]
    fn parses_all_gnu_quoting_style_names() {
        for style in [
            "literal",
            "shell",
            "shell-always",
            "shell-escape",
            "shell-escape-always",
            "c",
            "c-maybe",
            "escape",
            "locale",
            "clocale",
        ] {
            assert!(StatQuotingStyle::parse(style).is_some(), "style={style}");
        }
        assert!(StatQuotingStyle::parse("invalid").is_none());
    }

    #[test]
    fn shell_quoting_wraps_names_containing_control_characters() {
        let style = StatQuotingStyle::parse("shell").unwrap();
        assert_eq!(style.quote("a\nb"), "'a\nb'");
    }

    #[test]
    fn parses_directives_and_escapes_after_multibyte_characters() {
        let tokens = Stater::generate_tokens("é%9", false).unwrap();
        assert_eq!(
            tokens,
            vec![
                StatToken::Char('é'),
                StatToken::InvalidDirective("%9".to_string()),
            ]
        );

        assert_eq!(
            Stater::generate_tokens("éé\\x41|\\101", true).unwrap(),
            vec![
                StatToken::Char('é'),
                StatToken::Char('é'),
                StatToken::Byte(b'A'),
                StatToken::Char('|'),
                StatToken::Byte(b'A'),
            ]
        );
    }

    #[test]
    fn invalid_directive_preserves_already_rendered_multibyte_prefix() {
        let matches = ct_app()
            .try_get_matches_from(["stat", "-c", "é%9", "/"])
            .unwrap();
        let stater = Stater::new(&matches).unwrap();
        let mut output = Vec::new();

        let error = stater.exec(&mut output).unwrap_err();

        assert_eq!(output, "é".as_bytes());
        match error {
            StatExecutionError::InvalidDirective(message) => {
                assert_eq!(message, "'%9': invalid directive");
            }
            StatExecutionError::Write(error) => panic!("unexpected write error: {error}"),
        }
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Stat;

        // 测试 name 方法
        assert_eq!(tool.name(), "stat");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("stat"));

        // 测试 execute 方法 - 帮助命令应该返回错误，但不会崩溃
        let args = vec![OsString::from("stat"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    #[test]
    fn repeated_options_keep_the_last_value() {
        let matches = ct_app()
            .try_get_matches_from([
                "stat",
                "-L",
                "-L",
                "-f",
                "-f",
                "-t",
                "-t",
                "-c",
                "first",
                "-c",
                "second",
                "--cached=always",
                "--cached=never",
                "/",
            ])
            .unwrap();

        assert!(matches.get_flag(stat_options::STAT_DEREFERENCE));
        assert!(matches.get_flag(stat_options::STAT_FILE_SYSTEM));
        assert!(matches.get_flag(stat_options::STAT_TERSE));
        assert_eq!(
            matches.get_one::<String>(stat_options::STAT_FORMAT),
            Some(&"second".to_string())
        );
        assert_eq!(
            matches.get_one::<String>(stat_options::STAT_CACHED),
            Some(&"never".to_string())
        );
    }

    #[test]
    fn cached_mode_accepts_unique_prefixes() {
        for (value, expected) in [
            ("a", CachedMode::Always),
            ("al", CachedMode::Always),
            ("n", CachedMode::Never),
            ("ne", CachedMode::Never),
            ("d", CachedMode::Default),
            ("de", CachedMode::Default),
        ] {
            let matches = ct_app()
                .clone()
                .try_get_matches_from(["stat", &format!("--cached={value}"), "/"])
                .unwrap();
            let parsed = CachedMode::from_str(
                matches
                    .get_one::<String>(stat_options::STAT_CACHED)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(parsed, expected);
        }

        assert!(CachedMode::from_str("").is_err());
        assert!(CachedMode::from_str("other").is_err());
    }

    #[test]
    fn format_and_printf_use_the_last_option() {
        let matches = ct_app()
            .clone()
            .try_get_matches_from(["stat", "--printf=first", "--format=second", "file"])
            .unwrap();
        assert_eq!(
            Stater::configure_format(&matches).unwrap().0,
            vec![
                StatToken::Char('s'),
                StatToken::Char('e'),
                StatToken::Char('c'),
                StatToken::Char('o'),
                StatToken::Char('n'),
                StatToken::Char('d'),
                StatToken::Char('\n'),
            ]
        );

        let matches = ct_app()
            .try_get_matches_from(["stat", "--format=first", "--printf=second", "file"])
            .unwrap();
        assert_eq!(
            Stater::configure_format(&matches).unwrap().0,
            vec![
                StatToken::Char('s'),
                StatToken::Char('e'),
                StatToken::Char('c'),
                StatToken::Char('o'),
                StatToken::Char('n'),
                StatToken::Char('d'),
            ]
        );
    }

    #[test]
    fn test_scanners() {
        assert_eq!(Some((b'a', 3)), "141zxc".scan_char(8));
        assert_eq!(Some((b'\n', 2)), "12qzxc".scan_char(8)); // spell-checker:disable-line
        assert_eq!(Some((b'\r', 1)), "dqzxc".scan_char(16)); // spell-checker:disable-line
        assert_eq!(Some((0xff, 3)), "777".scan_char(8));
        assert_eq!(Some((0x00, 3)), "400".scan_char(8));
        assert_eq!(Some((0x3f, 3)), "477".scan_char(8));
        assert_eq!(None, "z2qzxc".scan_char(8)); // spell-checker:disable-line
    }

    #[test]
    fn device_modifiers_only_apply_to_d_and_r() {
        assert_eq!(
            Stater::generate_tokens("%HD|%L", false).unwrap(),
            vec![
                StatToken::Directive {
                    flag: StatFlags::default(),
                    width: 0,
                    precision: None,
                    modifier: None,
                    format: 'H',
                },
                StatToken::Char('D'),
                StatToken::Char('|'),
                StatToken::Directive {
                    flag: StatFlags::default(),
                    width: 0,
                    precision: None,
                    modifier: None,
                    format: 'L',
                },
                StatToken::Char('\n'),
            ]
        );
    }

    #[test]
    fn splits_linux_large_device_numbers() {
        let device = 4_294_049_791;
        assert_eq!(device_major(device), 511);
        assert_eq!(device_minor(device), 1_048_575);
    }

    #[test]
    fn reads_metadata_from_an_open_descriptor() {
        use std::os::fd::AsRawFd;

        let file = tempfile::tempfile().unwrap();
        file.set_len(123).unwrap();
        assert_eq!(metadata_for_fd(file.as_raw_fd()).unwrap().len(), 123);
    }

    #[test]
    fn ignores_directives_with_unrepresentable_width_or_precision() {
        for format in [
            "%999999999999999999999999n",
            "%.999999999999999999999999n",
            "%2147483648n",
        ] {
            assert_eq!(
                Stater::generate_tokens(format, false).unwrap(),
                vec![StatToken::IgnoredDirective, StatToken::Char('\n')]
            );
        }
    }

    #[test]
    fn file_size_uses_unsigned_formatting() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        temp.as_file().set_len(10).unwrap();
        let matches = ct_app()
            .try_get_matches_from(["stat", "-c", "%s", temp.path().to_str().unwrap()])
            .unwrap();
        let stater = Stater::new(&matches).unwrap();
        let metadata = temp.as_file().metadata().unwrap();

        let output = stater.get_file_output(
            &metadata,
            's',
            temp.path().as_os_str(),
            temp.path().to_str().unwrap(),
            None,
        );
        assert!(matches!(output, StatOutputType::Unsigned(10)));

        let flags = StatFlags {
            is_sign: true,
            is_zero: true,
            ..Default::default()
        };
        assert_eq!(render_output(&output, flags, 8, None), "00000010");
    }

    #[test]
    fn numeric_formatting_places_prefixes_and_honors_precision() {
        let alternate_zero = StatFlags {
            is_alter: true,
            is_zero: true,
            ..Default::default()
        };
        assert_eq!(
            render_output(
                &StatOutputType::UnsignedHex(0x81a4),
                alternate_zero,
                8,
                None,
            ),
            "0x0081a4"
        );

        let alternate = StatFlags {
            is_alter: true,
            ..Default::default()
        };
        assert_eq!(
            render_output(&StatOutputType::UnsignedOct(0o644), alternate, 0, Some(5),),
            "00644"
        );
        assert_eq!(
            render_output(
                &StatOutputType::Unsigned(0),
                StatFlags::default(),
                0,
                Some(0)
            ),
            ""
        );
        assert_eq!(
            render_output(
                &StatOutputType::UnsignedHex(0),
                StatFlags::default(),
                0,
                Some(0),
            ),
            ""
        );
    }

    #[test]
    fn filesystem_fields_use_gnu_signedness() {
        let metadata = statfs(b"/").unwrap();
        let matches = ct_app()
            .try_get_matches_from(["stat", "-f", "-c", "%b", "/"])
            .unwrap();
        let stater = Stater::new(&matches).unwrap();

        for format in ['b', 'f', 'a', 'd'] {
            assert!(matches!(
                stater.get_filesystem_output(&metadata, format, "/"),
                StatOutputType::Integer(_)
            ));
        }
        for format in ['s', 'S', 'c'] {
            assert!(matches!(
                stater.get_filesystem_output(&metadata, format, "/"),
                StatOutputType::Unsigned(_)
            ));
        }
    }

    #[test]
    fn negative_epoch_fraction_uses_mathematical_value() {
        let flags = StatFlags::default();
        assert_eq!(render_timestamp(-1, 876_543_211, &flags, 0, None), "-1");
        assert_eq!(
            render_timestamp(-1, 876_543_211, &flags, 0, Some(-1)),
            "-0.123456789"
        );
        assert_eq!(
            render_timestamp(-1, 876_543_211, &flags, 0, Some(3)),
            "-0.123"
        );

        let zero_padded = StatFlags {
            is_zero: true,
            ..Default::default()
        };
        assert_eq!(
            render_timestamp(-1, 876_543_211, &zero_padded, 13, Some(6)),
            "-00000.123456"
        );
    }

    #[cfg(unix)]
    #[test]
    fn mount_point_does_not_follow_symlink_without_dereference() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("cross-mount-link");
        symlink("/proc/version", &link).unwrap();
        let matches = ct_app()
            .try_get_matches_from(["stat", "-c", "%m", link.to_str().unwrap()])
            .unwrap();
        let stater = Stater::new(&matches).unwrap();

        assert_eq!(
            stater.find_mount_point(&link),
            stater.find_mount_point(temp.path())
        );
    }

    #[test]
    fn mount_table_is_loaded_only_for_mount_point_formats() {
        let without_mount = ct_app()
            .try_get_matches_from(["stat", "-c", "%s", "/"])
            .unwrap();
        assert!(Stater::new(&without_mount).unwrap().mount_list.is_none());

        let with_mount = ct_app()
            .try_get_matches_from(["stat", "-c", "%m", "/"])
            .unwrap();
        assert!(Stater::new(&with_mount).unwrap().mount_list.is_some());
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_group_num() {
        assert_eq!("12379821234", group_num("12379821234"));
        assert_eq!("21234", group_num("21234"));
        assert_eq!("821234", group_num("821234"));
        assert_eq!("1821234", group_num("1821234"));
        assert_eq!("1234", group_num("1234"));
        assert_eq!("234", group_num("234"));
        assert_eq!("24", group_num("24"));
        assert_eq!("4", group_num("4"));
        assert_eq!("", group_num(""));
        assert_eq!("-5", group_num("-5"));
        assert_eq!("-1234", group_num("-1234"));
    }

    #[test]
    #[should_panic]
    fn test_group_num_panic_if_invalid_numeric_characters() {
        group_num("³³³³³");
    }

    #[test]
    fn test_pretty_time_returns_expected_prefix() {
        let formatted = pretty_time(0, 0);
        assert!(
            formatted.contains("1970-01-01"),
            "格式化结果应包含 Unix Epoch 日期，当前输出为 {formatted}"
        );
    }

    #[test]
    fn normal_format() {
        let s = "%'010.2ac%-#5.w\n";
        let expected = vec![
            StatToken::Directive {
                flag: StatFlags {
                    is_group: true,
                    is_zero: true,
                    ..Default::default()
                },
                width: 10,
                precision: Some(2),
                modifier: None,
                format: 'a',
            },
            StatToken::Char('c'),
            StatToken::Directive {
                flag: StatFlags {
                    is_left: true,
                    is_alter: true,
                    ..Default::default()
                },
                width: 5,
                precision: Some(-1),
                modifier: None,
                format: 'w',
            },
            StatToken::Char('\n'),
        ];
        assert_eq!(&expected, &Stater::generate_tokens(s, false).unwrap());
    }

    #[test]
    fn printf_format() {
        let s = r#"%-# 15a\t\r\"\\\a\b\e\f\v%+020w\x12\167\132\112\n"#;
        let expected = vec![
            StatToken::Directive {
                flag: StatFlags {
                    is_left: true,
                    is_alter: true,
                    is_space: true,
                    ..Default::default()
                },
                width: 15,
                precision: None,
                modifier: None,
                format: 'a',
            },
            StatToken::Byte(b'\t'),
            StatToken::Byte(b'\r'),
            StatToken::Char('"'),
            StatToken::Char('\\'),
            StatToken::Byte(b'\x07'),
            StatToken::Byte(b'\x08'),
            StatToken::Byte(b'\x1B'),
            StatToken::Byte(b'\x0C'),
            StatToken::Byte(b'\x0B'),
            StatToken::Directive {
                flag: StatFlags {
                    is_sign: true,
                    is_zero: true,
                    ..Default::default()
                },
                width: 20,
                precision: None,
                modifier: None,
                format: 'w',
            },
            StatToken::Byte(b'\x12'),
            StatToken::Byte(b'w'),
            StatToken::Byte(b'Z'),
            StatToken::Byte(b'J'),
            StatToken::Byte(b'\n'),
        ];
        assert_eq!(&expected, &Stater::generate_tokens(s, true).unwrap());
    }
}

#[cfg(test)]
mod test_stat_all {
    use super::*;
    use clap::ArgMatches;
    use std::fs::File;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn create_test_matches(
        files: Vec<&str>,
        show_fs: bool,
        format: Option<&str>,
        use_printf: bool,
    ) -> ArgMatches {
        let cmd = ct_app();
        let mut args = vec!["stat"]; // 添加程序名称作为第一个参数

        if show_fs {
            args.push("-f");
        }

        if let Some(fmt) = format {
            if use_printf {
                args.extend_from_slice(&["--printf", fmt]);
            } else {
                args.extend_from_slice(&["-c", fmt]);
            }
        }

        // 添加文件参数
        args.extend(files);

        cmd.try_get_matches_from(args).unwrap()
    }

    #[test]
    fn test_get_files() {
        // Test empty files
        let matches = create_test_matches(vec![], false, None, false);
        assert!(Stater::get_files(&matches).is_err());

        // Test single file
        let matches = create_test_matches(vec!["file.txt"], false, None, false);
        let files = Stater::get_files(&matches).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], OsString::from("file.txt"));

        // Test multiple files
        let matches = create_test_matches(vec!["file1.txt", "file2.txt"], false, None, false);
        let files = Stater::get_files(&matches).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0], OsString::from("file1.txt"));
        assert_eq!(files[1], OsString::from("file2.txt"));
    }

    #[test]
    fn test_configure_format() {
        // Test default format
        let matches = create_test_matches(vec!["file.txt"], false, None, false);
        let (tokens, dev_tokens) = Stater::configure_format(&matches).unwrap();
        assert!(!tokens.is_empty());
        assert!(!dev_tokens.is_empty());

        // Test custom format
        let matches = create_test_matches(vec!["file.txt"], false, Some("%n %s"), false);
        let (tokens, _) = Stater::configure_format(&matches).unwrap();
        // %n + space + %s + newline = 4 tokens
        assert_eq!(tokens.len(), 4);

        // Test printf format
        let matches = create_test_matches(vec!["file.txt"], false, Some("%n\\n"), true);
        let (tokens, _) = Stater::configure_format(&matches).unwrap();
        // %n + \n = 2 tokens (printf mode doesn't add extra newline)
        assert_eq!(tokens.len(), 2);

        // Additional test cases to verify token parsing
        let matches = create_test_matches(vec!["file.txt"], false, Some("simple"), false);
        let (tokens, _) = Stater::configure_format(&matches).unwrap();
        // "simple" + newline = 7 tokens
        assert_eq!(tokens.len(), 7);

        let matches = create_test_matches(vec!["file.txt"], false, Some(""), false);
        let (tokens, _) = Stater::configure_format(&matches).unwrap();
        assert_eq!(tokens, vec![StatToken::Char('\n')]);

        let matches = create_test_matches(vec!["file.txt"], false, Some(""), true);
        let (tokens, _) = Stater::configure_format(&matches).unwrap();
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_get_mount_list() {
        let mount_list = Stater::get_mount_list().unwrap();
        assert!(mount_list.is_some());
        let list = mount_list.unwrap();
        assert!(!list.is_empty());

        // 验证列表已排序
        let mut sorted_list = list.clone();
        sorted_list.sort();
        sorted_list.reverse();
        assert_eq!(
            list, sorted_list,
            "Mount list should be sorted in reverse order"
        );

        // 打印挂载点列表以便调试
        #[cfg(test)]
        {
            println!("Mount points (sorted by length):");
            for path in list.iter() {
                println!("{}: {}", path.len(), path);
            }
        }
    }

    #[test]
    fn test_find_mount_point() {
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path();

        let matches =
            create_test_matches(vec![temp_path.to_str().unwrap()], false, Some("%m"), false);
        let stater = Stater::new(&matches).unwrap();

        let mount_point = stater.find_mount_point(temp_path);
        assert!(mount_point.is_some());
    }

    #[test]
    fn test_resolve_file_path() {
        let matches = create_test_matches(vec!["file.txt"], false, None, false);
        let stater = Stater::new(&matches).unwrap();

        // Test normal file
        let result = stater.resolve_file_path("file.txt", false);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), OsString::from("file.txt"));

        // Test stdin in filesystem mode
        let stater = Stater::new(&create_test_matches(vec!["-"], true, None, false)).unwrap();
        let result = stater.resolve_file_path("-", true);
        assert!(result.is_err());
    }

    #[test]
    fn test_handle_filesystem_stat() {
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path();

        let matches = create_test_matches(vec![temp_path.to_str().unwrap()], true, None, false);
        let stater = Stater::new(&matches).unwrap();

        let mut output = Vec::new();
        let result = stater
            .handle_filesystem_stat(
                &OsString::from(temp_path.as_os_str()),
                temp_path.to_string_lossy().as_ref(),
                &mut output,
            )
            .unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_handle_file_stat() {
        let temp_dir = tempdir().unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        File::create(&temp_file).unwrap();

        let file_path = temp_file.to_str().unwrap();

        // 确保文件路径被正确添加到参数中
        let matches = create_test_matches(vec![file_path], false, None, false);

        // 创建 Stater 实例前先验证参数
        assert!(matches.contains_id(stat_options::STAT_FILES));

        let stater = Stater::new(&matches).unwrap();

        let mut output = Vec::new();
        let result = stater
            .handle_file_stat(
                &OsString::from(temp_file.as_os_str()),
                temp_file.to_string_lossy().as_ref(),
                false,
                &mut output,
            )
            .unwrap();
        assert_eq!(result, 0);
    }

    #[cfg(unix)]
    #[test]
    fn test_default_format_shows_symlink_target_and_captured_access_time() {
        let temp_dir = tempdir().unwrap();
        let target = temp_dir.path().join("content.txt");
        let link = temp_dir.path().join("content.link");
        fs::write(&target, b"alpha\nbeta\n").unwrap();
        symlink("content.txt", &link).unwrap();
        rustix::fs::utimensat(
            rustix::fs::CWD,
            &link,
            &rustix::fs::Timestamps {
                last_access: rustix::fs::Timespec {
                    tv_sec: 1_893_456_000,
                    tv_nsec: 0,
                },
                last_modification: rustix::fs::Timespec {
                    tv_sec: 946_684_800,
                    tv_nsec: 0,
                },
            },
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .unwrap();

        let display_name = "content.link";
        let matches = create_test_matches(vec![display_name], false, None, false);
        let stater = Stater::new(&matches).unwrap();
        let meta = fs::symlink_metadata(&link).unwrap();
        let expected_access_time = pretty_time(meta.atime(), meta.atime_nsec());
        let rendered = render_file_tokens(
            &stater,
            &meta,
            stater.select_tokens(&meta),
            link.as_os_str(),
            display_name,
        );

        assert!(
            rendered
                .lines()
                .next()
                .is_some_and(|line| line.ends_with("content.link -> content.txt")),
            "unexpected default symlink heading: {rendered}"
        );
        assert!(
            rendered.contains(&expected_access_time),
            "default output did not use the metadata captured before readlink: {rendered}"
        );
    }

    #[test]
    fn test_handle_file_stat_returns_failure_for_unreadable_context() {
        let temp_dir = tempdir().unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        File::create(&temp_file).unwrap();

        if matches!(
            selinux::SecurityContext::of_path(temp_file.as_os_str(), false, false),
            Ok(Some(_))
        ) {
            return;
        }

        let file_path = temp_file.to_str().unwrap();
        let matches = create_test_matches(vec![file_path], false, Some("%C"), false);
        let stater = Stater::new(&matches).unwrap();

        let mut output = Vec::new();
        let result = stater
            .handle_file_stat(
                &OsString::from(temp_file.as_os_str()),
                temp_file.to_string_lossy().as_ref(),
                false,
                &mut output,
            )
            .unwrap();

        assert_eq!(result, 1);
    }

    #[test]
    fn test_select_tokens() {
        let temp_dir = tempdir().unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        File::create(&temp_file).unwrap();

        let matches = create_test_matches(
            vec![temp_file.to_str().unwrap()],
            false,
            Some("%n %s"),
            false,
        );
        let stater = Stater::new(&matches).unwrap();

        let metadata = fs::metadata(&temp_file).unwrap();
        let tokens = stater.select_tokens(&metadata);
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_get_filesystem_output() {
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path();

        let matches = create_test_matches(vec![temp_path.to_str().unwrap()], true, None, false);
        let stater = Stater::new(&matches).unwrap();

        let fs_meta = statfs(temp_path.as_os_str().as_bytes()).unwrap();

        // Test various format specifiers
        let output = stater.get_filesystem_output(&fs_meta, 'b', temp_path.to_str().unwrap());
        assert!(matches!(output, StatOutputType::Integer(_)));

        let output = stater.get_filesystem_output(&fs_meta, 'T', temp_path.to_str().unwrap());
        assert!(matches!(output, StatOutputType::Str(_)));
    }

    #[test]
    fn test_get_file_output() {
        let temp_dir = tempdir().unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        File::create(&temp_file).unwrap();

        let matches = create_test_matches(vec![temp_file.to_str().unwrap()], false, None, false);
        let stater = Stater::new(&matches).unwrap();

        let metadata = fs::metadata(&temp_file).unwrap();

        // Test various format specifiers
        let output = stater.get_file_output(
            &metadata,
            'n',
            temp_file.as_os_str(),
            temp_file.to_str().unwrap(),
            None,
        );
        assert!(matches!(output, StatOutputType::Str(_)));

        let output = stater.get_file_output(
            &metadata,
            's',
            temp_file.as_os_str(),
            temp_file.to_str().unwrap(),
            None,
        );
        assert!(matches!(output, StatOutputType::Unsigned(_)));
    }
}

#[cfg(test)]
mod test_i18n {
    use super::*;

    #[test]
    fn test_help_messages() {
        println!("\n=== Testing help messages ===");

        // 设置中文语言环境
        println!("Setting locale to zh-CN");
        rust_i18n::set_locale("zh-CN");
        let cmd = ct_app();

        // 测试帮助信息
        let helps = cmd.get_about().unwrap().to_string();

        //let helps = "hello";
        let trans = rust_i18n::t!(&helps);
        println!("\nOriginal help text:\n{helps}");
        println!("\nTranslated help text:\n{trans}");

        // 测试参数描述
        println!("\nArgument descriptions:");
        let args: Vec<_> = cmd.get_arguments().collect();
        for arg in args {
            if let Some(help) = arg.get_help() {
                println!("- {} => {}", arg.get_id(), help);
            }
        }

        println!("=== Test completed ===\n");
    }
}

#[cfg(test)]
mod test_cached {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn test_get_metadata_with_cached_modes() {
        // 创建一个临时文件用于测试
        let temp_dir = tempdir().unwrap();
        let temp_file = temp_dir.path().join("test_file.txt");
        File::create(&temp_file).unwrap();

        // 测试不同的缓存模式
        let file_path = OsString::from(&temp_file);

        // 测试 Default 模式
        let result = get_metadata(&file_path, true, CachedMode::Default);
        assert!(result.is_ok());

        // 测试 Always 模式
        let result = get_metadata(&file_path, true, CachedMode::Always);
        assert!(result.is_ok());

        // 测试 Never 模式
        let result = get_metadata(&file_path, true, CachedMode::Never);
        assert!(result.is_ok());

        // 测试不跟随符号链接
        let result = get_metadata(&file_path, false, CachedMode::Default);
        assert!(result.is_ok());
    }
}
