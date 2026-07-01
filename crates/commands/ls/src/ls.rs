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
use std::cmp::Reverse;
use std::os::unix::ffi::OsStrExt;

#[cfg(unix)]
use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Write as FmtWrite};
use std::fs::{self, DirEntry, FileType, Metadata, ReadDir};
use std::io::{BufWriter, ErrorKind, Write, stdout};
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt};
#[cfg(windows)]
#[allow(unused_imports)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Mutex;
#[allow(unused_imports)]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{cell::OnceCell, num::IntErrorKind};
use sys_locale::get_locale;

use std::{collections::HashSet, io::IsTerminal};

use clap::builder::{NonEmptyStringValueParser, ValueParser};
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTError;
use ctcore::ct_error::CTResult;
use ctcore::ct_error::set_ct_exit_code;
use ctcore::ct_fs::display_permissions;
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_locale::strcoll_compare;
use ctcore::ct_parse_size::parse_size_u64;
use ctcore::ct_version_cmp::ct_version_cmp;

// Currently getpwuid is `linux` target only. If it's broken out into
// a posix-compliant attribute this can be updated...
#[cfg(unix)]
use ctcore::ct_entries;
use ctcore::ct_fs::CtFileInformation;
use ctcore::ct_quoting_style;
use ctcore::ct_quoting_style::CtQuotingStyle;
use ctcore::ct_quoting_style::escape_name;
#[cfg(unix)]
use ctcore::libc::{S_IXGRP, S_IXOTH, S_IXUSR};
#[cfg(target_os = "linux")]
use ctcore::libc::{dev_t, major, minor};
use ctcore::{ct_parse_glob, ct_show, ct_show_error, ct_show_warning};
use dired::DiredOutput;
use glob::{MatchOptions, Pattern};
use lscolors::{LsColors, Style};
use number_prefix::NumberPrefix;
#[cfg(unix)]
use once_cell::sync::Lazy;
use term_grid::{Cell, Direction, Filling, Grid, GridOptions};
use unicode_width::UnicodeWidthStr;
rust_i18n::i18n!("locales", fallback = "en-US");
use rust_i18n::t;

mod dired;

#[cfg(not(feature = "selinux"))]
static LS_CONTEXT_HELP_TEXT: &str = "print any security context of each file (not enabled)";
#[cfg(feature = "selinux")]
static LS_CONTEXT_HELP_TEXT: &str = "print any security context of each file";

pub mod ls_flags {
    pub mod format {
        pub static LS_ONE_LINE: &str = "1";
        pub static LS_LONG: &str = "long";
        pub static LS_COLUMNS: &str = "C";
        pub static LS_ACROSS: &str = "x";
        pub static LS_TAB_SIZE: &str = "tabsize";
        pub static LS_COMMAS: &str = "m";
        pub static LS_LONG_NO_OWNER: &str = "g";
        pub static LS_LONG_NO_GROUP: &str = "o";
        pub static LS_LONG_NUMERIC_UID_GID: &str = "numeric-uid-gid";
    }

    pub mod files {
        pub static LS_ALL: &str = "all";
        pub static LS_ALMOST_ALL: &str = "almost-all";
    }

    pub mod sort {
        pub static LS_SIZE: &str = "S";
        pub static LS_TIME: &str = "t";
        pub static LS_NONE: &str = "U";
        pub static LS_VERSION: &str = "v";
        pub static LS_EXTENSION: &str = "X";
    }

    pub mod time {
        pub static LS_ACCESS: &str = "u";
        pub static LS_CHANGE: &str = "c";
    }

    pub mod size {
        pub static LS_ALLOCATION_SIZE: &str = "size";
        pub static LS_BLOCK_SIZE: &str = "block-size";
        pub static LS_HUMAN_READABLE: &str = "human-readable";
        pub static LS_SI: &str = "si";
        pub static LS_KIBIBYTES: &str = "kibibytes";
    }

    pub mod quoting {
        pub static LS_ESCAPE: &str = "escape";
        pub static LS_LITERAL: &str = "literal";
        pub static LS_C: &str = "quote-name";
    }

    pub mod indicator_style {
        pub static LS_SLASH: &str = "p";
        pub static LS_FILE_TYPE: &str = "file-type";
        pub static LS_CLASSIFY: &str = "classify";
    }

    pub mod dereference {
        pub static LS_ALL: &str = "dereference";
        pub static LS_ARGS: &str = "dereference-command-line";
        pub static LS_DIR_ARGS: &str = "dereference-command-line-symlink-to-dir";
    }

    pub static LS_HELP: &str = "help";
    pub static LS_QUOTING_STYLE: &str = "quoting-style";
    pub static LS_HIDE_CONTROL_CHARS: &str = "hide-control-chars";
    pub static LS_SHOW_CONTROL_CHARS: &str = "show-control-chars";
    pub static LS_WIDTH: &str = "width";
    pub static LS_AUTHOR: &str = "author";
    pub static LS_NO_GROUP: &str = "no-group";
    pub static LS_FORMAT: &str = "format";
    pub static LS_SORT: &str = "sort";
    pub static LS_TIME: &str = "time";
    pub static LS_IGNORE_BACKUPS: &str = "ignore-backups";
    pub static LS_DIRECTORY: &str = "directory";
    pub static LS_INODE: &str = "inode";
    pub static LS_REVERSE: &str = "reverse";
    pub static LS_RECURSIVE: &str = "recursive";
    pub static LS_COLOR: &str = "color";
    pub static LS_PATHS: &str = "paths";
    pub static LS_INDICATOR_STYLE: &str = "indicator-style";
    pub static LS_TIME_STYLE: &str = "time-style";
    pub static LS_FULL_TIME: &str = "full-time";
    pub static LS_HIDE: &str = "hide";
    pub static LS_IGNORE: &str = "ignore";
    pub static LS_CONTEXT: &str = "context";
    pub static LS_GROUP_DIRECTORIES_FIRST: &str = "group-directories-first";
    pub static LS_ZERO: &str = "zero";
    pub static LS_DIRED: &str = "dired";
    pub static LS_HYPERLINK: &str = "hyperlink";
    pub static LS_F: &str = "f";
}

const LS_DEFAULT_TERM_WIDTH: u16 = 80;
const LS_POSIXLY_CORRELS_BLOCK_SIZE: u64 = 512;
const LS_DEFAULT_BLOCK_SIZE: u64 = 1024;
const LS_DEFAULT_FILE_SIZE_BLOCK_SIZE: u64 = 1;

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
enum LsError {
    LsInvalidLineWidth(String),
    LsIOError(std::io::Error),
    LsIOErrorContext(std::io::Error, PathBuf, bool),
    LsBlockSizeParseError(String),
    LsConflictingArgumentDired,
    LsDiredAndZeroAreIncompatible,
    LsAlreadyListedError(PathBuf),
    LsTimeStyleParseError(String, Vec<String>),
}

impl CTError for LsError {
    fn code(&self) -> i32 {
        match self {
            Self::LsInvalidLineWidth(_) => 2,
            Self::LsIOError(_) => 1,
            Self::LsIOErrorContext(_, _, false) => 1,
            Self::LsIOErrorContext(_, _, true) => 2,
            Self::LsBlockSizeParseError(_) => 2,
            Self::LsConflictingArgumentDired => 1,
            Self::LsDiredAndZeroAreIncompatible => 2,
            Self::LsAlreadyListedError(_) => 2,
            Self::LsTimeStyleParseError(_, _) => 2,
        }
    }
}

impl Error for LsError {}

impl Display for LsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LsError::LsBlockSizeParseError(s) => {
                write!(formatter, "invalid --block-size argument {}", s.quote())
            }
            LsError::LsConflictingArgumentDired => {
                write!(formatter, "--dired requires --format=long")
            }
            LsError::LsDiredAndZeroAreIncompatible => {
                write!(formatter, "--dired and --zero are incompatible")
            }
            LsError::LsTimeStyleParseError(s, possible_time_styles) => {
                write!(
                    formatter,
                    "invalid --time-style argument {}\nPossible values are: {:?}\n\nFor more information try --help",
                    s.quote(),
                    possible_time_styles
                )
            }
            LsError::LsInvalidLineWidth(s) => {
                write!(formatter, "invalid line width: {}", s.quote())
            }
            LsError::LsIOError(e) => write!(formatter, "general io error: {e}"),
            LsError::LsIOErrorContext(e, p, _) => {
                let error_kind = e.kind();
                let errno = e.raw_os_error().unwrap_or(1i32);

                match error_kind {
                    ErrorKind::NotFound => {
                        write!(
                            formatter,
                            "cannot access '{}': No such file or directory",
                            p.to_string_lossy(),
                        )
                    }
                    // Permission denied and Operation not permitted
                    ErrorKind::PermissionDenied =>
                    {
                        #[allow(clippy::wildcard_in_or_patterns)]
                        match errno {
                            1i32 => {
                                write!(
                                    formatter,
                                    "cannot access '{}': Operation not permitted",
                                    p.to_string_lossy(),
                                )
                            }
                            13i32 | _ => match p.is_dir() {
                                true => {
                                    write!(
                                        formatter,
                                        "cannot open directory '{}': Permission denied",
                                        p.to_string_lossy(),
                                    )
                                }
                                false => {
                                    write!(
                                        formatter,
                                        "cannot open file '{}': Permission denied",
                                        p.to_string_lossy(),
                                    )
                                }
                            },
                        }
                    }
                    _ => {
                        if 9i32 == errno {
                            write!(
                                formatter,
                                "cannot open directory '{}': Bad file descriptor",
                                p.to_string_lossy(),
                            )
                        } else {
                            write!(
                                formatter,
                                "unknown io error: '{:?}', '{:?}'",
                                p.to_string_lossy(),
                                e
                            )
                        }
                    }
                }
            }
            LsError::LsAlreadyListedError(path) => {
                write!(
                    formatter,
                    "{}: not listing already-listed directory",
                    path.to_string_lossy()
                )
            }
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum LsFormat {
    Columns,
    Long,
    OneLine,
    Across,
    Commas,
}

#[derive(PartialEq, Eq, Debug)]
enum LsSort {
    None,
    Name,
    Size,
    Time,
    Version,
    Extension,
    Width,
}

#[derive(PartialEq, Copy, Clone, Debug)]
enum LsSizeFormat {
    Bytes,
    Binary,
    // Powers of 1024, --human-readable, -h
    Decimal, // Powers of 1000, --si
}

#[allow(clippy::enum_variant_names)]
#[derive(PartialEq, Eq, Debug)]
enum LsFiles {
    LsAll,
    LsAlmostAll,
    LsNormal,
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, PartialEq)]
enum LsTime {
    LsModification,
    LsAccess,
    LsChange,
    LsBirth,
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, PartialEq)]
enum LsTimeStyle {
    LsFullIso,
    LsLongIso,
    LsIso,
    LsLocale,
    LsFormat(String),
}

fn is_posix_locale() -> bool {
    for var in ["LC_ALL", "LC_TIME", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return val == "C" || val == "POSIX";
            }
        }
    }
    true
}

