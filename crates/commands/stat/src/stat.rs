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
use ctcore::libc::mode_t;
use ctcore::{ct_entries, ct_show_error, ct_show_warning};
use rustix::fs::{AtFlags, StatxFlags, statx};
use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::prelude::OsStrExt;
use std::path::Path;

// 声明 i18n 宏和初始化函数
rust_i18n::i18n!("locales", fallback = "zh-CN");
use sys_locale::get_locale;

mod stat_options {
    pub const STAT_DEREFERENCE: &str = "dereference";
    pub const STAT_FILE_SYSTEM: &str = "file-system";
    pub const STAT_FORMAT: &str = "ct_format";
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
        match s {
            "default" => Ok(CachedMode::Default),
            "never" => Ok(CachedMode::Never),
            "always" => Ok(CachedMode::Always),
            _ => Err(format!("invalid cached mode: {}", s)),
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
}

enum StatPadding {
    Zero,
    Space,
}

/// pads the string with zeroes or spaces and prints it
///
/// # Example
/// ```ignore
/// ct_stat::pad_and_print("1", false, 5, Padding::Zero) == "00001";
/// ```
/// currently only supports '0' & ' ' as the padding character
/// because the ct_format specification of print! does not support general
/// fill characters.
fn pad_and_print(result: &str, left: bool, width: usize, padding: StatPadding) {
    match (left, padding) {
        (false, StatPadding::Zero) => print!("{result:0>width$}"),
        (false, StatPadding::Space) => print!("{result:>width$}"),
        (true, StatPadding::Zero) => print!("{result:0<width$}"),
        (true, StatPadding::Space) => print!("{result:<width$}"),
    };
}

#[derive(Debug)]
pub enum StatOutputType {
    Str(String),
    Integer(i64),
    Unsigned(u64),
    UnsignedHex(u64),
    UnsignedOct(u32),
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
enum StatToken {
    Char(char),
    Directive {
        flag: StatFlags,
        width: usize,
        precision: Option<usize>,
        modifier: Option<char>,
        format: char,
    },
}

trait ScanUtil {
    fn scan_char(&self, radix: u32) -> Option<(char, usize)>;
}

impl ScanUtil for str {
    fn scan_char(&self, radix: u32) -> Option<(char, usize)> {
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
                    let tmp = res * radix + digit;
                    if tmp < 256 {
                        res = tmp;
                    } else {
                        break;
                    }
                }
                None => break,
            }
            offset = i + 1;
        }
        if offset > 0 {
            Some((res as u8 as char, offset))
        } else {
            None
        }
    }
}