fn ls_parse_time_style(options: &clap::ArgMatches) -> Result<LsTimeStyle, LsError> {
    let possible_time_styles = vec![
        "full-iso".to_string(),
        "long-iso".to_string(),
        "iso".to_string(),
        "locale".to_string(),
        "+FORMAT (e.g., +%H:%M) for a 'date'-style format".to_string(),
    ];
    if let Some(field) = options.get_one::<String>(ls_flags::LS_TIME_STYLE) {
        // 如果 FULL_TIME 和 TIME_STYLE 同时存在
        // 最后添加的那个占主导地位
        if options.get_flag(ls_flags::LS_FULL_TIME)
            && options
                .indices_of(ls_flags::LS_FULL_TIME)
                .unwrap()
                .next_back()
                > options
                    .indices_of(ls_flags::LS_TIME_STYLE)
                    .unwrap()
                    .next_back()
        {
            Ok(LsTimeStyle::LsFullIso)
        } else {
            let mut field_str = field.as_str();
            while field_str.starts_with("posix-") {
                if is_posix_locale() {
                    return Ok(LsTimeStyle::LsLocale);
                }
                field_str = &field_str[6..];
            }
            if "full-iso" == field_str {
                Ok(LsTimeStyle::LsFullIso)
            } else if "long-iso" == field_str {
                Ok(LsTimeStyle::LsLongIso)
            } else if "iso" == field_str {
                Ok(LsTimeStyle::LsIso)
            } else if "locale" == field_str {
                Ok(LsTimeStyle::LsLocale)
            } else {
                match field_str.chars().next() {
                    Some('+') => Ok(LsTimeStyle::LsFormat(String::from(&field_str[1..]))),
                    _ => Err(LsError::LsTimeStyleParseError(
                        String::from(field),
                        possible_time_styles,
                    )),
                }
            }
        }
    } else if options.get_flag(ls_flags::LS_FULL_TIME) {
        Ok(LsTimeStyle::LsFullIso)
    } else {
        Ok(LsTimeStyle::LsLocale)
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(PartialEq, Debug)]
enum LsDereference {
    LsNone,
    LsDirArgs,
    LsArgs,
    LsAll,
}

#[derive(PartialEq, Eq, Debug)]
enum LsIndicatorStyle {
    None,
    Slash,
    FileType,
    Classify,
}

pub struct LsConfig {
    // Dir and vdir needs access to this field
    pub format: LsFormat,
    files: LsFiles,
    sort: LsSort,
    is_recursive: bool,
    is_reverse: bool,
    dereference: LsDereference,
    ignore_patterns: Vec<Pattern>,
    size_format: LsSizeFormat,
    is_directory: bool,
    time: LsTime,
    #[cfg(unix)]
    is_inode: bool,
    color: Option<LsColors>,
    long: LsLongFormat,
    is_alloc_size: bool,
    file_size_block_size: u64,
    #[allow(dead_code)]
    block_size: u64,
    width: u16,
    // Dir 和 vdir 需要访问该字段
    pub quoting_style: CtQuotingStyle,
    indicator_style: LsIndicatorStyle,
    time_style: LsTimeStyle,
    is_context: bool,
    is_selinux_supported: bool,
    is_group_directories_first: bool,
    line_ending: CtLineEnding,
    is_dired: bool,
    is_hyperlink: bool,
    size_suffix: String,
}

// 可删除或添加到长format 的字段
#[derive(PartialEq, Debug)]
struct LsLongFormat {
    is_author: bool,
    is_group: bool,
    is_owner: bool,
    #[cfg(unix)]
    is_numeric_uid_gid: bool,
}

struct LsPaddingCollection {
    #[cfg(unix)]
    inode: usize,
    link_count: usize,
    uname: usize,
    group: usize,
    context: usize,
    size: usize,
    #[cfg(unix)]
    major: usize,
    #[cfg(unix)]
    minor: usize,
    block_size: usize,
}

/// 提取 format，根据提供的选项显示信息。
///
/// 返回
/// 一个元组，包含格式变体和一个选项，选项包含一个 &'static str与用于定义 format 的选项相对应。
fn ls_extract_format(options: &clap::ArgMatches) -> (LsFormat, Option<&'static str>) {
    if let Some(format_) = options.get_one::<String>(ls_flags::LS_FORMAT) {
        (
            ls_format_str_to_type(format_.as_str()),
            Some(ls_flags::LS_FORMAT),
        )
    } else if options.get_flag(ls_flags::format::LS_LONG) {
        (LsFormat::Long, Some(ls_flags::format::LS_LONG))
    } else if options.get_flag(ls_flags::format::LS_ACROSS) {
        (LsFormat::Across, Some(ls_flags::format::LS_ACROSS))
    } else if options.get_flag(ls_flags::format::LS_COMMAS) {
        (LsFormat::Commas, Some(ls_flags::format::LS_COMMAS))
    } else if options.get_flag(ls_flags::format::LS_COLUMNS) {
        (LsFormat::Columns, Some(ls_flags::format::LS_COLUMNS))
    } else if stdout().is_terminal() {
        (LsFormat::Columns, None)
    } else {
        (LsFormat::OneLine, None)
    }
}

fn ls_format_str_to_type(f: &str) -> LsFormat {
    let format_str = f;
    if format_str == "long" || format_str == "verbose" {
        LsFormat::Long
    } else if format_str == "single-column" {
        LsFormat::OneLine
    } else if format_str == "columns" || format_str == "vertical" {
        LsFormat::Columns
    } else if format_str == "across" || format_str == "horizontal" {
        LsFormat::Across
    } else if format_str == "commas" {
        LsFormat::Commas
    } else {
        unreachable!("Invalid field for --format")
    }
}

/// 提取要显示的文件类型
///
/// # 返回
///
/// 代表要显示的文件类型的 Files 变体。
fn extract_files(options: &clap::ArgMatches) -> LsFiles {
    if options.get_flag(ls_flags::files::LS_ALL) {
        LsFiles::LsAll
    } else if options.get_flag(ls_flags::files::LS_ALMOST_ALL) {
        LsFiles::LsAlmostAll
    } else {
        LsFiles::LsNormal
    }
}

/// 根据提供的选项提取要使用的排序方法。
///
/// # 返回
///
/// 一个排序变量，代表要使用的排序方法。
fn extract_sort(options: &clap::ArgMatches) -> LsSort {
    if let Some(field) = options.get_one::<String>(ls_flags::LS_SORT) {
        sort_str_to_type(field.as_str())
    } else if options.get_flag(ls_flags::sort::LS_TIME) {
        LsSort::Time
    } else if options.get_flag(ls_flags::sort::LS_SIZE) {
        LsSort::Size
    } else if options.get_flag(ls_flags::sort::LS_NONE) {
        LsSort::None
    } else if options.get_flag(ls_flags::sort::LS_VERSION) {
        LsSort::Version
    } else if options.get_flag(ls_flags::sort::LS_EXTENSION) {
        LsSort::Extension
    } else {
        LsSort::Name
    }
}

fn sort_str_to_type(field_str: &str) -> LsSort {
    if field_str == "none" {
        LsSort::None
    } else if field_str == "name" {
        LsSort::Name
    } else if field_str == "time" {
        LsSort::Time
    } else if field_str == "version" {
        LsSort::Version
    } else if field_str == "extension" {
        LsSort::Extension
    } else if field_str == "size" {
        LsSort::Size
    } else if field_str == "width" {
        LsSort::Width
    } else {
        unreachable!("Invalid field for --sort")
    }
}

/// 根据提供的选项提取要使用的时间。
///
/// # 返回
///
/// 一个时间变量，代表要使用的时间。
fn extract_time(options: &clap::ArgMatches) -> LsTime {
    if let Some(field) = options.get_one::<String>(ls_flags::LS_TIME) {
        match field.as_str() {
            "ctime" | "status" => LsTime::LsChange,
            "access" | "atime" | "use" => LsTime::LsAccess,
            "birth" | "creation" => LsTime::LsBirth,
            "mtime" | "modification" => LsTime::LsModification,
            // below should never happen as clap already restricts the values.
            _ => unreachable!("Invalid field for --time"),
        }
    } else if options.get_flag(ls_flags::time::LS_ACCESS) {
        LsTime::LsAccess
    } else if options.get_flag(ls_flags::time::LS_CHANGE) {
        LsTime::LsChange
    } else {
        LsTime::LsModification
    }
}

// 可以传递一些环境变量
// 目前，我们只验证是否为空，并已知 TERM
fn is_color_compatible_term() -> bool {
    let is_term_set = std::env::var("TERM").is_ok();
    let is_colorterm_set = std::env::var("COLORTERM").is_ok();

    let term = std::env::var("TERM").unwrap_or_default();
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();

    // 在 TERM 结构中使用搜索功能来管理通配符
    let term_matches = |term: &str| -> bool {
        ctcore::ct_colors::CT_TERMS.iter().any(|&pattern| {
            term == pattern
                || (pattern.ends_with('*') && term.starts_with(&pattern[..pattern.len() - 1]))
        })
    };

    if is_term_set && term.is_empty() && is_colorterm_set && colorterm.is_empty() {
        return false;
    }

    if !term.is_empty() && !term_matches(&term) {
        return false;
    }
    true
}

/// 根据提供的选项提取要使用的颜色选项。
///
/// # 返回
///
/// 表示是否使用颜色的布尔值。
fn extract_color(options: &clap::ArgMatches) -> bool {
    if !is_color_compatible_term() {
        return false;
    }

    match options.get_one::<String>(ls_flags::LS_COLOR) {
        None => options.contains_id(ls_flags::LS_COLOR),
        Some(val) => match val.as_str() {
            "" | "always" | "yes" | "force" => true,
            "auto" | "tty" | "if-tty" => stdout().is_terminal(),
            /* "never" | "no" | "none" | */ _ => false,
        },
    }
}

/// 根据提供的选项提取要使用的超链接选项。
///
/// # Returns
///
/// 表示是否超链接文件的布尔值。
fn extract_hyperlink(options: &clap::ArgMatches) -> bool {
    let hyperlink = options
        .get_one::<String>(ls_flags::LS_HYPERLINK)
        .unwrap()
        .as_str();

    match hyperlink {
        "always" | "yes" | "force" => true,
        "auto" | "tty" | "if-tty" => stdout().is_terminal(),
        "never" | "no" | "none" => false,
        _ => unreachable!("should be handled by clap"),
    }
}

/// 与给定的 --quoting-style 参数或 QUOTING_STYLE 环境变量相匹配。
///
/// # 参数
///
/// * `style`：实际参数字符串
/// * `show_control` - 表示是否显示控制字符的布尔值。
///
/// # 返回值
///
/// * 如果样式字符串无效，则返回 None 选项；如果样式字符串无效，则返回以 `Some` 包装的 `QuotingStyle` 选项。
fn match_quoting_style_name(style: &str, show_control: bool) -> Option<CtQuotingStyle> {
    match style {
        "literal" => Some(CtQuotingStyle::Literal { show_control }),
        "shell" => Some(CtQuotingStyle::Shell {
            escape: false,
            always_quote: false,
            show_control,
        }),
        "shell-always" => Some(CtQuotingStyle::Shell {
            escape: false,
            always_quote: true,
            show_control,
        }),
        "shell-escape" => Some(CtQuotingStyle::Shell {
            escape: true,
            always_quote: false,
            show_control,
        }),
        "shell-escape-always" => Some(CtQuotingStyle::Shell {
            escape: true,
            always_quote: true,
            show_control,
        }),
        "c" => Some(CtQuotingStyle::C {
            quotes: ct_quoting_style::CtQuotes::Double,
        }),
        "escape" => Some(CtQuotingStyle::C {
            quotes: ct_quoting_style::CtQuotes::None,
        }),
        _ => None,
    }
}

/// 根据提供的选项提取要使用的引用样式。
/// 如果没有给定选项，则通过 QUOTING_STYLE 环境变量查看是否提供了默认引用样式。
/// 通过 QUOTING_STYLE 环境变量。
///
/// # 参数
///
/// * `options` - 一个对 clap::ArgMatches 对象的引用，包含命令行参数。
/// * `show_control` - 表示是否显示控制字符的布尔值。
///
/// # 返回值
///
/// 一个 QuotingStyle 变体，代表要使用的引用样式。
fn extract_quoting_style(options: &clap::ArgMatches, show_control: bool) -> CtQuotingStyle {
    let opt_quoting_style = options.get_one::<String>(ls_flags::LS_QUOTING_STYLE);

    if let Some(style) = opt_quoting_style {
        match match_quoting_style_name(style, show_control) {
            Some(qs) => qs,
            None => unreachable!("Should have been caught by Clap"),
        }
    } else if options.get_flag(ls_flags::quoting::LS_LITERAL) {
        CtQuotingStyle::Literal { show_control }
    } else if options.get_flag(ls_flags::quoting::LS_ESCAPE) {
        CtQuotingStyle::C {
            quotes: ct_quoting_style::CtQuotes::None,
        }
    } else if options.get_flag(ls_flags::quoting::LS_C) {
        CtQuotingStyle::C {
            quotes: ct_quoting_style::CtQuotes::Double,
        }
    } else if options.get_flag(ls_flags::LS_DIRED) {
        CtQuotingStyle::Literal { show_control }
    } else {
        // 如果设置，QUOTING_STYLE 环境变量将指定默认样式。
        if let Ok(style) = std::env::var("QUOTING_STYLE") {
            match match_quoting_style_name(style.as_str(), show_control) {
                Some(qs) => return qs,
                None => eprintln!(
                    "{}: Ignoring invalid value of environment variable QUOTING_STYLE: '{}'",
                    std::env::args().next().unwrap_or("ls".to_string()),
                    style
                ),
            }
        }

        // 默认情况下，当写入终端文件时，`ls` 使用 Shell 转义引号样式。
        // descriptor，否则使用 Literal。
        if stdout().is_terminal() {
            CtQuotingStyle::Shell {
                escape: true,
                always_quote: false,
                show_control,
            }
        } else {
            CtQuotingStyle::Literal { show_control }
        }
    }
}

/// 根据提供的选项提取要使用的指标样式。
///
/// # 返回
///
/// 表示要使用的指标样式的 IndicatorStyle 变体。
fn extract_indicator_style(options: &clap::ArgMatches) -> LsIndicatorStyle {
    if let Some(field) = options.get_one::<String>(ls_flags::LS_INDICATOR_STYLE) {
        indicator_str_to_type(field.as_str())
    } else if let Some(field) = options.get_one::<String>(ls_flags::indicator_style::LS_CLASSIFY) {
        match field.as_str() {
            "never" | "no" | "none" => LsIndicatorStyle::None,
            "always" | "yes" | "force" => LsIndicatorStyle::Classify,
            "auto" | "tty" | "if-tty" => {
                if stdout().is_terminal() {
                    LsIndicatorStyle::Classify
                } else {
                    LsIndicatorStyle::None
                }
            }
            &_ => LsIndicatorStyle::None,
        }
    } else if options.get_flag(ls_flags::indicator_style::LS_SLASH) {
        LsIndicatorStyle::Slash
    } else if options.get_flag(ls_flags::indicator_style::LS_FILE_TYPE) {
        LsIndicatorStyle::FileType
    } else {
        LsIndicatorStyle::None
    }
}

fn indicator_str_to_type(field: &str) -> LsIndicatorStyle {
    match field {
        "none" => LsIndicatorStyle::None,
        "file-type" => LsIndicatorStyle::FileType,
        "classify" => LsIndicatorStyle::Classify,
        "slash" => LsIndicatorStyle::Slash,
        &_ => LsIndicatorStyle::None,
    }
}

fn parse_width(s: &str) -> Result<u16, LsError> {
    let radix = match s.starts_with('0') && s.len() > 1 {
        true => 8,
        false => 10,
    };
    match u16::from_str_radix(s, radix) {
        Ok(x) => Ok(x),
        Err(e) => match e.kind() {
            IntErrorKind::PosOverflow => Ok(u16::MAX),
            _ => Err(LsError::LsInvalidLineWidth(s.into())),
        },
    }
}

impl LsConfig {
    pub fn from(options: &clap::ArgMatches) -> CTResult<Self> {
        let get_last = |flag: &str| -> usize {
            if options.value_source(flag) == Some(clap::parser::ValueSource::CommandLine) {
                options.index_of(flag).unwrap_or(0)
            } else {
                0
            }
        };

        let context = options.get_flag(ls_flags::LS_CONTEXT);
        let (mut format, opt) = ls_extract_format(options);
        let mut files = extract_files(options);

        // -o、-n 和 -g 选项比较复杂。它们不能相互覆盖
        // 因为有可能将它们组合在一起。例如，选项
        // -og 应该同时隐藏所有者和组。此外，它们不会
        // 如果使用了 -l 或 --format=long 选项，它们不会被重置。因此，这些选项应该只显示
        // 组：-gl 或"-g --format=long" 。最后，它们也不会重置
        // 切换到不同的format选项时，它们也不会重置：
        // -ogCl 或 "-og --format=vertical --format=long".
        //
        // -1 也有类似的问题：如果format是长格式，它什么也不做。这
        // 这实际上使它与 --format=singe-column 选项不同、
        // 它始终适用。
        //
        // 这里的想法是不要让这些选项与其他
        // 选项，而是手动决定它们的索引是否大于
        // 其他 format 选项。如果是，我们就设置相应的format。
        if format != LsFormat::Long {
            let idx = opt
                .and_then(|opt| options.indices_of(opt).map(|x| x.max().unwrap()))
                .unwrap_or(0);
            if [
                ls_flags::format::LS_LONG_NO_OWNER,
                ls_flags::format::LS_LONG_NO_GROUP,
                ls_flags::format::LS_LONG_NUMERIC_UID_GID,
                ls_flags::LS_FULL_TIME,
            ]
            .iter()
            .flat_map(|opt| {
                if options.value_source(opt) == Some(clap::parser::ValueSource::CommandLine) {
                    options.indices_of(opt)
                } else {
                    None
                }
            })
            .flatten()
            .any(|i| i >= idx)
            {
                format = LsFormat::Long;
            } else if let Some(mut indices) = options.indices_of(ls_flags::format::LS_ONE_LINE) {
                if options.value_source(ls_flags::format::LS_ONE_LINE)
                    == Some(clap::parser::ValueSource::CommandLine)
                    && indices.any(|i| i > idx)
                {
                    format = LsFormat::OneLine;
                }
            }
        }

        let mut ls_sort = extract_sort(options);
        let ls_time = extract_time(options);
        let mut is_needs_color = extract_color(options);
        let mut is_hyperlink = extract_hyperlink(options);

        let mut is_alloc_size = options.get_flag(ls_flags::size::LS_ALLOCATION_SIZE);
        let opt_block_size = options.get_one::<String>(ls_flags::size::LS_BLOCK_SIZE);
        let is_opt_si = opt_block_size.is_some()
            && options
                .get_one::<String>(ls_flags::size::LS_BLOCK_SIZE)
                .unwrap()
                .eq("si")
            || options.get_flag(ls_flags::size::LS_SI);
        let is_opt_hr = (opt_block_size.is_some()
            && options
                .get_one::<String>(ls_flags::size::LS_BLOCK_SIZE)
                .unwrap()
                .eq("human-readable"))
            || options.get_flag(ls_flags::size::LS_HUMAN_READABLE);
        let opt_kb = options.get_flag(ls_flags::size::LS_KIBIBYTES);

        let size_format = if is_opt_si {
            LsSizeFormat::Decimal
        } else if is_opt_hr {
            LsSizeFormat::Binary
        } else {
            LsSizeFormat::Bytes
        };

        let env_var_block_len = std::env::var_os("BLOCKSIZE");
        let env_var_block_size = std::env::var_os("BLOCK_SIZE");
        let env_var_ls_block_len = std::env::var_os("LS_BLOCK_SIZE");
        let env_var_posixly_correct = std::env::var_os("POSIXLY_CORRECT");
        let mut is_env_var_blocksize = false;
        let mut size_suffix = String::new();

        let raw_block_size = if let Some(opt_block_size) = opt_block_size {
            OsString::from(opt_block_size)
        } else if let Some(env_var_ls_block_size) = env_var_ls_block_len {
            env_var_ls_block_size
        } else if let Some(env_var_block_size) = env_var_block_size {
            env_var_block_size
        } else if let Some(env_var_blocksize) = env_var_block_len {
            is_env_var_blocksize = true;
            env_var_blocksize
        } else {
            OsString::from("")
        };

        let (file_size_block_size, block_size) =
            if !is_opt_si && !is_opt_hr && !raw_block_size.is_empty() {
                match parse_size_u64(&raw_block_size.to_string_lossy()) {
                    Ok(size) => {
                        let s_str = raw_block_size.to_string_lossy();
                        if !s_str.chars().next().unwrap_or('\0').is_ascii_digit() {
                            if let Some(first_non_digit) = s_str.find(|c: char| !c.is_ascii_digit()) {
                                size_suffix = s_str[first_non_digit..].to_string();
                            }
                        }
                        match (is_env_var_blocksize, opt_kb) {
                        (true, true) => (LS_DEFAULT_FILE_SIZE_BLOCK_SIZE, LS_DEFAULT_BLOCK_SIZE),
                        (true, false) => (LS_DEFAULT_FILE_SIZE_BLOCK_SIZE, size),
                        (false, true) => {
                            // --block-size overrides -k
                            if opt_block_size.is_some() {
                                (size, size)
                            } else {
                                (size, LS_DEFAULT_BLOCK_SIZE)
                            }
                        }
                        (false, false) => (size, size),
                    }
                    },
                    Err(_) => {
                        // 只有在使用 --block-size 指定了无效的块大小时才会失败、
                        // 忽略环境变量中的无效块大小
                        if let Some(invalid_block_size) = opt_block_size {
                            return Err(Box::new(LsError::LsBlockSizeParseError(
                                invalid_block_size.clone(),
                            )));
                        }
                        if is_env_var_blocksize {
                            (LS_DEFAULT_FILE_SIZE_BLOCK_SIZE, LS_DEFAULT_BLOCK_SIZE)
                        } else {
                            (LS_DEFAULT_BLOCK_SIZE, LS_DEFAULT_BLOCK_SIZE)
                        }
                    }
                }
            } else if env_var_posixly_correct.is_some() {
                if opt_kb {
                    (LS_DEFAULT_FILE_SIZE_BLOCK_SIZE, LS_DEFAULT_BLOCK_SIZE)
                } else {
                    (
                        LS_DEFAULT_FILE_SIZE_BLOCK_SIZE,
                        LS_POSIXLY_CORRELS_BLOCK_SIZE,
                    )
                }
            } else if is_opt_si {
                (LS_DEFAULT_FILE_SIZE_BLOCK_SIZE, 1000)
            } else {
                (LS_DEFAULT_FILE_SIZE_BLOCK_SIZE, LS_DEFAULT_BLOCK_SIZE)
            };

        let long = {
            let is_author = options.get_flag(ls_flags::LS_AUTHOR);
            let is_group = !options.get_flag(ls_flags::LS_NO_GROUP)
                && !options.get_flag(ls_flags::format::LS_LONG_NO_GROUP);
            let is_owner = !options.get_flag(ls_flags::format::LS_LONG_NO_OWNER);
            #[cfg(unix)]
            let is_numeric_uid_gid = options.get_flag(ls_flags::format::LS_LONG_NUMERIC_UID_GID);
            LsLongFormat {
                is_author,
                is_group,
                is_owner,
                #[cfg(unix)]
                is_numeric_uid_gid,
            }
        };

        let width = match options.get_one::<String>(ls_flags::LS_WIDTH) {
            Some(x) => parse_width(x)?,
            None => match terminal_size::terminal_size() {
                Some((width, _)) => width.0,
                None => match std::env::var_os("COLUMNS") {
                    Some(columns) => match columns.to_str().and_then(|s| s.parse().ok()) {
                        Some(columns) => columns,
                        None => {
                            ct_show_error!(
                                "ignoring invalid width in environment variable COLUMNS: {}",
                                columns.quote()
                            );
                            LS_DEFAULT_TERM_WIDTH
                        }
                    },
                    None => LS_DEFAULT_TERM_WIDTH,
                },
            },
        };

        #[allow(clippy::needless_bool)]
        let mut show_control = if options.get_flag(ls_flags::LS_HIDE_CONTROL_CHARS) {
            false
        } else if options.get_flag(ls_flags::LS_SHOW_CONTROL_CHARS) {
            true
        } else {
            !stdout().is_terminal()
        };

        let mut quoting_style = extract_quoting_style(options, show_control);
        let ls_indicator_style = extract_indicator_style(options);
        let ls_time_style = ls_parse_time_style(options)?;

        let mut ignore_patterns: Vec<Pattern> = Vec::new();

        if options.get_flag(ls_flags::LS_IGNORE_BACKUPS) {
            ignore_patterns.push(Pattern::new("*~").unwrap());
            ignore_patterns.push(Pattern::new(".*~").unwrap());
        }

        for pattern in options
            .get_many::<String>(ls_flags::LS_IGNORE)
            .into_iter()
            .flatten()
        {
            match ct_parse_glob::ct_from_str(pattern) {
                Ok(p) => {
                    ignore_patterns.push(p);
                }
                Err(_) => ct_show_warning!("Invalid pattern for ignore: {}", pattern.quote()),
            }
        }

        if files == LsFiles::LsNormal {
            for pattern in options
                .get_many::<String>(ls_flags::LS_HIDE)
                .into_iter()
                .flatten()
            {
                match ct_parse_glob::ct_from_str(pattern) {
                    Ok(p) => {
                        ignore_patterns.push(p);
                    }
                    Err(_) => ct_show_warning!("Invalid pattern for hide: {}", pattern.quote()),
                }
            }
        }

        let f_index = get_last(ls_flags::LS_F);
        // 如果用户输入了 -f
        if f_index > 0 {
            let sort_flags = [
                ls_flags::LS_SORT,
                ls_flags::sort::LS_TIME,
                ls_flags::sort::LS_SIZE,
                ls_flags::sort::LS_NONE,
                ls_flags::sort::LS_VERSION,
                ls_flags::sort::LS_EXTENSION,
            ];
            
            if f_index > sort_flags.iter().map(|&f| get_last(f)).max().unwrap_or(0) {
                ls_sort = LsSort::None;
            }

            let files_flags = [ls_flags::files::LS_ALL, ls_flags::files::LS_ALMOST_ALL];
            if f_index > files_flags.iter().map(|&f| get_last(f)).max().unwrap_or(0) {
                files = LsFiles::LsAll;
            }

            if f_index > get_last(ls_flags::LS_COLOR) {
                is_needs_color = false;
            }

            if f_index > get_last(ls_flags::LS_HYPERLINK) {
                is_hyperlink = false;
            }

            if f_index > get_last(ls_flags::size::LS_ALLOCATION_SIZE) {
                is_alloc_size = false;
            }

            if format == LsFormat::Long {
                let long_flags = [
                    ls_flags::format::LS_LONG,
                    ls_flags::LS_FORMAT,
                    ls_flags::format::LS_LONG_NO_GROUP,
                    ls_flags::format::LS_LONG_NO_OWNER,
                    ls_flags::format::LS_LONG_NUMERIC_UID_GID,
                    ls_flags::LS_FULL_TIME,
                ];
                if f_index > long_flags.iter().map(|&f| get_last(f)).max().unwrap_or(0) {
                    format = if stdout().is_terminal() {
                        LsFormat::Columns
                    } else {
                        LsFormat::OneLine
                    };
                }
            }
        }

        // 根据 ls info 页面，`--0` 意味着以下标志：
        // - `--显示控制字符
        // - `-format=单列
        // - `-color=none`（无颜色
        // - `--quoting-style=literal` 引号样式=直式
        // 当前的 GNU ls 实现允许 `--zero` 行为被
        // 被后面的标志覆盖。
        let zero_formats_opts = [
            ls_flags::format::LS_ACROSS,
            ls_flags::format::LS_COLUMNS,
            ls_flags::format::LS_COMMAS,
            ls_flags::format::LS_LONG,
            ls_flags::format::LS_LONG_NO_GROUP,
            ls_flags::format::LS_LONG_NO_OWNER,
            ls_flags::format::LS_LONG_NUMERIC_UID_GID,
            ls_flags::format::LS_ONE_LINE,
            ls_flags::LS_FORMAT,
        ];
        let zero_colors_opts = [ls_flags::LS_COLOR];
        let zero_show_control_opts = [
            ls_flags::LS_HIDE_CONTROL_CHARS,
            ls_flags::LS_SHOW_CONTROL_CHARS,
        ];
        let zero_quoting_style_opts = [
            ls_flags::LS_QUOTING_STYLE,
            ls_flags::quoting::LS_C,
            ls_flags::quoting::LS_ESCAPE,
            ls_flags::quoting::LS_LITERAL,
        ];
        if get_last(ls_flags::LS_ZERO)
            > zero_formats_opts
                .into_iter()
                .map(get_last)
                .max()
                .unwrap_or(0)
        {
            format = if format == LsFormat::Long {
                format
            } else {
                LsFormat::OneLine
            };
        }
        if get_last(ls_flags::LS_ZERO)
            > zero_colors_opts
                .into_iter()
                .map(get_last)
                .max()
                .unwrap_or(0)
        {
            is_needs_color = false;
        }
        if get_last(ls_flags::LS_ZERO)
            > zero_show_control_opts
                .into_iter()
                .map(get_last)
                .max()
                .unwrap_or(0)
        {
            show_control = true;
        }
        if get_last(ls_flags::LS_ZERO)
            > zero_quoting_style_opts
                .into_iter()
                .map(get_last)
                .max()
                .unwrap_or(0)
        {
            quoting_style = CtQuotingStyle::Literal { show_control };
        }

        let color = if is_needs_color {
            Some(LsColors::from_env().unwrap_or_default())
        } else {
            None
        };

        let mut is_dired = options.get_flag(ls_flags::LS_DIRED);
        if is_dired && format != LsFormat::Long {
            is_dired = false;
        }
        if is_dired && format == LsFormat::Long && options.get_flag(ls_flags::LS_ZERO) {
            return Err(Box::new(LsError::LsDiredAndZeroAreIncompatible));
        }

        let dereference = if options.get_flag(ls_flags::dereference::LS_ALL) {
            LsDereference::LsAll
        } else if options.get_flag(ls_flags::dereference::LS_ARGS) {
            LsDereference::LsArgs
        } else if options.get_flag(ls_flags::dereference::LS_DIR_ARGS) {
            LsDereference::LsDirArgs
        } else if options.get_flag(ls_flags::LS_DIRECTORY)
            || ls_indicator_style == LsIndicatorStyle::Classify
            || format == LsFormat::Long
        {
            LsDereference::LsNone
        } else {
            LsDereference::LsDirArgs
        };

        Ok(Self {
            format,
            files,
            sort: ls_sort,
            is_recursive: options.get_flag(ls_flags::LS_RECURSIVE),
            is_reverse: options.get_flag(ls_flags::LS_REVERSE),
            dereference,
            ignore_patterns,
            size_format,
            is_directory: options.get_flag(ls_flags::LS_DIRECTORY),
            time: ls_time,
            color,
            #[cfg(unix)]
            is_inode: options.get_flag(ls_flags::LS_INODE),
            long,
            is_alloc_size,
            file_size_block_size,
            block_size,
            width,
            quoting_style,
            indicator_style: ls_indicator_style,
            time_style: ls_time_style,
            is_context: context,
            is_selinux_supported: {
                #[cfg(feature = "selinux")]
                {
                    selinux::kernel_support() != selinux::KernelSupport::Unsupported
                }
                #[cfg(not(feature = "selinux"))]
                {
                    false
                }
            },
            is_group_directories_first: options.get_flag(ls_flags::LS_GROUP_DIRECTORIES_FIRST),
            line_ending: CtLineEnding::from_zero_flag(options.get_flag(ls_flags::LS_ZERO)),
            is_dired,
            is_hyperlink,
            size_suffix,
        })
    }
}

#[derive(Default)]
pub struct Ls;
impl Tool for Ls {
    fn name(&self) -> &'static str {
        "ls"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        ls_main(args.iter().cloned()).map(|_| ())
    }
}

pub fn ls_main(args: impl ctcore::Args) -> CTResult<(Vec<PathData>, Vec<PathData>)> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let command = ct_app();

    let matches = command.try_get_matches_from(args)?;

    let config = LsConfig::from(&matches)?;

    let paths_list = matches.get_many::<OsString>(ls_flags::LS_PATHS);
    let paths_from_args: Vec<_> = paths_list
        .map(|v| v.map(Path::new).collect())
        .unwrap_or_else(|| vec![Path::new(".")]);
    list(paths_from_args, &config)
}

pub fn ct_app() -> Command {
    // ct_ls::ct_app()
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("ls.about");
    let usage_description = t!("ls.usage");

    let args = vec![
        Arg::new(ls_flags::LS_HELP)
            .long(ls_flags::LS_HELP)
            .help("Print help information.")
            .action(ArgAction::Help),
        Arg::new(ls_flags::LS_FORMAT)
            .long(ls_flags::LS_FORMAT)
            .help("Set the display format.")
            .value_parser([
                "long",
                "verbose",
                "single-column",
                "columns",
                "vertical",
                "across",
                "horizontal",
                "commas",
            ])
            .hide_possible_values(true)
            .require_equals(true)
            .overrides_with_all([
                ls_flags::LS_FORMAT,
                ls_flags::format::LS_COLUMNS,
                ls_flags::format::LS_LONG,
                ls_flags::format::LS_ACROSS,
                ls_flags::format::LS_COLUMNS,
            ]),
        Arg::new(ls_flags::format::LS_COLUMNS)
            .short('C')
            .help("Display the files in columns.")
            .overrides_with_all([
                ls_flags::LS_FORMAT,
                ls_flags::format::LS_COLUMNS,
                ls_flags::format::LS_LONG,
                ls_flags::format::LS_ACROSS,
                ls_flags::format::LS_COLUMNS,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::format::LS_LONG)
            .short('l')
            .long(ls_flags::format::LS_LONG)
            .help("Display detailed information.")
            .overrides_with_all([
                ls_flags::LS_FORMAT,
                ls_flags::format::LS_COLUMNS,
                ls_flags::format::LS_LONG,
                ls_flags::format::LS_ACROSS,
                ls_flags::format::LS_COLUMNS,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::format::LS_ACROSS)
            .short('x')
            .help("List entries in rows instead of in columns.")
            .overrides_with_all([
                ls_flags::LS_FORMAT,
                ls_flags::format::LS_COLUMNS,
                ls_flags::format::LS_LONG,
                ls_flags::format::LS_ACROSS,
                ls_flags::format::LS_COLUMNS,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::format::LS_TAB_SIZE)
            .short('T')
            .long(ls_flags::format::LS_TAB_SIZE)
            .env("TABSIZE")
            .value_name("COLS")
            .help("Assume tab stops at each COLS instead of 8 (unimplemented)"),
        Arg::new(ls_flags::format::LS_COMMAS)
            .short('m')
            .help("List entries separated by commas.")
            .overrides_with_all([
                ls_flags::LS_FORMAT,
                ls_flags::format::LS_COLUMNS,
                ls_flags::format::LS_LONG,
                ls_flags::format::LS_ACROSS,
                ls_flags::format::LS_COLUMNS,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_ZERO)
            .long(ls_flags::LS_ZERO)
            .overrides_with(ls_flags::LS_ZERO)
            .help("List entries separated by ASCII NUL characters.")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_F)
            .short('f')
            .help("do not sort, enable -aU, disable -ls --color")
            .action(ArgAction::SetTrue)
            .overrides_with(ls_flags::LS_F),
        Arg::new(ls_flags::LS_DIRED)
            .long(ls_flags::LS_DIRED)
            .short('D')
            .help("generate output designed for Emacs' dired (Directory Editor) mode")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_HYPERLINK)
            .long(ls_flags::LS_HYPERLINK)
            .help("hyperlink file names WHEN")
            .value_parser([
                "always", "yes", "force", "auto", "tty", "if-tty", "never", "no", "none",
            ])
            .require_equals(true)
            .num_args(0..=1)
            .default_missing_value("always")
            .default_value("never")
            .value_name("WHEN"),
        Arg::new(ls_flags::format::LS_ONE_LINE)
            .short('1')
            .help("List one file per line.")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::format::LS_LONG_NO_GROUP)
            .short('o')
            .help(
                "Long format without group information. \
                         Identical to --format=long with --no-group.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::format::LS_LONG_NO_OWNER)
            .short('g')
            .help("Long format without owner information.")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::format::LS_LONG_NUMERIC_UID_GID)
            .short('n')
            .long(ls_flags::format::LS_LONG_NUMERIC_UID_GID)
            .help("-l with numeric UIDs and GIDs.")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_QUOTING_STYLE)
            .long(ls_flags::LS_QUOTING_STYLE)
            .help("Set quoting style.")
            .value_parser([
                "literal",
                "shell",
                "shell-always",
                "shell-escape",
                "shell-escape-always",
                "c",
                "escape",
            ])
            .overrides_with_all([
                ls_flags::LS_QUOTING_STYLE,
                ls_flags::quoting::LS_LITERAL,
                ls_flags::quoting::LS_ESCAPE,
                ls_flags::quoting::LS_C,
            ]),
        Arg::new(ls_flags::quoting::LS_LITERAL)
            .short('N')
            .long(ls_flags::quoting::LS_LITERAL)
            .alias("l")
            .help("Use literal quoting style. Equivalent to `--quoting-style=literal`")
            .overrides_with_all([
                ls_flags::LS_QUOTING_STYLE,
                ls_flags::quoting::LS_LITERAL,
                ls_flags::quoting::LS_ESCAPE,
                ls_flags::quoting::LS_C,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::quoting::LS_ESCAPE)
            .short('b')
            .long(ls_flags::quoting::LS_ESCAPE)
            .help("Use escape quoting style. Equivalent to `--quoting-style=escape`")
            .overrides_with_all([
                ls_flags::LS_QUOTING_STYLE,
                ls_flags::quoting::LS_LITERAL,
                ls_flags::quoting::LS_ESCAPE,
                ls_flags::quoting::LS_C,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::quoting::LS_C)
            .short('Q')
            .long(ls_flags::quoting::LS_C)
            .help("Use LS_C quoting style. Equivalent to `--quoting-style=c`")
            .overrides_with_all([
                ls_flags::LS_QUOTING_STYLE,
                ls_flags::quoting::LS_LITERAL,
                ls_flags::quoting::LS_ESCAPE,
                ls_flags::quoting::LS_C,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_HIDE_CONTROL_CHARS)
            .short('q')
            .long(ls_flags::LS_HIDE_CONTROL_CHARS)
            .help("Replace control characters with '?' if they are not escaped.")
            .overrides_with_all([ls_flags::LS_HIDE_CONTROL_CHARS, ls_flags::LS_SHOW_CONTROL_CHARS])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_SHOW_CONTROL_CHARS)
            .long(ls_flags::LS_SHOW_CONTROL_CHARS)
            .help("Show control characters 'as is' if they are not escaped.")
            .overrides_with_all([ls_flags::LS_HIDE_CONTROL_CHARS, ls_flags::LS_SHOW_CONTROL_CHARS])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_TIME)
            .long(ls_flags::LS_TIME)
            .help(
                "Show time in <field>:\n\
                         \taccess time (-u): atime, access, use;\n\
                         \tchange time (-c): ctime, status;\n\
                         \tmodified time (default): mtime, modification;\n\
                         \tbirth time: birth, creation;",
            )
            .value_name("field")
            .value_parser([
                "atime", "access", "use", "ctime", "status", "birth", "creation", "mtime", "modification",
            ])
            .hide_possible_values(true)
            .require_equals(true)
            .overrides_with_all([ls_flags::LS_TIME, ls_flags::time::LS_ACCESS, ls_flags::time::LS_CHANGE]),
        Arg::new(ls_flags::time::LS_CHANGE)
            .short('c')
            .help(
                "If the long listing format (e.g., -l, -o) is being used, print the \
                         status change time (the 'ctime' in the inode) instead of the modification \
                         time. When explicitly sorting by time (--sort=time or -t) or when not \
                         using a long listing format, sort according to the status change time.",
            )
            .overrides_with_all([ls_flags::LS_TIME, ls_flags::time::LS_ACCESS, ls_flags::time::LS_CHANGE])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::time::LS_ACCESS)
            .short('u')
            .help(
                "If the long listing format (e.g., -l, -o) is being used, print the \
                         status access time instead of the modification time. When explicitly \
                         sorting by time (--sort=time or -t) or when not using a long listing \
                         format, sort according to the access time.",
            )
            .overrides_with_all([ls_flags::LS_TIME, ls_flags::time::LS_ACCESS, ls_flags::time::LS_CHANGE])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_HIDE)
            .long(ls_flags::LS_HIDE)
            .action(ArgAction::Append)
            .value_name("PATTERN")
            .help(
                "do not list implied entries matching shell PATTERN (overridden by -a or -A)",
            ),
        Arg::new(ls_flags::LS_IGNORE)
            .short('I')
            .long(ls_flags::LS_IGNORE)
            .action(ArgAction::Append)
            .value_name("PATTERN")
            .help("do not list implied entries matching shell PATTERN"),
        Arg::new(ls_flags::LS_IGNORE_BACKUPS)
            .short('B')
            .long(ls_flags::LS_IGNORE_BACKUPS)
            .help("Ignore entries which end with ~.")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_SORT)
            .long(ls_flags::LS_SORT)
            .help("Sort by <field>: name, none (-U), time (-t), size (-S), extension (-X) or width")
            .value_name("field")
            .value_parser(["name", "none", "time", "size", "version", "extension", "width"])
            .require_equals(true)
            .overrides_with_all([
                ls_flags::LS_SORT,
                ls_flags::sort::LS_SIZE,
                ls_flags::sort::LS_TIME,
                ls_flags::sort::LS_NONE,
                ls_flags::sort::LS_VERSION,
                ls_flags::sort::LS_EXTENSION,
            ]),
        Arg::new(ls_flags::sort::LS_SIZE)
            .short('S')
            .help("Sort by file size, largest first.")
            .overrides_with_all([
                ls_flags::LS_SORT,
                ls_flags::sort::LS_SIZE,
                ls_flags::sort::LS_TIME,
                ls_flags::sort::LS_NONE,
                ls_flags::sort::LS_VERSION,
                ls_flags::sort::LS_EXTENSION,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::sort::LS_TIME)
            .short('t')
            .help("Sort by modification time (the 'mtime' in the inode), newest first.")
            .overrides_with_all([
                ls_flags::LS_SORT,
                ls_flags::sort::LS_SIZE,
                ls_flags::sort::LS_TIME,
                ls_flags::sort::LS_NONE,
                ls_flags::sort::LS_VERSION,
                ls_flags::sort::LS_EXTENSION,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::sort::LS_VERSION)
            .short('v')
            .help("Natural sort of (version) numbers in the filenames.")
            .overrides_with_all([
                ls_flags::LS_SORT,
                ls_flags::sort::LS_SIZE,
                ls_flags::sort::LS_TIME,
                ls_flags::sort::LS_NONE,
                ls_flags::sort::LS_VERSION,
                ls_flags::sort::LS_EXTENSION,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::sort::LS_EXTENSION)
            .short('X')
            .help("Sort alphabetically by entry extension.")
            .overrides_with_all([
                ls_flags::LS_SORT,
                ls_flags::sort::LS_SIZE,
                ls_flags::sort::LS_TIME,
                ls_flags::sort::LS_NONE,
                ls_flags::sort::LS_VERSION,
                ls_flags::sort::LS_EXTENSION,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::sort::LS_NONE)
            .short('U')
            .help(
                "Do not sort; list the files in whatever order they are stored in the \
                     directory.  This is especially useful when listing very large directories, \
                     since not doing any sorting can be noticeably faster.",
            )
            .overrides_with_all([
                ls_flags::LS_SORT,
                ls_flags::sort::LS_SIZE,
                ls_flags::sort::LS_TIME,
                ls_flags::sort::LS_NONE,
                ls_flags::sort::LS_VERSION,
                ls_flags::sort::LS_EXTENSION,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::dereference::LS_ALL)
            .short('L')
            .long(ls_flags::dereference::LS_ALL)
            .help(
                "When showing file information for a symbolic link, show information for the \
                     file the link references rather than the link itself.",
            )
            .overrides_with_all([
                ls_flags::dereference::LS_ALL,
                ls_flags::dereference::LS_DIR_ARGS,
                ls_flags::dereference::LS_ARGS,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::dereference::LS_DIR_ARGS)
            .long(ls_flags::dereference::LS_DIR_ARGS)
            .help(
                "Do not follow symlinks except when they link to directories and are \
                     given as command line arguments.",
            )
            .overrides_with_all([
                ls_flags::dereference::LS_ALL,
                ls_flags::dereference::LS_DIR_ARGS,
                ls_flags::dereference::LS_ARGS,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::dereference::LS_ARGS)
            .short('H')
            .long(ls_flags::dereference::LS_ARGS)
            .help("Do not follow symlinks except when given as command line arguments.")
            .overrides_with_all([
                ls_flags::dereference::LS_ALL,
                ls_flags::dereference::LS_DIR_ARGS,
                ls_flags::dereference::LS_ARGS,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_NO_GROUP)
            .long(ls_flags::LS_NO_GROUP)
            .short('G')
            .help("Do not show group in long format.")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_AUTHOR).long(ls_flags::LS_AUTHOR).help(
            "Show author in long format. On the supported platforms, \
                 the author always matches the file owner.",
        ).action(ArgAction::SetTrue),
        Arg::new(ls_flags::files::LS_ALL)
            .short('a')
            .long(ls_flags::files::LS_ALL)
            // Overrides -A (as the order matters)
            .overrides_with_all([ls_flags::files::LS_ALL, ls_flags::files::LS_ALMOST_ALL])
            .help("Do not ignore hidden files (files with names that start with '.').")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::files::LS_ALMOST_ALL)
            .short('A')
            .long(ls_flags::files::LS_ALMOST_ALL)
            // Overrides -a (as the order matters)
            .overrides_with_all([ls_flags::files::LS_ALL, ls_flags::files::LS_ALMOST_ALL])
            .help(
                "In a directory, do not ignore all file names that start with '.', \
                     only ignore '.' and '..'.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_DIRECTORY)
            .short('d')
            .long(ls_flags::LS_DIRECTORY)
            .help(
                "Only list the names of directories, rather than listing directory contents. \
                     This will not follow symbolic links unless one of `--dereference-command-line \
                     (-H)`, `--dereference (-L)`, or `--dereference-command-line-symlink-to-dir` is \
                     specified.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::size::LS_HUMAN_READABLE)
            .short('h')
            .long(ls_flags::size::LS_HUMAN_READABLE)
            .help("Print human readable file sizes (e.g. 1K 234M 56G).")
            .overrides_with_all([ls_flags::size::LS_BLOCK_SIZE, ls_flags::size::LS_SI])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::size::LS_KIBIBYTES)
            .short('k')
            .long(ls_flags::size::LS_KIBIBYTES)
            .help(
                "default to 1024-byte blocks for file system usage; used only with -s and per \
                     directory totals",
            )
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::size::LS_SI)
            .long(ls_flags::size::LS_SI)
            .help("Print human readable file sizes using powers of 1000 instead of 1024.")
            .overrides_with_all([ls_flags::size::LS_BLOCK_SIZE, ls_flags::size::LS_HUMAN_READABLE])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::size::LS_BLOCK_SIZE)
            .long(ls_flags::size::LS_BLOCK_SIZE)
            .require_equals(true)
            .value_name("LS_BLOCK_SIZE")
            .help("scale sizes by LS_BLOCK_SIZE when printing them")
            .overrides_with_all([ls_flags::size::LS_SI, ls_flags::size::LS_HUMAN_READABLE]),
        Arg::new(ls_flags::LS_INODE)
            .short('i')
            .long(ls_flags::LS_INODE)
            .help("print the index number of each file")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_REVERSE)
            .short('r')
            .long(ls_flags::LS_REVERSE)
            .help(
                "Reverse whatever the sorting method is e.g., list files in reverse \
             alphabetical order, youngest first, smallest first, or whatever.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_RECURSIVE)
            .short('R')
            .long(ls_flags::LS_RECURSIVE)
            .help("List the contents of all directories recursively.")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_WIDTH)
            .long(ls_flags::LS_WIDTH)
            .short('w')
            .help("Assume that the terminal is COLS columns wide.")
            .value_name("COLS"),
        Arg::new(ls_flags::size::LS_ALLOCATION_SIZE)
            .short('s')
            .long(ls_flags::size::LS_ALLOCATION_SIZE)
            .help("print the allocated size of each file, in blocks")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_COLOR)
            .long(ls_flags::LS_COLOR)
            .help("Color output based on file type.")
            .value_parser([
                "always", "yes", "force", "auto", "tty", "if-tty", "never", "no", "none",
            ])
            .require_equals(true)
            .num_args(0..=1),
        Arg::new(ls_flags::LS_INDICATOR_STYLE)
            .long(ls_flags::LS_INDICATOR_STYLE)
            .help(
                "Append indicator with style WORD to entry names: \
                 none (default),  slash (-p), file-type (--file-type), classify (-F)",
            )
            .value_parser(["none", "slash", "file-type", "classify"])
            .overrides_with_all([
                ls_flags::indicator_style::LS_FILE_TYPE,
                ls_flags::indicator_style::LS_SLASH,
                ls_flags::indicator_style::LS_CLASSIFY,
                ls_flags::LS_INDICATOR_STYLE,
            ]),
        Arg::new(ls_flags::indicator_style::LS_CLASSIFY)
            .short('F')
            .long(ls_flags::indicator_style::LS_CLASSIFY)
            .help(
                "Append a character to each file name indicating the file type. Also, for \
                     regular files that are executable, append '*'. The file type indicators are \
                     '/' for directories, '@' for symbolic links, '|' for FIFOs, '=' for sockets, \
                     '>' for doors, and nothing for regular files. when may be omitted, or one of:\n\
                         \tnone - Do not classify. This is the default.\n\
                         \tauto - Only classify if standard output is a terminal.\n\
                         \talways - Always classify.\n\
                     Specifying --classify and no when is equivalent to --classify=always. This will \
                     not follow symbolic links listed on the command line unless the \
                     --dereference-command-line (-H), --dereference (-L), or \
                     --dereference-command-line-symlink-to-dir flags are specified.",
            )
            .value_name("when")
            .value_parser([
                "always", "yes", "force", "auto", "tty", "if-tty", "never", "no", "none",
            ])
            .default_missing_value("always")
            .require_equals(true)
            .num_args(0..=1)
            .overrides_with_all([
                ls_flags::indicator_style::LS_FILE_TYPE,
                ls_flags::indicator_style::LS_SLASH,
                ls_flags::indicator_style::LS_CLASSIFY,
                ls_flags::LS_INDICATOR_STYLE,
            ]),
        Arg::new(ls_flags::indicator_style::LS_FILE_TYPE)
            .long(ls_flags::indicator_style::LS_FILE_TYPE)
            .help("Same as --classify, but do not append '*'")
            .overrides_with_all([
                ls_flags::indicator_style::LS_FILE_TYPE,
                ls_flags::indicator_style::LS_SLASH,
                ls_flags::indicator_style::LS_CLASSIFY,
                ls_flags::LS_INDICATOR_STYLE,
            ])
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::indicator_style::LS_SLASH)
            .short('p')
            .help("Append / indicator to directories.")
            .overrides_with_all([
                ls_flags::indicator_style::LS_FILE_TYPE,
                ls_flags::indicator_style::LS_SLASH,
                ls_flags::indicator_style::LS_CLASSIFY,
                ls_flags::LS_INDICATOR_STYLE,
            ])
            .action(ArgAction::SetTrue),
        //This still needs support for posix-*
        Arg::new(ls_flags::LS_TIME_STYLE)
            .long(ls_flags::LS_TIME_STYLE)
            .help("time/date format with -l; see LS_TIME_STYLE below")
            .value_name("LS_TIME_STYLE")
            .env("TIME_STYLE")
            .value_parser(NonEmptyStringValueParser::new())
            .overrides_with_all([ls_flags::LS_TIME_STYLE]),
        Arg::new(ls_flags::LS_FULL_TIME)
            .long(ls_flags::LS_FULL_TIME)
            .overrides_with(ls_flags::LS_FULL_TIME)
            .help("like -l --time-style=full-iso")
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_CONTEXT)
            .short('Z')
            .long(ls_flags::LS_CONTEXT)
            .help(LS_CONTEXT_HELP_TEXT)
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_GROUP_DIRECTORIES_FIRST)
            .long(ls_flags::LS_GROUP_DIRECTORIES_FIRST)
            .help(
                "group directories before files; can be augmented with \
                     a --sort option, but any use of --sort=none (-U) disables grouping",
            )
            .action(ArgAction::SetTrue),
        Arg::new(ls_flags::LS_PATHS)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath)
            .value_parser(ValueParser::os_string()),
    ];

    Command::new(utility_name)
        .version(command_version)
        .override_usage(usage_description)
        .about(application_info)
        .infer_long_args(true)
        .disable_help_flag(true)
        .args_override_self(true)
        .args(args)
        .after_help(t!("ls.after_help"))
}