fn group_num(s: &str) -> Cow<str> {
    let is_negative = s.starts_with('-');
    assert!(is_negative || s.chars().take(1).all(|c| c.is_ascii_digit()));
    assert!(s.chars().skip(1).all(|c| c.is_ascii_digit()));
    if s.len() < 4 {
        return s.into();
    }
    let mut res = String::with_capacity((s.len() - 1) / 3);
    let s = if is_negative {
        res.push('-');
        &s[1..]
    } else {
        s
    };
    let mut alone = (s.len() - 1) % 3 + 1;
    res.push_str(&s[..alone]);
    while alone != s.len() {
        res.push(',');
        res.push_str(&s[alone..alone + 3]);
        alone += 3;
    }
    res.into()
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
fn print_it(output: &StatOutputType, flags: StatFlags, width: usize, precision: Option<usize>) {
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
    let padding_char = determine_padding_char(&flags, &precision);

    match output {
        StatOutputType::Str(s) => print_str(s, &flags, width, precision),
        StatOutputType::Integer(num) => print_integer(*num, &flags, width, precision, padding_char),
        StatOutputType::Unsigned(num) => {
            print_unsigned(*num, &flags, width, precision, padding_char)
        }
        StatOutputType::UnsignedOct(num) => {
            print_unsigned_oct(*num, &flags, width, precision, padding_char);
        }
        StatOutputType::UnsignedHex(num) => {
            print_unsigned_hex(*num, &flags, width, precision, padding_char);
        }
        StatOutputType::Unknown => print!("?"),
    }
}

/// Determines the padding character based on the provided flags and precision.
///
/// # Arguments
///
/// * `flags` - A reference to the Flags struct containing formatting flags.
/// * `precision` - An Option containing the precision value.
///
/// # Returns
///
/// * Padding - An instance of the Padding enum representing the padding character.
fn determine_padding_char(flags: &StatFlags, precision: &Option<usize>) -> StatPadding {
    if flags.is_zero && !flags.is_left && precision.is_none() {
        StatPadding::Zero
    } else {
        StatPadding::Space
    }
}

/// Prints a string value based on the provided flags, width, and precision.
///
/// # Arguments
///
/// * `s` - The string to be printed.
/// * `flags` - A reference to the Flags struct containing formatting flags.
/// * `width` - The width of the field for the printed string.
/// * `precision` - An Option containing the precision value.
fn print_str(s: &str, flags: &StatFlags, width: usize, precision: Option<usize>) {
    let s = match precision {
        Some(p) if p < s.len() => &s[..p],
        _ => s,
    };
    pad_and_print(s, flags.is_left, width, StatPadding::Space);
}

/// Prints an integer value based on the provided flags, width, and precision.
///
/// # Arguments
///
/// * `num` - The integer value to be printed.
/// * `flags` - A reference to the Flags struct containing formatting flags.
/// * `width` - The width of the field for the printed integer.
/// * `precision` - An Option containing the precision value.
/// * `padding_char` - The padding character as determined by `determine_padding_char`.
fn print_integer(
    num: i64,
    flags: &StatFlags,
    width: usize,
    precision: Option<usize>,
    padding_char: StatPadding,
) {
    let num = num.to_string();
    let arg = if flags.is_group {
        group_num(&num)
    } else {
        Cow::Borrowed(num.as_str())
    };
    let prefix = if flags.is_sign {
        "+"
    } else if flags.is_space {
        " "
    } else {
        ""
    };
    let extended = format!(
        "{prefix}{arg:0>precision$}",
        precision = precision.unwrap_or(0)
    );
    pad_and_print(&extended, flags.is_left, width, padding_char);
}

/// Prints an unsigned integer value based on the provided flags, width, and precision.
///
/// # Arguments
///
/// * `num` - The unsigned integer value to be printed.
/// * `flags` - A reference to the Flags struct containing formatting flags.
/// * `width` - The width of the field for the printed unsigned integer.
/// * `precision` - An Option containing the precision value.
/// * `padding_char` - The padding character as determined by `determine_padding_char`.
fn print_unsigned(
    num: u64,
    flags: &StatFlags,
    width: usize,
    precision: Option<usize>,
    padding_char: StatPadding,
) {
    let num = num.to_string();
    let s = if flags.is_group {
        group_num(&num)
    } else {
        Cow::Borrowed(num.as_str())
    };
    let s = format!("{s:0>precision$}", precision = precision.unwrap_or(0));
    pad_and_print(&s, flags.is_left, width, padding_char);
}

/// Prints an unsigned octal integer value based on the provided flags, width, and precision.
///
/// # Arguments
///
/// * `num` - The unsigned octal integer value to be printed.
/// * `flags` - A reference to the Flags struct containing formatting flags.
/// * `width` - The width of the field for the printed unsigned octal integer.
/// * `precision` - An Option containing the precision value.
/// * `padding_char` - The padding character as determined by `determine_padding_char`.
fn print_unsigned_oct(
    num: u32,
    flags: &StatFlags,
    width: usize,
    precision: Option<usize>,
    padding_char: StatPadding,
) {
    let prefix = if flags.is_alter { "0" } else { "" };
    let s = format!(
        "{prefix}{num:0>precision$o}",
        precision = precision.unwrap_or(0)
    );
    pad_and_print(&s, flags.is_left, width, padding_char);
}

/// Prints an unsigned hexadecimal integer value based on the provided flags, width, and precision.
///
/// # Arguments
///
/// * `num` - The unsigned hexadecimal integer value to be printed.
/// * `flags` - A reference to the Flags struct containing formatting flags.
/// * `width` - The width of the field for the printed unsigned hexadecimal integer.
/// * `precision` - An Option containing the precision value.
/// * `padding_char` - The padding character as determined by `determine_padding_char`.
fn print_unsigned_hex(
    num: u64,
    flags: &StatFlags,
    width: usize,
    precision: Option<usize>,
    padding_char: StatPadding,
) {
    let prefix = if flags.is_alter { "0x" } else { "" };
    let s = format!(
        "{prefix}{num:0>precision$x}",
        precision = precision.unwrap_or(0)
    );
    pad_and_print(&s, flags.is_left, width, padding_char);
}

impl Stater {
    fn handle_percent_case(
        chars: &[char],
        i: &mut usize,
        bound: usize,
        format_str: &str,
    ) -> CTResult<StatToken> {
        let old = *i;

        *i += 1;
        if *i >= bound {
            return Ok(StatToken::Char('%'));
        }
        if chars[*i] == '%' {
            *i += 1;
            return Ok(StatToken::Char('%'));
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
                'I' => unimplemented!(),
                _ => break,
            }
            *i += 1;
        }

        let mut width = 0;
        let mut precision = None;
        let mut j = *i;

        // 使用 chars 数组来处理数字
        while j < bound && chars[j].is_ascii_digit() {
            width = width * 10 + chars[j].to_digit(10).unwrap() as usize;
            j += 1;
        }

        if j < bound && chars[j] == '.' {
            j += 1;
            if j < bound {
                let mut prec = 0;
                let mut has_precision = false;
                while j < bound && chars[j].is_ascii_digit() {
                    prec = prec * 10 + chars[j].to_digit(10).unwrap() as usize;
                    has_precision = true;
                    j += 1;
                }
                if has_precision {
                    precision = Some(prec);
                } else {
                    precision = Some(0);
                }
            }
        }

        *i = j;
        if *i >= bound {
            return Err(CtSimpleError::new(
                1,
                format!("{}: invalid directive", &format_str[old..]),
            ));
        }

        // 检查修饰符 (H, L)
        let mut modifier = None;
        if chars[*i] == 'H' || chars[*i] == 'L' {
            modifier = Some(chars[*i]);
            *i += 1;
            if *i >= bound {
                return Err(CtSimpleError::new(
                    1,
                    format!("{}: invalid directive", &format_str[old..]),
                ));
            }
        }

        Ok(StatToken::Directive {
            width,
            flag,
            precision,
            modifier,
            format: chars[*i],
        })
    }

    fn handle_escape_sequences(
        chars: &[char],
        i: &mut usize,
        bound: usize,
        format_str: &str,
    ) -> StatToken {
        *i += 1;
        if *i >= bound {
            ct_show_warning!("backslash at end of ct_format");
            return StatToken::Char('\\');
        }
        match chars[*i] {
            'x' if *i + 1 < bound => {
                if let Some((c, offset)) = format_str[*i + 1..].scan_char(16) {
                    *i += offset;
                    StatToken::Char(c)
                } else {
                    ct_show_warning!("unrecognized escape '\\x'");
                    StatToken::Char('x')
                }
            }
            '0'..='7' => {
                let (c, offset) = format_str[*i..].scan_char(8).unwrap();
                *i += offset - 1;
                StatToken::Char(c)
            }
            '"' => StatToken::Char('"'),
            '\\' => StatToken::Char('\\'),
            'a' => StatToken::Char('\x07'),
            'b' => StatToken::Char('\x08'),
            'e' => StatToken::Char('\x1B'),
            'f' => StatToken::Char('\x0C'),
            'n' => StatToken::Char('\n'),
            'r' => StatToken::Char('\r'),
            't' => StatToken::Char('\t'),
            'v' => StatToken::Char('\x0B'),
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
            match chars[i] {
                '%' => tokens.push(Self::handle_percent_case(
                    &chars, &mut i, bound, format_str,
                )?),
                '\\' => {
                    if use_printf {
                        tokens.push(Self::handle_escape_sequences(
                            &chars, &mut i, bound, format_str,
                        ));
                    } else {
                        tokens.push(StatToken::Char('\\'));
                    }
                }
                c => tokens.push(StatToken::Char(c)),
            }
            i += 1;
        }
        if !use_printf && !format_str.ends_with('\n') {
            tokens.push(StatToken::Char('\n'));
        }
        Ok(tokens)
    }

    fn new(matches: &ArgMatches) -> CTResult<Self> {
        // Get files
        let files = Self::get_files(matches)?;

        // Get format configuration
        let (default_tokens, default_dev_tokens) = Self::configure_format(matches)?;

        // Get mount list if needed
        let mount_list = if matches.get_flag(stat_options::STAT_FILE_SYSTEM) {
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
            is_show_fs: matches.get_flag(stat_options::STAT_FILE_SYSTEM),
            is_from_user: matches.contains_id(stat_options::STAT_FORMAT)
                || matches.contains_id(stat_options::STAT_PRINTF),
            cached_mode,
            files,
            mount_list,
            default_tokens,
            default_dev_tokens,
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
            matches
                .get_one::<String>(stat_options::STAT_PRINTF)
                .expect("Invalid format string")
        } else {
            matches
                .get_one::<String>(stat_options::STAT_FORMAT)
                .map(|s| s.as_str())
                .unwrap_or("")
        };

        let use_printf = matches.contains_id(stat_options::STAT_PRINTF);
        let terse = matches.get_flag(stat_options::STAT_TERSE);
        let show_fs = matches.get_flag(stat_options::STAT_FILE_SYSTEM);

        let default_tokens = if format_str.is_empty() {
            Self::generate_tokens(&Self::default_format(show_fs, terse, false), use_printf)?
        } else {
            Self::generate_tokens(format_str, use_printf)?
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
        let path = p.as_ref().canonicalize().ok()?;

        for root in self.mount_list.as_ref()? {
            if path.starts_with(root) {
                return Some(root.clone());
            }
        }
        None
    }

    fn exec(&self) -> i32 {
        let mut stdin_is_fifo = false;
        if cfg!(unix) {
            if let Ok(md) = fs::metadata("/dev/stdin") {
                stdin_is_fifo = md.file_type().is_fifo();
            }
        }

        let mut ret = 0;
        for f in &self.files {
            ret |= self.do_stat(f, stdin_is_fifo);
        }
        ret
    }

    fn do_stat(&self, file: &OsStr, stdin_is_fifo: bool) -> i32 {
        let display_name = file.to_string_lossy();

        // Handle file path resolution
        let file = match self.resolve_file_path(display_name.as_ref(), stdin_is_fifo) {
            Ok(path) => path,
            Err(status) => return status,
        };

        // Process based on mode (filesystem or file)
        if self.is_show_fs {
            self.handle_filesystem_stat(&file, display_name.as_ref())
        } else {
            self.handle_file_stat(&file, display_name.as_ref(), stdin_is_fifo)
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

        Ok(if let Ok(p) = Path::new("/dev/stdin").canonicalize() {
            p.into_os_string()
        } else {
            OsString::from("/dev/stdin")
        })
    }

    fn handle_filesystem_stat(&self, file: &OsStr, display_name: &str) -> i32 {
        #[cfg(unix)]
        let path = file.as_bytes();
        #[cfg(not(unix))]
        let path = file.to_string_lossy();

        // 注意：与 GNU coreutils 的 stat 实现一致，statfs 函数不支持 cached 选项
        // 缓存控制选项仅影响文件元数据获取，不影响文件系统信息获取

        match statfs(path) {
            Ok(meta) => {
                self.print_filesystem_info(&meta, &self.default_tokens);
                0
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
                1
            }
        }
    }

    fn handle_file_stat(&self, file: &OsStr, display_name: &str, stdin_is_fifo: bool) -> i32 {
        let result = if stdin_is_fifo && display_name == "-" {
            // 对于标准输入，使用标准库的方法
            fs::metadata(file)
        } else {
            // 使用我们的新函数，它会根据缓存模式设置适当的标志
            get_metadata(file, self.is_follow, self.cached_mode)
        };

        match result {
            Ok(meta) => {
                let tokens = self.select_tokens(&meta);
                self.print_file_info(&meta, tokens, file);
                0
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
                ct_show_error!(
                    "cannot statx {}: {}",
                    display_name.quote(),
                    error_description
                );
                1
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

    fn print_filesystem_info(&self, meta: &impl FsMeta, tokens: &[StatToken]) {
        for token in tokens {
            match token {
                StatToken::Char(c) => print!("{c}"),
                StatToken::Directive {
                    flag,
                    width,
                    precision,
                    modifier: _,
                    format,
                } => {
                    let output = self.get_filesystem_output(meta, *format);
                    print_it(&output, *flag, *width, *precision);
                }
            }
        }
    }

    fn print_file_info(&self, meta: &fs::Metadata, tokens: &[StatToken], file: &OsStr) {
        for token in tokens {
            match token {
                StatToken::Char(c) => print!("{c}"),
                StatToken::Directive {
                    flag,
                    width,
                    precision,
                    modifier,
                    format,
                } => {
                    let output = self.get_file_output(meta, *format, file, *modifier);
                    print_it(&output, *flag, *width, *precision);
                }
            }
        }
    }

    fn get_filesystem_output(&self, meta: &impl FsMeta, format: char) -> StatOutputType {
        match format {
            // free blocks available to non-superuser
            'a' => StatOutputType::Unsigned(meta.avail_blocks()),
            // total data blocks in file system
            'b' => StatOutputType::Unsigned(meta.total_blocks()),
            // total file nodes in file system
            'c' => StatOutputType::Unsigned(meta.total_file_nodes()),
            // free file nodes in file system
            'd' => StatOutputType::Unsigned(meta.free_file_nodes()),
            // free blocks in file system
            'f' => StatOutputType::Unsigned(meta.free_blocks()),
            // file system ID in hex
            'i' => StatOutputType::UnsignedHex(meta.fsid()),
            // maximum length of filenames
            'l' => StatOutputType::Unsigned(meta.namelen()),
            // file name
            'n' => StatOutputType::Str(
                self.files
                    .first()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            // block size (for faster transfers)
            's' => StatOutputType::Unsigned(meta.io_size()),
            // fundamental block size (for block counts)
            'S' => StatOutputType::Integer(meta.block_size()),
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
        modifier: Option<char>,
    ) -> StatOutputType {
        let display_name = file.to_string_lossy();
        let file_type = meta.file_type();
        let mut context_str = "".to_string();
        let substitute_string = "?".to_string();

        if !file.is_empty() {
            context_str = match selinux::SecurityContext::of_path(file, false, false) {
                Err(_r) => {
                    // TODO: show the actual reason why it failed
                    ct_show_warning!("failed to get security context of: {}", file.quote());
                    substitute_string
                }

                Ok(None) => substitute_string,
                Ok(Some(context)) => {
                    let context = context.as_bytes();
                    let context_strip_suffix = context.strip_suffix(&[0]).unwrap_or(context);
                    String::from_utf8(context_strip_suffix.to_vec()).unwrap_or_else(|e| {
                        ct_show_warning!(
                            "getting security context of: {}: {}",
                            file.quote(),
                            e.to_string()
                        );
                        String::from_utf8_lossy(context_strip_suffix).into_owned()
                    })
                }
            }
        }

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
            'C' => StatOutputType::Str(context_str),
            // device number - handle modifier for major/minor separation
            'd' => {
                match modifier {
                    Some('H') => StatOutputType::Unsigned(meta.dev() >> 8), // major device number
                    Some('L') => StatOutputType::Unsigned(meta.dev() & 0xff), // minor device number
                    _ => StatOutputType::Unsigned(meta.dev()),              // full device number
                }
            }
            // device number in hex
            'D' => StatOutputType::UnsignedHex(meta.dev()),
            // raw mode in hex
            'f' => StatOutputType::UnsignedHex(meta.mode() as u64),
            // file type
            'F' => {
                StatOutputType::Str(pretty_filetype(meta.mode() as mode_t, meta.len()).to_owned())
            }
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
            'm' => StatOutputType::Str(
                self.find_mount_point(display_name.as_ref())
                    .unwrap_or_default(),
            ),
            // file name
            'n' => StatOutputType::Str(display_name.to_string()),
            // quoted file name with dereference if symbolic link
            'N' => {
                let file_name = if file_type.is_symlink() {
                    let dst = match fs::read_link(display_name.as_ref()) {
                        Ok(path) => path,
                        Err(e) => {
                            println!("{e}");
                            return StatOutputType::Unknown;
                        }
                    };
                    format!("'{}' -> '{}'", display_name, dst.display())
                } else {
                    format!("'{}'", display_name)
                };
                StatOutputType::Str(file_name)
            }
            // optimal I/O transfer size hint
            'o' => StatOutputType::Unsigned(meta.blksize()),
            // total size, in bytes
            's' => StatOutputType::Integer(meta.len() as i64),
            // major device type in hex, for character/block device special
            // files, with modifier support
            't' => {
                match modifier {
                    Some('H') => StatOutputType::UnsignedHex(meta.rdev() >> 8), // major device type
                    Some('L') => StatOutputType::UnsignedHex(meta.rdev() & 0xff), // minor device type
                    _ => StatOutputType::UnsignedHex(meta.rdev() >> 8), // default to major
                }
            }
            // minor device type in hex, for character/block device special
            // files, with modifier support
            'T' => {
                match modifier {
                    Some('H') => StatOutputType::UnsignedHex(meta.rdev() >> 8), // major device type
                    Some('L') => StatOutputType::UnsignedHex(meta.rdev() & 0xff), // minor device type
                    _ => StatOutputType::UnsignedHex(meta.rdev() & 0xff), // default to minor
                }
            }
            // device type (r format) - mainly used with modifiers %Hr,%Lr
            'r' => {
                match modifier {
                    Some('H') => StatOutputType::UnsignedHex(meta.rdev() >> 8), // major device type
                    Some('L') => StatOutputType::UnsignedHex(meta.rdev() & 0xff), // minor device type
                    _ => StatOutputType::UnsignedHex(meta.rdev()),                // full rdev
                }
            }
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
            'W' => StatOutputType::Unsigned(meta.birth().unwrap_or_default().0),

            // time of last access, human-readable
            'x' => StatOutputType::Str(pretty_time(meta.atime(), meta.atime_nsec())),
            // time of last access, seconds since Epoch
            'X' => StatOutputType::Integer(meta.atime()),
            // time of last data modification, human-readable
            'y' => StatOutputType::Str(pretty_time(meta.mtime(), meta.mtime_nsec())),
            // time of last data modification, seconds since Epoch
            'Y' => StatOutputType::Integer(meta.mtime()),
            // time of last status change, human-readable
            'z' => StatOutputType::Str(pretty_time(meta.ctime(), meta.ctime_nsec())),
            // time of last status change, seconds since Epoch
            'Z' => StatOutputType::Integer(meta.ctime()),

            _ => StatOutputType::Unknown,
        }
    }

    fn default_format(show_fs: bool, terse: bool, show_dev_type: bool) -> String {
        // SELinux related ct_format is *ignored*

        if show_fs {
            if terse {
                "%n %i %l %t %s %S %b %f %a %c %d\n".into()
            } else {
                "  File: \"%n\"\n    ID: %-8i Namelen: %-7l Type: %T\nBlock \
                 size: %-10s Fundamental block size: %S\nBlocks: Total: %-10b \
                 Free: %-10f Available: %a\nInodes: Total: %-10c Free: %d\n"
                    .into()
            }
        } else if terse {
            "%n %s %b %f %u %g %D %i %h %t %T %X %Y %Z %W %o\n".into()
        } else {
            [
                "  File: %n\n  Size: %-10s\tBlocks: %-10b IO Block: %-6o %F\n",
                if show_dev_type {
                    "Device: %Hd,%Ld\tInode: %-10i  Links: %-5h Device type: %Hr,%Lr\n"
                } else {
                    "Device: %Hd,%Ld\tInode: %-10i  Links: %h\n"
                },
                "Access: (%04a/%10.10A)  Uid: (%5u/%8U)   Gid: (%5g/%8G)\n",
                "Access: %x\nModify: %y\nChange: %z\n Birth: %w\n",
            ]
            .join("")
        }
    }
}

// 添加一个函数，直接使用 rustix 的 statx 获取文件信息并转换为 fs::Metadata
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
    // 设置语言（需转换为 `rust_i18n` 支持的格式，例如 "en" -> "en", "zh-CN" -> "zh-CN"）
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let matches = ct_app()
        .after_help(rust_i18n::t!(stat_options::STAT_LONG_USAGE))
        .try_get_matches_from(args)?;

    let stater = Stater::new(&matches)?;

    // Convert non-zero exit status to error
    match stater.exec() {
        0 => Ok(()),
        status => Err(status.into()),
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
            .value_name("FORMAT"),
        Arg::new(stat_options::STAT_PRINTF)
            .long(stat_options::STAT_PRINTF)
            .value_name("FORMAT")
            .help(rust_i18n::t!(stat_options::STAT_PRINTF)),
        Arg::new(stat_options::STAT_CACHED)
            .long(stat_options::STAT_CACHED)
            .value_name("MODE")
            .help("specify how to use cached attributes; useful on remote file systems")
            .value_parser(["always", "never", "default"]),
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

    #[test]
    fn test_tool_implementation() {
        let tool = Stat::default();

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
    fn test_scanners() {
        assert_eq!(Some(('a', 3)), "141zxc".scan_char(8));
        assert_eq!(Some(('\n', 2)), "12qzxc".scan_char(8)); // spell-checker:disable-line
        assert_eq!(Some(('\r', 1)), "dqzxc".scan_char(16)); // spell-checker:disable-line
        assert_eq!(None, "z2qzxc".scan_char(8)); // spell-checker:disable-line
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_group_num() {
        assert_eq!("12,379,821,234", group_num("12379821234"));
        assert_eq!("21,234", group_num("21234"));
        assert_eq!("821,234", group_num("821234"));
        assert_eq!("1,821,234", group_num("1821234"));
        assert_eq!("1,234", group_num("1234"));
        assert_eq!("234", group_num("234"));
        assert_eq!("24", group_num("24"));
        assert_eq!("4", group_num("4"));
        assert_eq!("", group_num(""));
        assert_eq!("-5", group_num("-5"));
        assert_eq!("-1,234", group_num("-1234"));
    }

    #[test]
    #[should_panic]
    fn test_group_num_panic_if_invalid_numeric_characters() {
        group_num("³³³³³");
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
                precision: Some(0),
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
            StatToken::Char('\t'),
            StatToken::Char('\r'),
            StatToken::Char('"'),
            StatToken::Char('\\'),
            StatToken::Char('\x07'),
            StatToken::Char('\x08'),
            StatToken::Char('\x1B'),
            StatToken::Char('\x0C'),
            StatToken::Char('\x0B'),
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
            StatToken::Char('\x12'),
            StatToken::Char('w'),
            StatToken::Char('Z'),
            StatToken::Char('J'),
            StatToken::Char('\n'),
        ];
        assert_eq!(&expected, &Stater::generate_tokens(s, true).unwrap());
    }
}

#[cfg(test)]
mod test_stat_all {
    use super::*;
    use clap::ArgMatches;
    use std::fs::File;
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
        // Empty format uses default format
        assert!(!tokens.is_empty());
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

        let matches = create_test_matches(vec![temp_path.to_str().unwrap()], false, None, false);
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

        let result = stater.handle_filesystem_stat(
            &OsString::from(temp_path.as_os_str()),
            temp_path.to_string_lossy().as_ref(),
        );
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

        let result = stater.handle_file_stat(
            &OsString::from(temp_file.as_os_str()),
            temp_file.to_string_lossy().as_ref(),
            false,
        );
        assert_eq!(result, 0);
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
        let output = stater.get_filesystem_output(&fs_meta, 'b');
        assert!(matches!(output, StatOutputType::Unsigned(_)));

        let output = stater.get_filesystem_output(&fs_meta, 'T');
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
        let output = stater.get_file_output(&metadata, 'n', temp_file.as_os_str(), None);
        assert!(matches!(output, StatOutputType::Str(_)));

        let output = stater.get_file_output(&metadata, 's', temp_file.as_os_str(), None);
        assert!(matches!(output, StatOutputType::Integer(_)));
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
        println!("\nOriginal help text:\n{}", helps);
        println!("\nTranslated help text:\n{}", trans);

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