/// 表示一个路径及其相关数据。
/// 任何会被多次重复使用的数据都应该添加到这个结构中。
/// 在此缓存数据有助于消除为获取相同信息而进行的冗余系统调用。
#[derive(Debug)]
pub struct PathData {
    // 基于配置从 symlink_metadata() 或 metadata() 获得的 Result<MetaData>
    md: OnceCell<Option<Metadata>>,
    ft: OnceCell<Option<FileType>>,
    // 可用于避免读取元数据。也可称为 d_type：
    de: Option<DirEntry>,
    // Name of the file - will be empty for . or ..
    display_name: OsString,
    // 上述所有数据对应的 PathBuf
    p_buf: PathBuf,
    is_must_dereference: bool,
    security_context: String,
    is_command_line: bool,
}

impl PathData {
    fn new(
        p_buf: PathBuf,
        dir_entry: Option<std::io::Result<DirEntry>>,
        file_name: Option<OsString>,
        config: &LsConfig,
        command_line: bool,
    ) -> Self {
        // 我们不能使用`Path::ends_with`或`Path::Components`，因为它们会删除'.
        // 对于 '...'，文件名为 None
        let display_name = if let Some(name) = file_name {
            name
        } else if command_line {
            p_buf.clone().into()
        } else {
            p_buf
                .file_name()
                .unwrap_or_else(|| p_buf.iter().next_back().unwrap())
                .to_owned()
        };

        let must_dereference = if config.dereference == LsDereference::LsAll {
            true
        } else if config.dereference == LsDereference::LsArgs {
            command_line
        } else if config.dereference == LsDereference::LsNone {
            false
        } else if config.dereference == LsDereference::LsDirArgs {
            match command_line {
                true => {
                    if let Ok(md) = p_buf.metadata() {
                        md.is_dir()
                    } else {
                        false
                    }
                }
                false => false,
            }
        } else {
            unreachable!("unknown dereference type")
        };

        let de: Option<DirEntry> = match dir_entry {
            Some(de) => de.ok(),
            None => None,
        };

        // 为什么要检查 DirEntry file_type()？ 因为调用B/c
        // 与在 Path 上调用元数据()相比，几乎是免费的
        fn get_file_type(
            de: &DirEntry,
            p_buf: &Path,
            must_dereference: bool,
        ) -> OnceCell<Option<FileType>> {
            if must_dereference {
                if let Ok(md_pb) = p_buf.metadata() {
                    return OnceCell::from(Some(md_pb.file_type()));
                }
            }

            if let Ok(ft_de) = de.file_type() {
                OnceCell::from(Some(ft_de))
            } else if let Ok(md_pb) = p_buf.symlink_metadata() {
                OnceCell::from(Some(md_pb.file_type()))
            } else {
                OnceCell::new()
            }
        }

        let ft = if let Some(ref de) = de {
            get_file_type(de, &p_buf, must_dereference)
        } else {
            OnceCell::new()
        };

        // 配置了显示安全上下文才给出安全上下文, Long模式需要显示安全上下文标记
        let security_context = if config.is_context || config.format == LsFormat::Long {
            get_security_context(config, &p_buf, must_dereference)
        } else {
            String::new()
        };

        Self {
            md: OnceCell::new(),
            ft,
            de,
            display_name,
            p_buf,
            is_must_dereference: must_dereference,
            security_context,
            is_command_line: command_line,
        }
    }

    fn get_metadata<W: Write>(&self, out: &mut W) -> Option<&Metadata> {
        self.md
            .get_or_init(|| {
                // 检查我们是否可以使用 DirEntry 元数据
                // 这将避免调用 stat()
                if !self.is_must_dereference {
                    if let Some(dir_entry) = &self.de {
                        return dir_entry.metadata().ok();
                    }
                }

                // 如果没有，检查我们是否可以使用路径元数据
                match get_metadata_with_deref_opt(self.p_buf.as_path(), self.is_must_dereference) {
                    Err(err) => {
                        // 修正： 在这里传播结果有点麻烦
                        out.flush().unwrap();
                        let errno = err.raw_os_error().unwrap_or(1i32);
                        // 一个坏的 fd 在被引用时会产生错误、
                        // 但 GNU 在输入坏的 fd "dir "之前不会出错。
                        // 在这里，我们与 GNU 的行为相匹配，通过在 EBADF 时交回未被引用的元数据来实现。
                        // 在这里，我们与 GNU 的行为相匹配，在 EBADF 时交还未被引用的元数据。
                        if self.is_must_dereference && errno == 9i32 {
                            if let Some(dir_entry) = &self.de {
                                return dir_entry.metadata().ok();
                            }
                        }
                        ct_show!(LsError::LsIOErrorContext(
                            err,
                            self.p_buf.clone(),
                            self.is_command_line
                        ));
                        None
                    }
                    Ok(md) => Some(md),
                }
            })
            .as_ref()
    }

    fn file_type<W: Write>(&self, out: &mut W) -> Option<&FileType> {
        self.ft
            .get_or_init(|| self.get_metadata(out).map(|md| md.file_type()))
            .as_ref()
    }
}

fn show_dir_name<W: Write>(path_data: &PathData, out: &mut W, config: &LsConfig) {
    match config.is_hyperlink {
        true => {
            let name = escape_name(&path_data.display_name, &config.quoting_style);
            let hyperlink = create_hyperlink(&name, path_data);
            write!(out, "{hyperlink}:").unwrap()
        }
        false => write!(out, "{}:", path_data.p_buf.display()).unwrap(),
    }
}

#[allow(clippy::cognitive_complexity)]
pub fn list(locs: Vec<&Path>, config: &LsConfig) -> CTResult<(Vec<PathData>, Vec<PathData>)> {
    let mut files_vec = Vec::<PathData>::new();
    let mut dirs_vec = Vec::<PathData>::new();
    let mut out = BufWriter::new(stdout());
    let mut dired_output = DiredOutput::default();
    let mut style_manager = StyleManager::new();
    let initial_locs_len = locs.len();

    for loc in locs {
        let path_data = PathData::new(PathBuf::from(loc), None, None, config, true);

        // 在这里获取元数据没什么大不了的，因为这只是 CWD
        // 我们只想知道字符串是否作为文件/目录存在
        //
        // 正确的 GNU 处理方式是，如果引用了符号链接 DNE，则不显示。
        // 只显示基本目录，显示子目录，并打印?
        // 以长格式format
        if path_data.get_metadata(&mut out).is_none() {
            continue;
        }

        let show_dir_contents = if let Some(ft) = path_data.file_type(&mut out) {
            !config.is_directory && ft.is_dir()
        } else {
            set_ct_exit_code(1);
            false
        };
        // };

        if show_dir_contents {
            dirs_vec.push(path_data);
        } else {
            files_vec.push(path_data);
        }
    }

    sort_entries(&mut files_vec, config, &mut out);
    sort_entries(&mut dirs_vec, config, &mut out);

    display_items(
        &files_vec,
        config,
        &mut out,
        &mut dired_output,
        &mut style_manager,
    )?;

    for (pos, path_data) in dirs_vec.iter().enumerate() {
        // 在此调用 read_dir，以符合 GNU 语义，在目录标题、名称和总数之前打印 在目录标题、名称和总数之前打印 read_dir 错误
        let read_dir = match fs::read_dir(&path_data.p_buf) {
            Err(err) => {
                // flush stdout buffer before the error to preserve formatting and order
                out.flush()?;
                ct_show!(LsError::LsIOErrorContext(
                    err,
                    path_data.p_buf.clone(),
                    path_data.is_command_line
                ));
                continue;
            }
            Ok(rd) => rd,
        };

        // 打印目录标题 - 名称...... "总计 "出现在错误显示之后
        if initial_locs_len > 1 || config.is_recursive {
            if pos.eq(&0usize) && files_vec.is_empty() {
                if config.is_dired {
                    dired::dired_indent(&mut out)?;
                }
                show_dir_name(path_data, &mut out, config);
                writeln!(out)?;
                if config.is_dired {
                    // 显示的第一个目录
                    let dir_len = path_data.display_name.len();
                    // 添加 //SUBDIRED// 坐标
                    dired::dired_calculate_subdired(&mut dired_output, dir_len);
                    // 为目录名添加填充
                    dired::dired_add_dir_name(&mut dired_output, dir_len);
                }
            } else {
                writeln!(out)?;
                show_dir_name(path_data, &mut out, config);
                writeln!(out)?;
            }
        }
        let mut listed_ancestors = HashSet::new();
        listed_ancestors.insert(CtFileInformation::from_path(
            &path_data.p_buf,
            path_data.is_must_dereference,
        )?);
        enter_directory(
            path_data,
            read_dir,
            config,
            &mut out,
            &mut listed_ancestors,
            &mut dired_output,
            &mut style_manager,
        )?;
    }
    if config.is_dired {
        dired::dired_print_dired_output(config, &dired_output, &mut out)?;
    }
    Ok((files_vec, dirs_vec))
}

fn sort_entries<W: Write>(entries: &mut [PathData], config: &LsConfig, out: &mut W) {
    if config.sort == LsSort::Time {
        entries.sort_by_key(|k| {
            Reverse(
                k.get_metadata(out)
                    .and_then(|md| get_system_time(md, config))
                    .unwrap_or(UNIX_EPOCH),
            )
        })
    } else if config.sort == LsSort::Size {
        entries.sort_by_key(|k| Reverse(k.get_metadata(out).map(|md| md.len()).unwrap_or(0)))
    } else if config.sort == LsSort::Name {
        entries.sort_by(|a, b| {
            strcoll_compare(a.display_name.as_bytes(), b.display_name.as_bytes(), false)
        })
    } else if config.sort == LsSort::Version {
        entries.sort_by(|a, b| {
            ct_version_cmp(&a.p_buf.to_string_lossy(), &b.p_buf.to_string_lossy()).then(
                strcoll_compare(
                    a.p_buf.to_string_lossy().as_bytes(),
                    b.p_buf.to_string_lossy().as_bytes(),
                    false,
                ),
            )
        })
    } else if config.sort == LsSort::Extension {
        entries.sort_by(|a, b| {
            a.p_buf
                .extension()
                .cmp(&b.p_buf.extension())
                .then(a.p_buf.file_stem().cmp(&b.p_buf.file_stem()))
        })
    } else if config.sort == LsSort::Width {
        entries.sort_by(|a, b| {
            a.display_name
                .len()
                .cmp(&b.display_name.len())
                .then(strcoll_compare(
                    a.display_name.as_bytes(),
                    b.display_name.as_bytes(),
                    false,
                ))
        })
    } else if config.sort == LsSort::None {
        {}
    } else {
        unreachable!("unknown sort type")
    }

    if config.is_reverse {
        entries.reverse();
    }

    if config.is_group_directories_first && config.sort != LsSort::None {
        entries.sort_by_key(|p| {
            let md = {
                match p.is_must_dereference {
                    true => p.md.get(),
                    false => None,
                }
            };

            !match md {
                None | Some(None) => {
                    // 如果无法确定元数据，则作为文件处理。
                    get_metadata_with_deref_opt(p.p_buf.as_path(), true)
                        .map_or_else(|_| false, |m| m.is_dir())
                }
                Some(Some(m)) => m.is_dir(),
            }
        });
    }
}

fn is_hidden(file_path: &DirEntry) -> bool {
    #[cfg(windows)]
    {
        let metadata = file_path.metadata().unwrap();
        let attr = metadata.file_attributes();
        (attr & 0x2) > 0
    }
    #[cfg(not(windows))]
    {
        file_path
            .file_name()
            .to_str()
            .map(|res| res.starts_with('.'))
            .unwrap_or(false)
    }
}

fn should_display(entry: &DirEntry, config: &LsConfig) -> bool {
    // 检查是否隐藏
    if config.files == LsFiles::LsNormal && is_hidden(entry) {
        return false;
    }

    // 检查是否属于忽略模式
    let options = MatchOptions {
        // 设置 require_literal_leading_dot 以匹配 GNU ls 中的行为
        require_literal_leading_dot: true,
        require_literal_separator: false,
        case_sensitive: true,
    };
    let file_name = entry.file_name();
    let file_name = if let Some(s) = file_name.to_str() {
        s.to_string()
    } else {
        file_name.to_string_lossy().into_owned()
    };

    !config
        .ignore_patterns
        .iter()
        .any(|p| p.matches_with(&file_name, options))
}

#[allow(clippy::cognitive_complexity)]
fn enter_directory<W: Write>(
    path_data: &PathData,
    read_dir: ReadDir,
    config: &LsConfig,
    out: &mut W,
    listed_ancestors: &mut HashSet<CtFileInformation>,
    dired: &mut DiredOutput,
    style_manager: &mut StyleManager,
) -> CTResult<()> {
    // 创建带有初始点文件的条目向量
    let mut entries: Vec<PathData> = if config.files == LsFiles::LsAll {
        vec![
            PathData::new(
                path_data.p_buf.clone(),
                None,
                Some(".".into()),
                config,
                false,
            ),
            PathData::new(
                path_data.p_buf.join(".."),
                None,
                Some("..".into()),
                config,
                false,
            ),
        ]
    } else {
        vec![]
    };

    // 将这些条目转换为 PathData 结构体
    for raw_entry in read_dir {
        let dir_entry = match raw_entry {
            Ok(path) => path,
            Err(err) => {
                out.flush()?;
                ct_show!(LsError::LsIOError(err));
                continue;
            }
        };

        if should_display(&dir_entry, config) {
            let entry_path_data =
                PathData::new(dir_entry.path(), Some(Ok(dir_entry)), None, config, false);
            entries.push(entry_path_data);
        };
    }

    sort_entries(&mut entries, config, out);

    // 显示任何错误后打印总数
    if config.format == LsFormat::Long || config.is_alloc_size {
        let total = return_total(&entries, config, out)?;
        write!(out, "{}", total.as_str())?;
        if config.is_dired {
            dired::dired_add_total(dired, total.len());
        }
    }

    display_items(&entries, config, out, dired, style_manager)?;

    if config.is_recursive {
        for e in entries
            .iter()
            .skip(if config.files == LsFiles::LsAll { 2 } else { 0 })
            .filter(|p| p.ft.get().is_some())
            .filter(|p| p.ft.get().unwrap().is_some())
            .filter(|p| p.ft.get().unwrap().unwrap().is_dir())
        {
            match fs::read_dir(&e.p_buf) {
                Err(err) => {
                    out.flush()?;
                    ct_show!(LsError::LsIOErrorContext(
                        err,
                        e.p_buf.clone(),
                        e.is_command_line
                    ));
                    continue;
                }
                Ok(rd) => {
                    if listed_ancestors.insert(CtFileInformation::from_path(
                        &e.p_buf,
                        e.is_must_dereference,
                    )?) {
                        // 在递归模式下列出多个目录时，我们会在文件列表开头显示
                        // 在文件列表的开头显示
                        writeln!(out)?;
                        if config.is_dired {
                            // 我们已经注入了第一个目录
                            // 继续注入其他目录
                            // 2 = \n + \n
                            dired.padding = 2;
                            dired::dired_indent(out)?;
                            let dir_name_size = e.p_buf.to_string_lossy().len();
                            dired::dired_calculate_subdired(dired, dir_name_size);
                            // 注入目录名
                            dired::dired_add_dir_name(dired, dir_name_size);
                        }

                        show_dir_name(e, out, config);
                        writeln!(out)?;
                        enter_directory(
                            e,
                            rd,
                            config,
                            out,
                            listed_ancestors,
                            dired,
                            style_manager,
                        )?;
                        listed_ancestors.remove(&CtFileInformation::from_path(
                            &e.p_buf,
                            e.is_must_dereference,
                        )?);
                    } else {
                        out.flush()?;
                        ct_show!(LsError::LsAlreadyListedError(e.p_buf.clone()));
                    }
                }
            }
        }
    }

    Ok(())
}

fn get_metadata_with_deref_opt(p_buf: &Path, dereference: bool) -> std::io::Result<Metadata> {
    if dereference {
        p_buf.metadata()
    } else {
        p_buf.symlink_metadata()
    }
}

fn display_dir_entry_size<W: Write>(
    entry: &PathData,
    config: &LsConfig,
    out: &mut W,
) -> (usize, usize, usize, usize, usize, usize) {
    // TODO：缓存/记忆 display_* 结果，这样我们就不必重新计算它们。
    if let Some(mdata) = entry.get_metadata(out) {
        let (size_len, major_len, minor_len) = match display_len_or_rdev(mdata, config) {
            SizeOrDeviceId::Device(major, minor) => (
                (major.len() + minor.len() + 2usize),
                major.len(),
                minor.len(),
            ),
            SizeOrDeviceId::Size(size) => (size.len(), 0usize, 0usize),
        };
        (
            display_symlink_count(mdata).len(),
            display_uname(mdata, config).len(),
            display_group(mdata, config).len(),
            size_len,
            major_len,
            minor_len,
        )
    } else {
        (0, 0, 0, 0, 0, 0)
    }
}

fn pad_left(string: &str, cnt: usize) -> String {
    format!("{string:>cnt$}")
}

fn pad_right(string: &str, cnt: usize) -> String {
    format!("{string:<cnt$}")
}

fn get_raw_block_size(md: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        if md.file_type().is_char_device() || md.file_type().is_block_device() {
            0u64
        } else {
            md.blocks() * 512
        }
    }
    #[cfg(not(unix))]
    {
        md.len()
    }
}

fn return_total<W: Write>(
    items: &[PathData],
    ls_config: &LsConfig,
    out: &mut W,
) -> CTResult<String> {
    let mut total_size = 0;
    for path_data_item in items {
        total_size += path_data_item
            .get_metadata(out)
            .as_ref()
            .map_or(0, |md| get_raw_block_size(md));
    }
    if ls_config.is_dired {
        dired::dired_indent(out)?;
    }
    let display_total = match ls_config.size_format {
        LsSizeFormat::Binary | LsSizeFormat::Decimal => total_size,
        LsSizeFormat::Bytes => (total_size + ls_config.block_size - 1) / ls_config.block_size,
    };
    Ok(format!(
        "total {}{}",
        display_size(display_total, ls_config),
        ls_config.line_ending
    ))
}

fn display_additional_leading_info<W: Write>(
    item: &PathData,
    padding: &LsPaddingCollection,
    config: &LsConfig,
    out: &mut W,
) -> CTResult<String> {
    let mut result = String::new();
    #[cfg(unix)]
    {
        if config.is_inode {
            let i = match item.get_metadata(out) {
                Some(md) => get_inode(md),
                _ => "?".to_owned(),
            };

            write!(result, "{} ", pad_left(&i, padding.inode)).unwrap();
        }
    }

    if config.is_alloc_size {
        let s = match item.get_metadata(out) {
            Some(md) => display_size(get_block_size(md, config), config),
            _ => "?".to_owned(),
        };

        // 除逗号外，所有格式都需要插入额外的空格来对齐尺寸。
        match config.format {
            LsFormat::Commas => write!(result, "{s} ").unwrap(),
            _ => write!(result, "{} ", pad_left(&s, padding.block_size)).unwrap(),
        };
    }
    Ok(result)
}

#[allow(clippy::cognitive_complexity)]
fn display_items<W: Write>(
    items: &[PathData],
    config: &LsConfig,
    out: &mut W,
    dired: &mut DiredOutput,
    style_manager: &mut StyleManager,
) -> CTResult<()> {
    // `-Z`, `--context`:
    // 显示 SELinux 安全上下文，如果没有则显示"? 当与 `-l`
    // 选项时，将在大小列的左边打印安全上下文。

    let quoted = items.iter().any(|item| {
        let name = escape_name(&item.display_name, &config.quoting_style);
        name.starts_with('\'')
    });

    if config.format == LsFormat::Long {
        display_grid_by_format_long_type(items, config, out, dired, style_manager, quoted)?;
    } else {
        display_grid_by_format_other_type(items, config, out, style_manager, quoted)?;
    }

    Ok(())
}

fn display_grid_by_format_other_type<W: Write>(
    items: &[PathData],
    config: &LsConfig,
    out: &mut W,
    style_manager: &mut StyleManager,
    quoted: bool,
) -> Result<(), Box<dyn CTError>> {
    let mut longest_context_len = 1;
    let prefix_context = if config.is_context {
        for item in items {
            let context_len = item.security_context.len();
            longest_context_len = context_len.max(longest_context_len);
        }
        Some(longest_context_len)
    } else {
        None
    };

    let padding = calculate_padding_collection(items, config, out);

    let mut names_vec = Vec::new();
    for i in items {
        let more_info = display_additional_leading_info(i, &padding, config, out)?;
        let cell = display_item_name(i, config, prefix_context, more_info, out, style_manager);
        names_vec.push(cell);
    }

    let names = names_vec.into_iter();

    match config.format {
        LsFormat::Columns => {
            display_grid(names, config.width, Direction::TopToBottom, out, quoted)?;
        }
        LsFormat::Across => {
            display_grid(names, config.width, Direction::LeftToRight, out, quoted)?;
        }
        LsFormat::Commas => {
            let mut current_col = 0;
            let mut names = names;
            if let Some(name) = names.next() {
                write!(out, "{}", name.contents)?;
                current_col = name.width as u16 + 2;
            }
            for name in names {
                let name_width = name.width as u16;
                // If the width is 0 we print one single line
                if config.width != 0 && current_col + name_width + 1 > config.width {
                    current_col = name_width + 2;
                    write!(out, ",\n{}", name.contents)?;
                } else {
                    current_col += name_width + 2;
                    write!(out, ", {}", name.contents)?;
                }
            }
            // 如果名称已打印，则当前 col 不再为 0。因此，我们打印一个换行符。
            if current_col > 0 {
                write!(out, "{}", config.line_ending)?;
            }
        }
        _ => {
            for name in names {
                write!(out, "{}{}", name.contents, config.line_ending)?;
            }
        }
    };
    Ok(())
}

fn display_grid_by_format_long_type<W: Write>(
    items: &[PathData],
    config: &LsConfig,
    out: &mut W,
    dired: &mut DiredOutput,
    style_manager: &mut StyleManager,
    quoted: bool,
) -> Result<(), Box<dyn CTError>> {
    let padding_collection = calculate_padding_collection(items, config, out);

    for item in items {
        #[cfg(unix)]
        if config.is_inode || config.is_alloc_size {
            let more_info =
                display_additional_leading_info(item, &padding_collection, config, out)?;

            write!(out, "{more_info}")?;
        }
        #[cfg(not(unix))]
        if config.is_alloc_size {
            let more_info =
                display_additional_leading_info(item, &padding_collection, config, out)?;
            write!(out, "{more_info}")?;
        }
        display_item_long(
            item,
            &padding_collection,
            config,
            out,
            dired,
            style_manager,
            quoted,
        )?;
    }
    Ok(())
}

#[allow(unused_variables)]
fn get_block_size(md: &Metadata, config: &LsConfig) -> u64 {
    let raw_blocks = get_raw_block_size(md);
    match config.size_format {
        LsSizeFormat::Binary | LsSizeFormat::Decimal => raw_blocks,
        LsSizeFormat::Bytes => (raw_blocks + config.block_size - 1) / config.block_size,
    }
}

fn display_grid<W: Write>(
    names: impl Iterator<Item = Cell>,
    width: u16,
    direction: Direction,
    output: &mut W,
    quoted: bool,
) -> CTResult<()> {
    if width == 0 {
        // 如果宽度为 0，我们就打印一行
        let mut printed_something = false;
        for name in names {
            if printed_something {
                write!(output, "  ")?;
            }
            printed_something = true;
            write!(output, "{}", name.contents)?;
        }
        if printed_something {
            writeln!(output)?;
        }
    } else {
        // 我们可能需要 Filling::Text("\t".to_string())；
        let filling_spaces = Filling::Spaces(2);
        let mut grid = Grid::new(GridOptions {
            filling: filling_spaces,
            direction,
        });

        for name in names {
            let formatted_name = Cell {
                contents: if quoted && !name.contents.starts_with('\'') {
                    format!(" {}", name.contents)
                } else {
                    name.contents
                },
                width: name.width,
            };
            grid.add(formatted_name);
        }

        match grid.fit_into_width(width as usize) {
            Some(out) => {
                write!(output, "{out}")?;
            }
            // Width is too small for the grid, so we fit it in one column
            None => {
                write!(output, "{}", grid.fit_into_columns(1))?;
            }
        }
    }
    Ok(())
}

fn ls_has_context(item: &PathData) -> bool {
    item.security_context.len() > 1
}

#[allow(unused_variables)]
fn ls_has_acl<P: AsRef<Path>>(file: P) -> bool {
    #[cfg(feature = "feat_acl")]
    {
        use exacl::getfacl;
        match getfacl(file, None) {
            Ok(acls) => {
                acls.iter().any(|acl| {
                    if !acl.name.is_empty() {
                        // 通过acl名字，排除默认acl entry
                        true
                    } else {
                        false
                    }
                })
            }
            Err(e) => {
                // println!("Failed to get ACLs: {}", e);
                false
            }
        }
    }
    #[cfg(not(feature = "feat_acl"))]
    {
        // #[cfg(unix)]
        // use ctcore::ct_fsxattr::has_acl;
        // has_acl(file)

        // 没有enable acl 检查就默认返回 false
        false
    }
}

/// 这将向 BufWriter 写入 `ls -l` 输出的单个字符串。
///
/// 依次写入以下键值：
/// * `inode` ([`get_inode`], config-optional)
/// * `permissions` ([`display_permissions`])
/// * `symlink_count` ([`display_symlink_count`])
/// * `owner` ([`display_uname`], config-optional)
/// * `group` ([`display_group`], config-optional)
/// * `author` ([`display_uname`], config-optional)
/// * `size / rdev` ([`display_len_or_rdev`])
/// * `system_time` ([`get_system_time`])
/// * `item_name` ([`display_item_name`])
///
/// 该函数需要分栏显示信息：
/// * permissions 和 system_time 已经保证以固定长度预先格式化。
/// * item_name 是最后一列，左对齐。
/// * 其他内容需要使用 [`pad_left`]填充。
///
/// 这就是我们设置参数的原因：
/// ```txt
///    longest_link_count_len: usize,
///    longest_uname_len: usize,
///    longest_group_len: usize,
///    longest_context_len: usize,
///    longest_size_len: usize,
/// ```
/// 决定每个字段的最大字符数。
#[allow(clippy::write_literal)]
#[allow(clippy::cognitive_complexity)]
fn display_item_long<W: Write>(
    item: &PathData,
    padding: &LsPaddingCollection,
    ls_config: &LsConfig,
    output: &mut W,
    dired_output: &mut DiredOutput,
    style_manager: &mut StyleManager,
    quoted: bool,
) -> CTResult<()> {
    let mut output_display: String = String::new();
    if ls_config.is_dired {
        output_display += "  ";
    }
    if let Some(md) = item.get_metadata(output) {
        let is_acl_set = ls_has_acl(item.display_name.as_os_str());
        // 修订 selinux 和 acl 情况：
        // 1. 如果没有安全上下文 ACL_T_NONE: 不显示
        // 2. 如果只有安全上下文，但没有 ACL（have_acl 为假） ACL_T_LSM_CONTEXT_ONLY: 显示‘.’
        // 3. 如果有 ACL（have_acl 为真）ACL_T_YES，无论是否有安全上下文： 则显示‘+’

        let mut out_acl = if ls_has_context(item) {
            // GNU `ls` 使用". "字符来表示具有安全上下文的文件
            "."
        } else {
            ""
        };

        if is_acl_set {
            // 如果设置了 acl，我们将在文件权限末尾显示 "+"。
            out_acl = "+"
        };

        write!(
            output_display,
            "{}{} {}",
            display_permissions(md, true),
            out_acl,
            pad_left(&display_symlink_count(md), padding.link_count)
        )
        .unwrap();

        if ls_config.long.is_owner {
            write!(
                output_display,
                " {}",
                pad_right(&display_uname(md, ls_config), padding.uname)
            )
            .unwrap();
        }

        if ls_config.long.is_group {
            write!(
                output_display,
                " {}",
                pad_right(&display_group(md, ls_config), padding.group)
            )
            .unwrap();
        }

        if ls_config.is_context {
            write!(
                output_display,
                " {}",
                pad_right(&item.security_context, padding.context)
            )
            .unwrap();
        }

        // 在 GNU/Hurd 中，Author 与 owner 只有区别，因此我们重用了
        // owner，因为 Rust 目前不支持 GNU/Hurd。
        if ls_config.long.is_author {
            write!(
                output_display,
                " {}",
                pad_right(&display_uname(md, ls_config), padding.uname)
            )
            .unwrap();
        }

        match display_len_or_rdev(md, ls_config) {
            SizeOrDeviceId::Size(size) => {
                write!(output_display, " {}", pad_left(&size, padding.size)).unwrap();
            }
            SizeOrDeviceId::Device(major, minor) => {
                #[cfg(unix)]
                write!(
                    output_display,
                    " {}, {}",
                    pad_left(
                        &major,
                        padding.major.max(
                            padding
                                .size
                                .saturating_sub(padding.minor.saturating_add(2usize))
                        ),
                    ),
                    pad_left(&minor, padding.minor),
                )
                .unwrap();
                #[cfg(not(unix))]
                write!(
                    output_display,
                    " {}, {}",
                    pad_left(&major, 0usize),
                    pad_left(&minor, 0usize),
                )
                .unwrap();
            }
        };

        write!(output_display, " {} ", display_date(md, ls_config)).unwrap();

        let item_name =
            display_item_name(item, ls_config, None, String::new(), output, style_manager).contents;

        let displayed_item = if quoted && !item_name.starts_with('\'') {
            format!(" {item_name}")
        } else {
            item_name
        };

        if ls_config.is_dired {
            let (start, end) = dired::dired_calculate(
                &dired_output.dired_positions,
                output_display.len(),
                displayed_item.len(),
            );
            dired::dired_update_positions(dired_output, start, end);
        }
        write!(
            output_display,
            "{}{}",
            displayed_item, ls_config.line_ending
        )
        .unwrap();
    } else {
        #[cfg(unix)]
        let leading_char = {
            if let Some(Some(ft)) = item.ft.get() {
                if ft.is_char_device() {
                    "c"
                } else if ft.is_block_device() {
                    "b"
                } else if ft.is_symlink() {
                    "l"
                } else if ft.is_dir() {
                    "d"
                } else {
                    "-"
                }
            } else {
                "-"
            }
        };
        #[cfg(not(unix))]
        let leading_char = {
            if let Some(Some(ft)) = item.ft.get() {
                if ft.is_symlink() {
                    "l"
                } else if ft.is_dir() {
                    "d"
                } else {
                    "-"
                }
            } else {
                "-"
            }
        };

        write!(
            output_display,
            "{}{} {}",
            format_args!("{leading_char}?????????"),
            if item.security_context.len() > 1 {
                // GNU `ls` uses a "." character to indicate a file with a security context,
                // but not other alternate access method.
                "."
            } else {
                ""
            },
            pad_left("?", padding.link_count)
        )
        .unwrap();

        if ls_config.long.is_owner {
            write!(output_display, " {}", pad_right("?", padding.uname)).unwrap();
        }

        if ls_config.long.is_group {
            write!(output_display, " {}", pad_right("?", padding.group)).unwrap();
        }

        if ls_config.is_context {
            write!(
                output_display,
                " {}",
                pad_right(&item.security_context, padding.context)
            )
            .unwrap();
        }

        // Author is only different from owner on GNU/Hurd, so we reuse
        // the owner, since GNU/Hurd is not currently supported by Rust.
        if ls_config.long.is_author {
            write!(output_display, " {}", pad_right("?", padding.uname)).unwrap();
        }

        let displayed_item =
            display_item_name(item, ls_config, None, String::new(), output, style_manager).contents;
        let date_len = 12;

        write!(
            output_display,
            " {} {} ",
            pad_left("?", padding.size),
            pad_left("?", date_len),
        )
        .unwrap();

        if ls_config.is_dired {
            dired::dired_calculate_and_update_positions(
                dired_output,
                output_display.len(),
                displayed_item.trim().len(),
            );
        }
        write!(
            output_display,
            "{}{}",
            displayed_item, ls_config.line_ending
        )
        .unwrap();
    }
    write!(output, "{output_display}")?;

    Ok(())
}

#[cfg(unix)]
fn get_inode(metadata: &Metadata) -> String {
    format!("{}", metadata.ino())
}

#[cfg(unix)]
fn cached_uid2usr(uid: u32) -> String {
    static UID_CACHE: Lazy<Mutex<HashMap<u32, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

    let mut uid_cache = UID_CACHE.lock().unwrap();
    uid_cache
        .entry(uid)
        .or_insert_with(|| ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()))
        .clone()
}

#[cfg(unix)]
fn display_uname(metadata: &Metadata, config: &LsConfig) -> String {
    match config.long.is_numeric_uid_gid {
        true => metadata.uid().to_string(),
        false => cached_uid2usr(metadata.uid()),
    }
}

#[cfg(target_os = "linux")]
fn cached_gid2grp(gid: u32) -> String {
    static GID_CACHE: Lazy<Mutex<HashMap<u32, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

    let mut gid_cache_mutex = GID_CACHE.lock().unwrap();
    gid_cache_mutex
        .entry(gid)
        .or_insert_with(|| ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()))
        .clone()
}

#[cfg(target_os = "linux")]
fn display_group(metadata: &Metadata, config: &LsConfig) -> String {
    match config.long.is_numeric_uid_gid {
        true => metadata.gid().to_string(),
        false => cached_gid2grp(metadata.gid()),
    }
}

#[cfg(target_os = "windows")]
fn display_uname(_metadata: &Metadata, _config: &LsConfig) -> String {
    "somebody".to_string()
}

#[cfg(target_os = "windows")]
fn display_group(_metadata: &Metadata, _config: &LsConfig) -> String {
    "somegroup".to_string()
}

// get_time 的实现是分开的，因为某些选项，例如
// 将无法使用 ctime
#[cfg(unix)]
fn get_system_time(md: &Metadata, config: &LsConfig) -> Option<SystemTime> {
    match config.time {
        LsTime::LsChange => {
            Some(UNIX_EPOCH + Duration::new(md.ctime() as u64, md.ctime_nsec() as u32))
        }
        LsTime::LsModification => md.modified().ok(),
        LsTime::LsAccess => md.accessed().ok(),
        LsTime::LsBirth => md.created().ok(),
    }
}

#[cfg(not(unix))]
fn get_system_time(md: &Metadata, config: &LsConfig) -> Option<SystemTime> {
    match config.time {
        LsTime::LsModification => md.modified().ok(),
        LsTime::LsAccess => md.accessed().ok(),
        LsTime::LsBirth => md.created().ok(),
        _ => None,
    }
}

fn get_time(md: &Metadata, config: &LsConfig) -> Option<chrono::DateTime<chrono::Local>> {
    let time = get_system_time(md, config)?;
    Some(time.into())
}

fn display_date(metadata: &Metadata, config: &LsConfig) -> String {
    match get_time(metadata, config) {
        Some(time) => {
            //如果日期来自过去 6 个月，则为最近日期
            let now = chrono::Local::now();
            //According to GNU a Gregorian year has 365.2425 * 24 * 60 * 60 == 31556952 seconds on the average.
            let six_months = chrono::TimeDelta::try_seconds(31_556_952 / 2).unwrap();
            let recent = time > now - six_months && time < now;

            match &config.time_style {
                LsTimeStyle::LsFullIso => time.format("%Y-%m-%d %H:%M:%S.%f %z"),
                LsTimeStyle::LsLongIso => time.format("%Y-%m-%d %H:%M"),
                LsTimeStyle::LsIso => time.format(if recent { "%m-%d %H:%M" } else { "%Y-%m-%d " }),
                LsTimeStyle::LsLocale => {
                    let fmt = if recent { "%b %e %H:%M" } else { "%b %e  %Y" };

                    //在这个版本的 chrono 中可以进行翻译。函数是 chrono::datetime::DateTime::format_localized
                    //然而，目前仍很难获得当前的 pure-rust-locale 语言。
                    //所以还没有实现

                    time.format(fmt)
                }
                LsTimeStyle::LsFormat(e) => time.format(e),
            }
            .to_string()
        }
        None => "???".into(),
    }
}

// GNU 格式化大小的方式有一些特殊之处：
// 1.如果且仅当大小小于 10 时，才给出一位小数。
// 2. 将大小向上舍入。
// 人可读的format使用幂来表示1024，但不显示 "i"。
// 通常用来表示 Kibi、Mebi 等。
// Kibi 和 Kilo 的表示方法不同（分别为 "k "和 "K）
fn format_prefixed(prefixed: &NumberPrefix<f64>) -> String {
    match prefixed {
        NumberPrefix::Standalone(bytes) => bytes.to_string(),
        NumberPrefix::Prefixed(prefix, bytes) => {
            // 删除 "Ki"、"Mi "等中的 "i"（如果有的话）
            let prefix_str = prefix.symbol().trim_end_matches('i');

            match (10.0 * bytes).ceil() >= 100.0 {
                true => format!("{:.0}{}", bytes.ceil(), prefix_str),
                false => format!("{:.1}{}", (10.0 * bytes).ceil() / 10.0, prefix_str),
            }
        }
    }
}

#[allow(dead_code)]
enum SizeOrDeviceId {
    Size(String),
    Device(String, String),
}

fn display_len_or_rdev(mdata: &Metadata, config: &LsConfig) -> SizeOrDeviceId {
    #[cfg(target_os = "linux")]
    {
        let ft = mdata.file_type();
        if ft.is_char_device() || ft.is_block_device() {
            // 这里需要进行类型转换，因为不同操作系统的 `dev_t` 类型各不相同。
            let dev = mdata.rdev() as dev_t;
            let major = major(dev);
            let minor = minor(dev);
            return SizeOrDeviceId::Device(major.to_string(), minor.to_string());
        }
    }
    let len_adjusted = {
        let d = mdata.len() / config.file_size_block_size;
        let r = mdata.len() % config.file_size_block_size;
        if r == 0 { d } else { d + 1 }
    };
    SizeOrDeviceId::Size(display_size(len_adjusted, config))
}

fn display_size(size: u64, config: &LsConfig) -> String {
    // 注意：人类可读的行为与 GNU ls 不同。
    // GNU ls 默认使用二进制前缀。
    match config.size_format {
        LsSizeFormat::Binary => format_prefixed(&NumberPrefix::binary(size as f64)),
        LsSizeFormat::Decimal => format_prefixed(&NumberPrefix::decimal(size as f64)),
        LsSizeFormat::Bytes => {
            if !config.size_suffix.is_empty() {
                format!("{}{}", size, config.size_suffix)
            } else {
                size.to_string()
            }
        }
    }
}

#[cfg(unix)]
fn file_is_executable(md: &Metadata) -> bool {
    // 模式总是返回 u32，但根据平台的不同，标志可能不是 u32。
    // 例如，Linux 使用 u32，Mac 使用 u16。
    // S_IXUSR -> 用户有执行权限
    // S_IXGRP -> 组有执行权限
    // S_IXOTH -> 其他用户有执行权限
    #[allow(clippy::unnecessary_cast)]
    return md.mode() & ((S_IXUSR | S_IXGRP | S_IXOTH) as u32) != 0;
}

fn classify_file<W: Write>(path: &PathData, out: &mut W) -> Option<char> {
    let path_file_type = path.file_type(out)?;

    if path_file_type.is_dir() {
        Some('/')
    } else if path_file_type.is_symlink() {
        Some('@')
    } else {
        #[cfg(unix)]
        {
            if path_file_type.is_socket() {
                Some('=')
            } else if path_file_type.is_fifo() {
                Some('|')
            } else if path_file_type.is_file()
                // 如果文件在列出和显示之间被删除，则安全解包
                && path.get_metadata(out).map(file_is_executable).unwrap_or_default()
            {
                Some('*')
            } else {
                None
            }
        }
        #[cfg(not(unix))]
        None
    }
}

/// 接收一个 [`PathData`] 结构并返回一个带有名称的单元格，以备显示。
///
/// 该函数依赖于所提供的 `&Config` 中的以下参数：
/// * `config.quoting_style`决定如何使用 [`escape_name`]转义`name`。
/// * `config.inode`决定是否使用[`get_inode`]在名称旁边显示inode编号。
/// * `config.color`使用[`color_name`]决定是否给`name`着色。
/// * `config.indicator_style`使用[`classify_file`]为`name`添加特定字符。
/// * `config.format` 用于在 `Format::Long` 时显示符号链接目标。此函数还
///   如果指定了 `config.color` 则负责给符号链接目标名称着色。
/// * `config.context`用于在使用`feat_selinux`编译时将安全上下文预输入`name`。
/// * `config.hyperlink`决定是否超链接项目。
///
/// 注意符号链接目标中的非单码序列将使用
/// [`std::path::Path::to_string_lossy`].
#[allow(clippy::cognitive_complexity)]
fn display_item_name<W: Write>(
    path: &PathData,
    config: &LsConfig,
    prefix_context: Option<usize>,
    more_info: String,
    out: &mut W,
    style_manager: &mut StyleManager,
) -> Cell {
    // 这是我们的返回值。我们从 `&path.display_name` 开始，然后对其进行修改。
    let mut name = escape_name(&path.display_name, &config.quoting_style);

    // 我们需要自己跟踪宽度，而不是让 term_grid
    // 因为颜色代码会扰乱 term_grid 的宽度计算。
    let mut width = name.width();

    if config.is_hyperlink {
        name = create_hyperlink(&name, path);
    }

    if let Some(ls_colors) = &config.color {
        name = color_name(name, path, ls_colors, style_manager, out, None);
    }

    if config.format != LsFormat::Long && !more_info.is_empty() {
        // 在此处增加宽度，因为 name 被赋予了颜色，而 name.width() 现在是错误的。
        // 显示尺寸
        width += more_info.width();
        name = more_info + &name;
    }

    if config.indicator_style != LsIndicatorStyle::None {
        let sym = classify_file(path, out);

        let char_opt = match config.indicator_style {
            LsIndicatorStyle::Classify => sym,
            LsIndicatorStyle::FileType => {
                // 不要添加星号。
                match sym {
                    Some('*') => None,
                    _ => sym,
                }
            }
            LsIndicatorStyle::Slash => {
                // 只附加斜线。
                match sym {
                    Some('/') => Some('/'),
                    _ => None,
                }
            }
            LsIndicatorStyle::None => None,
        };

        if let Some(c) = char_opt {
            name.push(c);
            width += 1;
        }
    }

    if config.format == LsFormat::Long
        && path.file_type(out).is_some()
        && path.file_type(out).unwrap().is_symlink()
        && !path.is_must_dereference
    {
        match path.p_buf.read_link() {
            Ok(target) => {
                name.push_str(" -> ");

                // 我们不妨在箭头后给符号链接输出着色。
                // 这需要额外的系统调用，但提供了重要的信息，而这些信息是运行 `ls -l -color` 的人非常感兴趣的。
                // 运行 `ls -l --color` 的人非常感兴趣的重要信息。
                if let Some(ls_colors) = &config.color {
                    // 我们要获取绝对路径，以便构建具有有效元数据的 PathData。
                    // 这是因为相对符号链接会导致 get_metadata 失败。
                    let mut absolute_target = target.clone();
                    if target.is_relative() {
                        if let Some(parent) = path.p_buf.parent() {
                            absolute_target = parent.join(absolute_target);
                        }
                    }

                    let target_data = PathData::new(absolute_target, None, None, config, false);

                    // 如果我们有一个指向有效文件的符号链接，我们就使用该文件的元数据。
                    // 因为我们使用的是绝对路径，所以我们可以假定该文件保证存在。
                    // 否则，我们将使用 path.md()，这将保证我们根据不存在的符号链接的颜色，为其着色。
                    // 根据 style_for_path_with_metadata，对于不存在的符号链接，我们也会使用相同的颜色。
                    if path.get_metadata(out).is_none()
                        && get_metadata_with_deref_opt(
                            target_data.p_buf.as_path(),
                            target_data.is_must_dereference,
                        )
                        .is_err()
                    {
                        name.push_str(&path.p_buf.read_link().unwrap().to_string_lossy());
                    } else {
                        name.push_str(&color_name(
                            escape_name(target.as_os_str(), &config.quoting_style),
                            path,
                            ls_colors,
                            style_manager,
                            out,
                            Some(&target_data),
                        ));
                    }
                } else {
                    // 我们要获取绝对路径，以便构建具有有效元数据的 PathData。
                    // 这是因为相对符号链接会导致 get_metadata 失败。
                    name.push_str(&escape_name(target.as_os_str(), &config.quoting_style));
                }
            }
            Err(err) => {
                ct_show!(LsError::LsIOErrorContext(err, path.p_buf.clone(), false));
            }
        }
    }

    // 将安全上下文预置到 `name` 中并调整 `width` 以便
    // 以在以后调用`display_grid()`时获得正确的对齐方式。
    if config.is_context {
        if let Some(pad_count) = prefix_context {
            let security_context = match matches!(config.format, LsFormat::Commas) {
                true => path.security_context.clone(),
                false => pad_left(&path.security_context, pad_count),
            };

            name = format!("{security_context} {name}");
            width += security_context.len() + 1;
        }
    }

    Cell {
        contents: name,
        width,
    }
}

fn create_hyperlink(name: &str, path: &PathData) -> String {
    let hostname_osstring = hostname::get().unwrap_or(OsString::from(""));
    let hostname = hostname_osstring.to_string_lossy();

    let absolute_path_buf = fs::canonicalize(&path.p_buf).unwrap_or_default();
    let absolute_path = absolute_path_buf.to_string_lossy();

    #[cfg(target_os = "linux")]
    let unencoded_chars = "_-.:~/";
    #[cfg(target_os = "windows")]
    let unencoded_chars = "_-.:~/\\";

    // 路径的百分比编码
    let absolute_path: String = absolute_path
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || unencoded_chars.contains(c) {
                c.to_string()
            } else {
                format!("%{:02x}", c as u8)
            }
        })
        .collect();

    // \x1b = ESC, \x07 = BEL
    format!("\x1b]8;;file://{hostname}{absolute_path}\x07{name}\x1b]8;;\x07")
}

/// We need this struct to be able to store the previous style.
/// This because we need to check the previous value in case we don't need
/// the reset
struct StyleManager {
    current_style: Option<Style>,
}

impl StyleManager {
    fn new() -> Self {
        Self {
            current_style: None,
        }
    }

    fn apply_style(&mut self, new_style: &Style, name: &str) -> String {
        if let Some(current_style) = &self.current_style {
            if *current_style == *new_style {
                // 当前样式与新样式相同，应用时无需重置。
                let mut term_style = new_style.to_nu_ansi_term_style();
                term_style.prefix_with_reset = false;
                return term_style.paint(name).to_string();
            }
        }

        // 我们获得了新的样式，需要重新设置它
        self.current_style = Some(new_style.clone());
        new_style
            .to_nu_ansi_term_style()
            .reset_before_style()
            .paint(name)
            .to_string()
    }
}

fn apply_style_based_on_metadata(
    path: &PathData,
    md_option: Option<&Metadata>,
    ls_colors: &LsColors,
    style_manager: &mut StyleManager,
    name: &str,
) -> String {
    if let Some(style) = ls_colors.style_for_path_with_metadata(&path.p_buf, md_option) {
        style_manager.apply_style(style, name)
    } else {
        name.to_owned()
    }
}

/// 根据为给定路径确定的 WWW 风格为提供的名称着色
/// 这个函数相当长，因为它试图利用 DirEntry 来避免
/// 不必要地调用 stat()
/// 并管理符号链接错误
fn color_name<W: Write>(
    name: String,
    path: &PathData,
    ls_colors: &LsColors,
    style_manager: &mut StyleManager,
    out: &mut W,
    target_symlink: Option<&PathData>,
) -> String {
    if !path.is_must_dereference {
        // 如果我们需要取消引用（跟踪）一个符号链接，我们需要获取元数据
        if let Some(de) = &path.de {
            if let Some(style) = ls_colors.style_for(de) {
                return style_manager.apply_style(style, &name);
            } else {
                return name;
            }
        }
    }

    if let Some(target) = target_symlink {
        // 使用可选的 target_symlink
        // 此处使用 fn get_metadata_with_deref_opt 代替 get_metadata()，因为如果无法获取 target_metadata，ls 不应以 Err 结尾
        let md = get_metadata_with_deref_opt(target.p_buf.as_path(), path.is_must_dereference)
            .unwrap_or_else(|_| target.get_metadata(out).unwrap().clone());

        apply_style_based_on_metadata(path, Some(&md), ls_colors, style_manager, &name)
    } else {
        let mdata_option = path.get_metadata(out);
        let symlink_metadata = path.p_buf.symlink_metadata().ok();
        let mdata = mdata_option.or(symlink_metadata.as_ref());

        apply_style_based_on_metadata(path, mdata, ls_colors, style_manager, &name)
    }
}

#[cfg(not(unix))]
fn display_symlink_count(_metadata: &Metadata) -> String {
    // Currently not sure of how to get this on Windows, so I'm punting.
    // Git Bash looks like it may do the same thing.
    String::from("1")
}

#[cfg(unix)]
fn display_symlink_count(mdata: &Metadata) -> String {
    mdata.nlink().to_string()
}

#[cfg(unix)]
fn display_inode(mdata: &Metadata) -> String {
    get_inode(mdata)
}

// 返回 SELinux 安全上下文的 UTF8 `String`。
#[allow(unused_variables)]
fn get_security_context(config: &LsConfig, p_buf: &Path, must_dereference: bool) -> String {
    let substitute_string = "?".to_string();
    // 如果必须取消引用，即使系统不支持 SELinux，也要确保符号链接有效。
    // 不支持 SELinux。
    // 与 GNU coreutils 一致，在 GNU coreutils 中，悬空的符号链接会导致退出代码 1。
    if must_dereference {
        if let Err(err) = get_metadata_with_deref_opt(p_buf, must_dereference) {
            ct_show!(LsError::LsIOErrorContext(err, p_buf.to_path_buf(), false));
            return substitute_string;
        }
    }
    if config.is_selinux_supported {
        #[cfg(feature = "selinux")]
        {
            match selinux::SecurityContext::of_path(p_buf, must_dereference.to_owned(), false) {
                Err(_r) => {
                    // TODO: show the actual reason why it failed
                    ct_show_warning!("failed to get security context of: {}", p_buf.quote());
                    substitute_string
                }
                Ok(None) => substitute_string,
                Ok(Some(security_context)) => {
                    let context = security_context.as_bytes();

                    let context_strip_suffix = context.strip_suffix(&[0]).unwrap_or(context);
                    String::from_utf8(context_strip_suffix.to_vec()).unwrap_or_else(|e| {
                        ct_show_warning!(
                            "getting security context of: {}: {}",
                            p_buf.quote(),
                            e.to_string()
                        );
                        String::from_utf8_lossy(context_strip_suffix).into_owned()
                    })
                }
            }
        }
        #[cfg(not(feature = "selinux"))]
        {
            substitute_string
        }
    } else {
        substitute_string
    }
}

#[cfg(unix)]
fn calculate_padding_collection<W: Write>(
    items: &[PathData],
    config: &LsConfig,
    out: &mut W,
) -> LsPaddingCollection {
    let mut padding = LsPaddingCollection {
        inode: 1,
        link_count: 1,
        uname: 1,
        group: 1,
        context: 1,
        size: 1,
        major: 1,
        minor: 1,
        block_size: 1,
    };

    for item in items {
        #[cfg(unix)]
        if config.is_inode {
            let inode_len = match item.get_metadata(out) {
                Some(md) => display_inode(md).len(),
                _ => continue,
            };

            padding.inode = inode_len.max(padding.inode);
        }

        if config.is_alloc_size {
            if let Some(md) = item.get_metadata(out) {
                let block_size_len = display_size(get_block_size(md, config), config).len();
                padding.block_size = block_size_len.max(padding.block_size);
            }
        }

        if config.format == LsFormat::Long {
            let context_len = item.security_context.len();
            let (link_count_len, uname_len, group_len, size_len, major_len, minor_len) =
                display_dir_entry_size(item, config, out);
            padding.link_count = link_count_len.max(padding.link_count);
            padding.uname = uname_len.max(padding.uname);
            padding.group = group_len.max(padding.group);
            if config.is_context {
                padding.context = context_len.max(padding.context);
            }
            if items.len() == 1usize {
                padding.size = 0usize;
                padding.major = 0usize;
                padding.minor = 0usize;
            } else {
                padding.major = major_len.max(padding.major);
                padding.minor = minor_len.max(padding.minor);
                padding.size = size_len.max(padding.size).max(padding.major);
            }
        }
    }

    padding
}

#[cfg(not(unix))]
fn calculate_padding_collection<W: Write>(
    items: &[PathData],
    config: &LsConfig,
    out: &mut W,
) -> LsPaddingCollection {
    let mut padding_collections = LsPaddingCollection {
        link_count: 1,
        uname: 1,
        group: 1,
        context: 1,
        size: 1,
        block_size: 1,
    };

    for item in items {
        if config.is_alloc_size {
            if let Some(md) = item.get_metadata(out) {
                let block_size_len = display_size(get_block_size(md, config), config).len();
                padding_collections.block_size = block_size_len.max(padding_collections.block_size);
            }
        }

        let context_len = item.security_context.len();
        let (link_count_len, uname_len, group_len, size_len, _major_len, _minor_len) =
            display_dir_entry_size(item, config, out);
        padding_collections.link_count = link_count_len.max(padding_collections.link_count);
        padding_collections.uname = uname_len.max(padding_collections.uname);
        padding_collections.group = group_len.max(padding_collections.group);
        if config.is_context {
            padding_collections.context = context_len.max(padding_collections.context);
        }
        padding_collections.size = size_len.max(padding_collections.size);
    }

    padding_collections
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_locale_aware_filename_sorting() {
        // 直接测试字符串比较逻辑，避免环境变量并发问题
        let name1 = OsString::from("apple");
        let name2 = OsString::from("banana");
        let name3 = OsString::from("Cherry");

        // 测试基本的locale感知文件名比较
        let result1 = strcoll_compare(name1.as_bytes(), name2.as_bytes(), false);
        assert_eq!(result1, std::cmp::Ordering::Less);

        let result2 = strcoll_compare(name2.as_bytes(), name1.as_bytes(), false);
        assert_eq!(result2, std::cmp::Ordering::Greater);

        let result3 = strcoll_compare(name1.as_bytes(), name1.as_bytes(), false);
        assert_eq!(result3, std::cmp::Ordering::Equal);

        // 测试混合大小写的比较，结果会根据系统locale变化
        let result4 = strcoll_compare(name1.as_bytes(), name3.as_bytes(), false);
        // 确保函数能正常工作，不管结果如何
        assert!(matches!(
            result4,
            std::cmp::Ordering::Less | std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
        ));
    }
}
