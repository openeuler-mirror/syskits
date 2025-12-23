/*
 *  Copyright(c)2022-2024 china Telecom cloud Technologies co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 *  You can use this software according to the terms and conditions of the Mulan PSL V2
 *  You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *  THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *  KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *  NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *  See the Mulan PSL v2 for more details.
 */

use std::cmp::Reverse;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Write as FmtWrite};
use std::fs::{self, DirEntry, FileType, Metadata, ReadDir};
use std::io::{stdout, BufWriter, ErrorKind, Write};
#[cfg(likeunix)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{cell::OnceCell, num::IntErrorKind};

#[cfg(unix)]
use std::collections::HashMap;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::time::Duration;

use std::{collections::HashSet, io::IsTerminal};

use clap::builder::{NonEmptyStringValueParser, ValueParser};
use clap::{crate_version, Arg, ArgAction, Command};
use glob::{MatchOptions, Pattern};
use lscolors::{LsColors, Style};
use number_prefix::NumberPrefix;
#[cfg(unix)]
use once_cell::sync::Lazy;
use term_grid::{Cell, Direction, Filling, Grid, GridOptions};
use unicode_width::UnicodeWidthStr;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::set_ct_exit_code;
use ctcore::ct_error::CTError;
use ctcore::ct_error::CTResult;
use ctcore::ct_fs::display_permissions;
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_parse_size::parse_size_u64;
use ctcore::ct_version_cmp::ct_version_cmp;
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
use ctcore::{ct_parse_glob, ct_show, ct_show_error, ct_show_warning};
// Currently getpwuid is `linux` target only. If it's broken out into
// a posix-compliant attribute this can be updated...
#[cfg(unix)]
use ctcore::ct_entries;
use ctcore::ct_fs::CtFileInformation;
#[cfg(unix)]
use ctcore::ct_fsxattr::has_acl;
use ctcore::ct_quoting_style;
use ctcore::ct_quoting_style::escape_name;
use ctcore::ct_quoting_style::CtQuotingStyle;
#[cfg(target_os = "linux")]
use ctcore::libc::{dev_t, major, minor};
#[cfg(unix)]
use ctcore::libc::{S_IXGRP, S_IXOTH, S_IXUSR};
use dired::DiredOutput;

mod dired;

#[cfg(not(feature = "selinux"))]
static LS_CONTEXT_HELP_TEXT: &str = "print any security context of each file (not enabled)";
#[cfg(feature = "selinux")]
static LS_CONTEXT_HELP_TEXT: &str = "print any security context of each file";

const LS_ABOUT: &str = ct_help_about!("ls.md");
const LS_AFTER_HELP: &str = ct_help_section!("after help", "ls.md");
const LS_USAGE: &str = ct_help_usage!("ls.md");

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
            && options.indices_of(ls_flags::LS_FULL_TIME).unwrap().last()
                > options.indices_of(ls_flags::LS_TIME_STYLE).unwrap().last()
        {
            Ok(LsTimeStyle::LsFullIso)
        } else {
            let field_str = field.as_str();
            if "full-iso" == field_str {
                Ok(LsTimeStyle::LsFullIso)
            } else if "long-iso" == field_str {
                Ok(LsTimeStyle::LsLongIso)
            } else if "iso" == field_str {
                Ok(LsTimeStyle::LsIso)
            } else if "locale" == field_str {
                Ok(LsTimeStyle::LsLocale)
            } else {
                match field.chars().next().unwrap() {
                    '+' => Ok(LsTimeStyle::LsFormat(String::from(&field[1..]))),
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
        let context = options.get_flag(ls_flags::LS_CONTEXT);
        let (mut format, opt) = ls_extract_format(options);
        let files = extract_files(options);

        // -o、-n 和 -g 选项比较复杂。它们不能相互覆盖
        // 因为有可能将它们组合在一起。例如，选项
        // -og 应该同时隐藏所有者和组。此外，它们不会
        // 如果使用了 -l 或 --format=long 选项，它们不会被重置。因此，这些选项应该只显示
        // 组：-gl 或"-g --format=long" 。最后，它们也不会重置
        // 切换到不同的ct_format选项时，它们也不会重置：
        // -ogCl 或 "-og --format=vertical --format=long".
        //
        // -1 也有类似的问题：如果ct_format是长格式，它什么也不做。这
        // 这实际上使它与 --format=singe-column 选项不同、
        // 它始终适用。
        //
        // 这里的想法是不要让这些选项与其他
        // 选项，而是手动决定它们的索引是否大于
        // 其他 format 选项。如果是，我们就设置相应的ct_format。
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

        let ls_sort = extract_sort(options);
        let ls_time = extract_time(options);
        let mut is_needs_color = extract_color(options);
        let is_hyperlink = extract_hyperlink(options);

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
                    Ok(size) => match (is_env_var_blocksize, opt_kb) {
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
        let get_last = |flag: &str| -> usize {
            if options.value_source(flag) == Some(clap::parser::ValueSource::CommandLine) {
                options.index_of(flag).unwrap_or(0)
            } else {
                0
            }
        };
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

        let is_dired = options.get_flag(ls_flags::LS_DIRED);
        if is_dired && format != LsFormat::Long {
            return Err(Box::new(LsError::LsConflictingArgumentDired));
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
            is_alloc_size: options.get_flag(ls_flags::size::LS_ALLOCATION_SIZE),
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
        })
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    ct_main(args).map(|_| ())
}

pub fn ct_main(args: impl ctcore::Args) -> CTResult<(Vec<PathData>, Vec<PathData>)> {
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
    let application_info = LS_ABOUT;
    let usage_description = ct_format_usage(LS_USAGE);

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
                         \tchange time (-t): ctime, status.\n\
                         \tbirth time: birth, creation;",
             )
             .value_name("field")
             .value_parser([
                 "atime", "access", "use", "ctime", "status", "birth", "creation",
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
             .env("LS_TIME_STYLE")
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
        .after_help(LS_AFTER_HELP)
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

        let security_context = if config.is_context {
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
            write!(out, "{}:", hyperlink).unwrap()
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
        // 以长格式ct_format
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
        entries.sort_by(|a, b| a.display_name.cmp(&b.display_name))
    } else if config.sort == LsSort::Version {
        entries.sort_by(|a, b| {
            ct_version_cmp(&a.p_buf.to_string_lossy(), &b.p_buf.to_string_lossy())
                .then(a.p_buf.to_string_lossy().cmp(&b.p_buf.to_string_lossy()))
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
                .then(a.display_name.cmp(&b.display_name))
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
    #[cfg(likeunix)]
    {
        let metadata = file_path.metadata().unwrap();
        let attr = metadata.file_attributes();
        (attr & 0x2) > 0
    }
    #[cfg(not(likeunix))]
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
            .map_or(0, |md| get_block_size(md, ls_config));
    }
    if ls_config.is_dired {
        dired::dired_indent(out)?;
    }
    Ok(format!(
        "total {}{}",
        display_size(total_size, ls_config),
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
        if config.alloc_size {
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
    #[cfg(unix)]
    {
        let raw_blocks = if md.file_type().is_char_device() || md.file_type().is_block_device() {
            0u64
        } else {
            md.blocks() * 512
        };
        match config.size_format {
            LsSizeFormat::Binary | LsSizeFormat::Decimal => raw_blocks,
            LsSizeFormat::Bytes => raw_blocks / config.block_size,
        }
    }
    #[cfg(not(unix))]
    {
        // 无法获取 likeunix 的块大小，只能返回到文件大小
        md.len()
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
        #[cfg(any(not(unix), target_os = "android", target_os = "macos"))]
        // TODO: See how Mac should work here
        let is_acl_set = false;
        #[cfg(all(unix, not(any(target_os = "android", target_os = "macos"))))]
        let is_acl_set = has_acl(item.display_name.as_os_str());
        write!(
            output_display,
            "{}{}{} {}",
            display_permissions(md, true),
            if item.security_context.len() > 1 {
                // GNU `ls` 使用". "字符来表示具有安全上下文的文件、
                // 但不使用其他替代访问方法。
                "."
            } else {
                ""
            },
            if is_acl_set {
                // 如果设置了 acl，我们将在文件权限末尾显示 "+"。
                "+"
            } else {
                ""
            },
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
            format!(" {}", item_name)
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
    write!(output, "{}", output_display)?;

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

#[cfg(all(unix, not(target_os = "redox")))]
fn cached_gid2grp(gid: u32) -> String {
    static GID_CACHE: Lazy<Mutex<HashMap<u32, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

    let mut gid_cache_mutex = GID_CACHE.lock().unwrap();
    gid_cache_mutex
        .entry(gid)
        .or_insert_with(|| ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()))
        .clone()
}

#[cfg(all(unix, not(target_os = "redox")))]
fn display_group(metadata: &Metadata, config: &LsConfig) -> String {
    match config.long.is_numeric_uid_gid {
        true => metadata.gid().to_string(),
        false => cached_gid2grp(metadata.gid()),
    }
}

#[cfg(target_os = "redox")]
fn display_group(metadata: &Metadata, _config: &LsConfig) -> String {
    metadata.gid().to_string()
}

#[cfg(not(unix))]
fn display_uname(_metadata: &Metadata, _config: &LsConfig) -> String {
    "somebody".to_string()
}

#[cfg(not(unix))]
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
            //According to GNU a Gregorian year has 365.2425 * 24 * 60 * 60 == 31556952 seconds on the average.
            let recent = time + chrono::TimeDelta::try_seconds(31_556_952 / 2).unwrap()
                > chrono::Local::now();

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
// 人可读的ct_format使用幂来表示1024，但不显示 "i"。
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
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "android",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "illumos",
        target_os = "solaris"
    ))]
    {
        let ft = mdata.file_type();
        if ft.is_char_device() || ft.is_block_device() {
            // 这里需要进行类型转换，因为不同操作系统的 `dev_t` 类型各不相同。
            let dev = mdata.rdev() as dev_t;
            let major = unsafe { major(dev) };
            let minor = unsafe { minor(dev) };
            return SizeOrDeviceId::Device(major.to_string(), minor.to_string());
        }
    }
    let len_adjusted = {
        let d = mdata.len() / config.file_size_block_size;
        let r = mdata.len() % config.file_size_block_size;
        if r == 0 {
            d
        } else {
            d + 1
        }
    };
    SizeOrDeviceId::Size(display_size(len_adjusted, config))
}

fn display_size(size: u64, config: &LsConfig) -> String {
    // 注意：人类可读的行为与 GNU ls 不同。
    // GNU ls 默认使用二进制前缀。
    match config.size_format {
        LsSizeFormat::Binary => format_prefixed(&NumberPrefix::binary(size as f64)),
        LsSizeFormat::Decimal => format_prefixed(&NumberPrefix::decimal(size as f64)),
        LsSizeFormat::Bytes => size.to_string(),
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
/// 如果指定了 `config.color` 则负责给符号链接目标名称着色。
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

    #[cfg(not(target_os = "likeunix"))]
    let unencoded_chars = "_-.:~/";
    #[cfg(target_os = "likeunix")]
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

    #[cfg(test)]
    mod base_functons_tests {
        use std::fs;
        use std::fs::File;
        use std::io::ErrorKind::PermissionDenied;
        use std::io::{BufRead, Cursor};
        use std::os::unix::fs::MetadataExt;

        use chrono::DateTime;
        use chrono::Local;
        use tempfile::NamedTempFile;
        use tempfile::{tempdir, TempDir};
        use users::get_user_by_uid;

        use super::*;
        use ctcore::ct_quoting_style::CtQuotes;
        use number_prefix::Prefix;
        use std::os::unix::fs::PermissionsExt;

        // 生成默认config测试
        fn setup_default_config() -> LsConfig {
            LsConfig {
                format: LsFormat::Columns,
                files: LsFiles::LsNormal,
                sort: LsSort::Name,
                is_recursive: true,
                is_reverse: false,
                dereference: LsDereference::LsNone,
                ignore_patterns: Vec::new(),
                size_format: LsSizeFormat::Decimal,
                is_directory: false,
                time: LsTime::LsAccess,
                is_inode: false,
                color: None,
                long: LsLongFormat {
                    is_author: true,
                    is_group: true,
                    is_owner: true,
                    is_numeric_uid_gid: true,
                },
                is_alloc_size: false,
                file_size_block_size: 512,
                block_size: 4096,
                width: 80,
                quoting_style: CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: true,
                },
                indicator_style: LsIndicatorStyle::None,
                time_style: LsTimeStyle::LsFullIso,
                is_context: false,
                is_selinux_supported: false,
                is_group_directories_first: false,
                line_ending: CtLineEnding::Newline,
                is_dired: true,
                is_hyperlink: false,
            }
        }

        fn default_pathdata() -> PathData {
            default_path_data_by_file_name("testfile.tmp")
        }

        fn default_padding() -> LsPaddingCollection {
            LsPaddingCollection {
                inode: 1,
                link_count: 1,
                uname: 1,
                group: 1,
                context: 1,
                size: 1,
                major: 1,
                minor: 1,
                block_size: 1,
            }
        }

        fn default_path_data_by_file_name(file_name: &str) -> PathData {
            PathData {
                md: OnceCell::new(),
                ft: OnceCell::new(),
                de: None,
                display_name: OsString::from(file_name), // 明确指定扩展名以匹配颜色规则
                p_buf: PathBuf::from(file_name),
                is_must_dereference: false,
                security_context: String::new(),
                is_command_line: false,
            }
        }

        fn default_ls_colors() -> LsColors {
            LsColors::from_string("*.tmp=01;32")
        }

        #[test]
        fn test_parse_width() {
            assert_eq!(parse_width("10").unwrap(), 10);
            assert_eq!(parse_width("070").unwrap(), 56);
            assert!(parse_width("abc").is_err());
            assert_eq!(parse_width("10000").unwrap(), 10000);
            assert!(parse_width("-5").is_err());
        }

        #[test]
        fn test_parse_width_valid_input() {
            assert_eq!(parse_width("10").unwrap(), 10);
            assert_eq!(parse_width("010").unwrap(), 8);
            assert_eq!(parse_width("1000").unwrap(), 1000);
        }

        #[test]
        fn test_parse_width_overflow() {
            assert_eq!(parse_width("65536").unwrap(), 65535);
            assert_eq!(parse_width("65537").unwrap(), 65535);
            assert_eq!(parse_width("065536").unwrap(), 27486);
            assert_eq!(parse_width("0265537").unwrap(), 65535);
        }

        #[test]
        fn test_format_prefixed_high_precision_small_numbers() {
            let number = NumberPrefix::Prefixed(Prefix::Kilo, 0.0001234);
            assert_eq!(format_prefixed(&number), "0.1k");
        }

        #[test]
        fn test_format_prefixed_very_large_numbers() {
            let number = NumberPrefix::Prefixed(Prefix::Tera, 123456789.0);
            assert_eq!(format_prefixed(&number), "123456789T");
        }

        #[test]
        fn test_format_prefixed_negative_values() {
            let number = NumberPrefix::Prefixed(Prefix::Mega, -3.456);
            assert_eq!(format_prefixed(&number), "-3.4M");
        }

        #[test]
        fn test_format_prefixed_zero_values() {
            let number = NumberPrefix::Prefixed(Prefix::Giga, 0.0);
            assert_eq!(format_prefixed(&number), "0.0G");
        }

        #[test]
        fn test_format_prefixed_non_standard_prefixes() {
            let number = NumberPrefix::Prefixed(Prefix::Exbi, 5.555);
            // Expect the binary prefix 'Exbi' to remove 'i' and round properly.
            assert_eq!(format_prefixed(&number), "5.6E");
        }

        #[test]
        fn test_format_prefixed_standalone_large_value() {
            let number = NumberPrefix::Standalone(1000000.0);
            assert_eq!(format_prefixed(&number), "1000000");
        }

        #[test]
        fn test_format_prefixed_standalone() {
            let num1 = NumberPrefix::Standalone(1024.0);
            assert_eq!(format_prefixed(&num1), "1024");
        }

        #[test]
        fn test_format_prefixed_prefixed_decimal() {
            let num2 = NumberPrefix::Prefixed(Prefix::Kilo, 1024.0);
            assert_eq!(format_prefixed(&num2), "1024k");
        }

        #[test]
        fn test_format_prefixed_prefixed_decimal_rounded_up() {
            let num3 = NumberPrefix::Prefixed(Prefix::Mega, 981.0);
            assert_eq!(format_prefixed(&num3), "981M");
        }

        #[test]
        fn test_format_prefixed_standalone_small() {
            let num4 = NumberPrefix::Standalone(0.01);
            assert_eq!(format_prefixed(&num4), "0.01");
        }

        #[test]
        fn test_format_prefixed_prefixed_small_decimal() {
            let num5 = NumberPrefix::Prefixed(Prefix::Kilo, 0.01);
            assert_eq!(format_prefixed(&num5), "0.1k");
        }

        #[test]
        fn test_format_prefixed_prefixed_no_decimal() {
            let num6 = NumberPrefix::Prefixed(Prefix::Giga, 1000.0);
            assert_eq!(format_prefixed(&num6), "1000G");
        }

        #[test]
        fn test_format_prefixed_kilo_base() {
            let num7 = NumberPrefix::Prefixed(Prefix::Kilo, 1024.0);
            assert_eq!(format_prefixed(&num7), "1024k");
        }

        #[test]
        fn test_format_prefixed_small_decimal() {
            let num8 = NumberPrefix::Prefixed(Prefix::Kilo, 0.01);
            assert_eq!(format_prefixed(&num8), "0.1k");
        }

        #[test]
        fn test_format_prefixed_no_decimal() {
            let num9 = NumberPrefix::Prefixed(Prefix::Giga, 1024.0);
            assert_eq!(format_prefixed(&num9), "1024G");
        }

        #[test]
        fn test_format_prefixed_tera_prefix() {
            let num10 = NumberPrefix::Prefixed(Prefix::Tera, 1024.0);
            assert_eq!(format_prefixed(&num10), "1024T");
        }

        #[test]
        fn test_format_prefixed_exa_prefix() {
            let num11 = NumberPrefix::Prefixed(Prefix::Exa, 1024.0);
            assert_eq!(format_prefixed(&num11), "1024E");
        }

        #[test]
        fn test_format_prefixed_zebi_prefix() {
            let num12 = NumberPrefix::Prefixed(Prefix::Zebi, 1024.0);
            assert_eq!(format_prefixed(&num12), "1024Z");
        }

        #[test]
        fn test_format_prefixed_yobi_prefix() {
            let num13 = NumberPrefix::Prefixed(Prefix::Yobi, 1024.0);
            assert_eq!(format_prefixed(&num13), "1024Y");
        }

        #[test]
        fn test_format_prefixed_standalone2() {
            let number = NumberPrefix::Standalone(123.456);
            assert_eq!(format_prefixed(&number), "123.456");
        }

        #[test]
        fn test_format_prefixed_rounding_up() {
            let number = NumberPrefix::Prefixed(Prefix::Kilo, 9.999);
            assert_eq!(format_prefixed(&number), "10k");

            let number = NumberPrefix::Prefixed(Prefix::Mega, 9.95);
            assert_eq!(format_prefixed(&number), "10M");
        }

        #[test]
        fn test_format_prefixed_rounding_normal() {
            let number = NumberPrefix::Prefixed(Prefix::Giga, 9.949);
            assert_eq!(format_prefixed(&number), "10G");

            let number = NumberPrefix::Prefixed(Prefix::Giga, 1.234);
            assert_eq!(format_prefixed(&number), "1.3G");
        }

        #[test]
        fn test_format_prefixed_with_trailing_zero() {
            let number = NumberPrefix::Prefixed(Prefix::Kilo, 5.0);
            assert_eq!(format_prefixed(&number), "5.0k");
        }

        #[test]
        fn test_get_time_valid_time_change() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let tmp_file = File::create(&file_path).unwrap();

            let metadata = tmp_file.metadata().expect("获取文件元数据失败");

            let mut config = setup_default_config();
            config.time = LsTime::LsChange;
            let result = get_time(&metadata, &config);

            assert_eq!(
                result,
                Some(DateTime::<Local>::from(metadata.modified().unwrap())),
                "测试用例 1失败：有效的配置"
            );
        }

        #[test]
        fn test_classify_file_directory() {
            let dir = tempdir().unwrap();
            let path = dir.path();

            let config = setup_default_config();
            let p_buf = PathBuf::from(&path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            let mut out = Vec::new();

            let result = classify_file(&path_data, &mut out);
            assert_eq!(result, Some('/'));
        }

        #[test]
        fn test_classify_file_symlink() {
            let dir = tempdir().unwrap();
            let target_path = dir.path().join("target_file");
            let symlink_path = dir.path().join("symlink");

            fs::create_dir_all(&target_path).unwrap();
            std::os::unix::fs::symlink(&target_path, &symlink_path).unwrap();
            let config = setup_default_config();
            let p_buf = PathBuf::from(&symlink_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            let mut out = Vec::new();

            let result = classify_file(&path_data, &mut out);
            assert_eq!(result, Some('@'));
        }

        #[test]
        fn test_classify_file_socket() {
            // This test is skipped on non-Unix platforms as sockets are Unix-specific
            if cfg!(not(unix)) {
                return;
            }
            let dir = tempdir().unwrap();
            let socket_path = dir.path().join("socket_file");
            let _ = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
            let mut out = Vec::new();

            let config = setup_default_config();
            let p_buf = PathBuf::from(&socket_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);
            let result = classify_file(&path_data, &mut out);
            assert_eq!(result, Some('='));
        }

        #[test]
        fn test_classify_file_executable() {
            let dir = tempdir().unwrap();
            let exec_path = dir.path().join("exec_file");
            File::create(&exec_path)
                .unwrap()
                .write_all(b"test file content")
                .unwrap();
            fs::set_permissions(exec_path.as_path(), fs::Permissions::from_mode(0o777)).unwrap();

            let mut out = Vec::new();
            let config = setup_default_config();
            let p_buf = exec_path;
            let dir_entry = None;
            let file_name = None;
            let command_line = false;
            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);
            let result = classify_file(&path_data, &mut out);

            assert_eq!(result, Some('*'));
        }

        #[test]
        fn test_classify_file_regular() {
            let dir = tempdir().unwrap();
            let regular_path = dir.path().join("regular_file");

            File::create(&regular_path)
                .unwrap()
                .write_all(b"test file content")
                .unwrap();

            let mut out = Vec::new();

            let config = setup_default_config();
            let p_buf = regular_path;
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);
            let result = classify_file(&path_data, &mut out);
            assert_eq!(result, None);
        }

        #[test]
        fn test_get_time_valid_time_access() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let tmp_file = File::create(&file_path).unwrap();

            let metadata = tmp_file.metadata().expect("获取文件元数据失败");

            let mut config = setup_default_config();
            config.time = LsTime::LsAccess;
            let result = get_time(&metadata, &config);

            assert_eq!(
                result,
                Some(DateTime::<Local>::from(metadata.accessed().unwrap()))
            );
        }

        #[test]
        fn test_get_time_valid_time_modification() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let tmp_file = File::create(&file_path).unwrap();

            let metadata = tmp_file.metadata().expect("获取文件元数据失败");

            let mut config = setup_default_config();
            config.time = LsTime::LsModification;
            let result = get_time(&metadata, &config);

            assert_eq!(
                result,
                Some(DateTime::<Local>::from(metadata.modified().unwrap())),
            );
        }

        #[test]
        fn test_get_time_valid_time_birth() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let tmp_file = File::create(&file_path).unwrap();

            let metadata = tmp_file.metadata().expect("获取文件元数据失败");

            // 使用有效的配置调用函数
            let mut config = setup_default_config();
            config.time = LsTime::LsBirth;
            let result = get_time(&metadata, &config);

            if metadata.created().is_ok() {
                assert!(result.is_some());
            } else {
                assert!(result.is_none());
            }
        }

        #[test]
        fn test_get_time_permission_denied_change() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let tmp_file = File::create(&file_path).unwrap();
            // let file = std::path::from(&file_path);
            // 使当前用户没有读取权限
            fs::set_permissions(&file_path, fs::Permissions::from_mode(0)).expect("设置权限失败");

            let metadata = tmp_file.metadata();
            let mut config = setup_default_config();
            config.time = LsTime::LsChange;
            let result = get_time(&metadata.unwrap(), &config);

            assert!(result.is_some());
        }

        #[test]
        fn test_get_time_permission_denied_access() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let tmp_file = File::create(&file_path).unwrap();
            // let file = std::path::from(&file_path);
            // 使当前用户没有读取权限
            fs::set_permissions(&file_path, fs::Permissions::from_mode(0)).expect("设置权限失败");

            let metadata = tmp_file.metadata();
            let mut config = setup_default_config();
            config.time = LsTime::LsAccess;
            let result = get_time(&metadata.unwrap(), &config);

            assert!(result.is_some());
        }

        #[test]
        fn test_get_time_permission_denied_modification() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let tmp_file = File::create(&file_path).unwrap();
            // let file = std::path::from(&file_path);
            // 使当前用户没有读取权限
            fs::set_permissions(&file_path, fs::Permissions::from_mode(0)).expect("设置权限失败");

            let metadata = tmp_file.metadata();
            let mut config = setup_default_config();
            config.time = LsTime::LsModification;
            let result = get_time(&metadata.unwrap(), &config);

            assert!(result.is_some());
        }

        #[test]
        fn test_get_time_permission_denied_birth() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let tmp_file = File::create(&file_path).unwrap();
            // 使当前用户没有读取权限
            fs::set_permissions(&file_path, fs::Permissions::from_mode(0)).expect("设置权限失败");

            let metadata = tmp_file.metadata().unwrap();
            let matedata2 = metadata.clone();
            let mut config = setup_default_config();
            config.time = LsTime::LsBirth;
            let result = get_time(&metadata, &config);
            if matedata2.created().is_err() {
                assert!(result.is_none());
            } else {
                assert!(result.is_some());
            }
        }

        #[test]
        fn test_get_system_time_modification() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let tmp_file = temp_dir.path().join("test_file");
            fs::File::create(&tmp_file).expect("Failed to create temporary file");
            let metadata = tmp_file.metadata().expect("Failed to get file metadata");
            let mut config = setup_default_config();
            config.time = LsTime::LsModification;
            let result = get_system_time(&metadata, &config);

            assert_eq!(result, metadata.modified().ok(),);
        }

        #[test]
        fn test_get_system_time_change() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let tmp_file = temp_dir.path().join("test_file");
            fs::File::create(&tmp_file).expect("Failed to create temporary file");
            let metadata = tmp_file.metadata().expect("Failed to get file metadata");
            let mut config = setup_default_config();
            config.time = LsTime::LsChange;
            let result = get_system_time(&metadata, &config);

            assert!(result.is_some());
        }

        #[test]
        fn test_get_system_time_access() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let tmp_file = temp_dir.path().join("test_file");
            fs::File::create(&tmp_file).expect("Failed to create temporary file");
            let metadata = tmp_file.metadata().expect("Failed to get file metadata");
            let mut config = setup_default_config();
            config.time = LsTime::LsAccess;
            let result = get_system_time(&metadata, &config);

            assert_eq!(result, metadata.accessed().ok(),);
        }

        #[test]
        fn test_get_system_time_birth() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let tmp_file = temp_dir.path().join("test_file");
            fs::File::create(&tmp_file).expect("Failed to create temporary file");
            let metadata = tmp_file.metadata().expect("Failed to get file metadata");
            let mut config = setup_default_config();
            config.time = LsTime::LsBirth;
            let result = get_system_time(&metadata, &config);

            assert_eq!(result, metadata.created().ok(),);
        }

        #[test]
        fn test_get_system_time_change_error() {
            let tmp_file = PathBuf::from("non_existent_file.txt");
            let metadata = tmp_file.metadata();

            let result = metadata.map(|m| {
                let mut config = setup_default_config();
                config.time = LsTime::LsChange;
                get_system_time(&m, &config)
            });

            assert!(result.is_err(), "测试用例 失败：文件不存在");
        }

        #[test]
        fn test_get_system_time_birth_error() {
            let tmp_file = PathBuf::from("non_existent_file.txt");
            let metadata = tmp_file.metadata();

            let result = metadata.map(|m| {
                let mut config = setup_default_config();
                config.time = LsTime::LsBirth;
                get_system_time(&m, &config)
            });

            assert!(result.is_err(), "测试用例 失败：文件不存在");
        }

        #[test]
        fn test_get_system_time_modification_error() {
            let tmp_file = PathBuf::from("non_existent_file.txt");
            let metadata = tmp_file.metadata();

            let result = metadata.map(|m| {
                let mut config = setup_default_config();
                config.time = LsTime::LsModification;
                get_system_time(&m, &config)
            });

            assert!(result.is_err(), "测试用例 失败：文件不存在");
        }

        #[test]
        fn test_get_system_time_access_error() {
            let tmp_file = PathBuf::from("non_existent_file.txt");
            let metadata = tmp_file.metadata();

            let result = metadata.map(|m| {
                let mut config = setup_default_config();
                config.time = LsTime::LsAccess;
                get_system_time(&m, &config)
            });

            assert!(result.is_err(), "测试用例 失败：文件不存在");
        }

        #[test]
        fn test_cached_gid2grp_with_real_data() {
            let file = File::open("/etc/group").expect("Failed to parse /etc/group");
            let reader = std::io::BufReader::new(file);
            let mut gid_group_pairs = Vec::new();

            for line in reader.lines() {
                let line = line.unwrap();
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() > 2 {
                    if let Ok(gid) = parts[2].parse::<u32>() {
                        let group_name = parts[0].to_string();
                        gid_group_pairs.push((gid, group_name));
                    }
                }
            }

            for (gid, expected_group_name) in gid_group_pairs {
                let group_name = cached_gid2grp(gid);
                assert_eq!(
                    group_name, expected_group_name,
                    "Group name should match for GID {}",
                    gid
                );
            }
        }

        #[test]
        fn test_handles_nonexistent_gid() {
            // let resolver = MockGroupResolver {};
            let gid = 9999; // Assuming 999 does not exist

            let group_name = cached_gid2grp(gid);
            assert_eq!(
                group_name, "9999",
                "Should fallback to returning GID as string for nonexistent GIDs"
            );
        }

        #[test]
        fn test_cached_uid2usr_retrieves_and_caches() {
            use std::io::BufRead; // Ensure BufRead is imported
            let file = File::open("/etc/passwd").unwrap();
            let reader = std::io::BufReader::new(file);
            let mut uid_username_pairs = Vec::new();

            for line in reader.lines() {
                let line = line.unwrap();
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() > 2 {
                    if let Ok(uid) = parts[2].parse::<u32>() {
                        let username = parts[0].to_string();
                        uid_username_pairs.push((uid, username));
                    }
                }
            }

            for (uid, expected_username) in uid_username_pairs {
                let username = cached_uid2usr(uid);
                assert_eq!(
                    username, expected_username,
                    "Username should match for UID {}",
                    uid
                );
            }
        }

        #[test]
        fn test_cached_uid2usr_handles_nonexistent_user() {
            let username = cached_uid2usr(9999);
            assert_eq!(username, "9999"); // Fallback to returning the UID as a string
        }

        #[test]
        fn test_get_inode() {
            let temp_file = NamedTempFile::new().expect("Failed to create temporary file");
            let metadata = temp_file
                .as_file()
                .metadata()
                .expect("Failed to get metadata");
            let inode = get_inode(&metadata);

            assert_eq!(
                inode,
                format!("{}", metadata.ino()),
                "The inode should match the metadata's inode number"
            );
        }

        #[test]
        fn test_get_inode_directory() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let metadata = fs::metadata(temp_dir.path()).expect("Failed to get metadata");

            let inode = get_inode(&metadata);
            assert_eq!(
                inode,
                format!("{}", metadata.ino()),
                "The inode of the directory should be correctly formatted"
            );
        }

        #[test]
        fn test_inode_stability() {
            let temp_file = NamedTempFile::new().expect("Failed to create temporary file");
            let metadata1 = temp_file
                .as_file()
                .metadata()
                .expect("Failed to get metadata first time");

            // Access metadata again to see if it remains consistent
            let metadata2 = temp_file
                .as_file()
                .metadata()
                .expect("Failed to get metadata second time");

            let inode1 = get_inode(&metadata1);
            let inode2 = get_inode(&metadata2);
            assert_eq!(
                inode1, inode2,
                "Inode numbers should remain consistent across multiple metadata accesses"
            );
        }

        #[test]
        fn test_get_inode_symlink() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let target_path = temp_dir.path().join("target");
            let symlink_path = temp_dir.path().join("symlink");

            File::create(&target_path).expect("Failed to create target file");
            std::os::unix::fs::symlink(&target_path, &symlink_path)
                .expect("Failed to create symlink");

            let symlink_metadata =
                fs::symlink_metadata(&symlink_path).expect("Failed to get symlink metadata");

            let inode = get_inode(&symlink_metadata);
            assert_eq!(
                inode,
                format!("{}", symlink_metadata.ino()),
                "Should return the inode of the symlink, not the target"
            );
        }

        #[test]
        fn test_get_inode_error_handling() {
            use std::os::unix::fs::PermissionsExt;

            let temp_file = NamedTempFile::new().expect("Failed to create temporary file");
            let file_path = temp_file.path();

            fs::set_permissions(file_path, std::fs::Permissions::from_mode(0o000))
                .expect("Failed to set permissions");

            match fs::metadata(file_path) {
                Ok(metadata) => {
                    let inode = get_inode(&metadata);
                    assert!(
                        !inode.is_empty(),
                        "Inode should still be formatted correctly"
                    );
                }
                Err(_) => assert!(
                    true,
                    "Expected an error when accessing file metadata with restricted permissions"
                ),
            }
        }

        #[test]
        fn test_display_grid_zero_width() {
            let names = vec![
                Cell {
                    contents: "item1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "item2".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();

            display_grid(names, 0, Direction::TopToBottom, &mut output, false).unwrap();
            let output_str = String::from_utf8(output).unwrap();
            assert_eq!(output_str, "item1  item2\n");
        }

        #[test]
        fn test_display_grid_zero_width_quoted_true() {
            let names = vec![
                Cell {
                    contents: "item1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "item2".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();

            display_grid(names, 0, Direction::TopToBottom, &mut output, true).unwrap();
            let output_str = String::from_utf8(output).unwrap();
            assert_eq!(output_str, "item1  item2\n");
        }

        #[test]
        fn test_display_grid_non_zero_width() {
            let names = vec![
                Cell {
                    contents: "item1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "item2".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();

            display_grid(names, 50, Direction::TopToBottom, &mut output, false).unwrap();
            let output_str = String::from_utf8(output).unwrap();
            // println!("{}",output_str);
            assert!(output_str.contains("item1  item2"));
        }

        #[test]
        fn test_display_grid_non_zero_width_quoted_true() {
            let names = vec![
                Cell {
                    contents: "item1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "item2".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();

            display_grid(names, 50, Direction::TopToBottom, &mut output, true).unwrap();
            let output_str = String::from_utf8(output).unwrap();
            println!("{}", output_str);
            assert!(output_str.contains("item1   item2"));
        }

        #[test]
        fn test_display_grid_single_line() {
            let cells = vec![
                Cell {
                    contents: "File1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File2".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File3".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            let result = display_grid(cells, 0, Direction::LeftToRight, &mut output, false);
            assert!(result.is_ok());
            assert_eq!(String::from_utf8(output).unwrap(), "File1  File2  File3\n");
        }

        #[test]
        fn test_display_grid_single_line_quoted_true() {
            let cells = vec![
                Cell {
                    contents: "File1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File2".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File3".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            let result = display_grid(cells, 0, Direction::LeftToRight, &mut output, true);
            assert!(result.is_ok());
            assert_eq!(String::from_utf8(output).unwrap(), "File1  File2  File3\n");
        }

        #[test]
        fn test_display_grid_left_to_right() {
            let cells = vec![
                Cell {
                    contents: "File1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File2".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File3".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            let result = display_grid(cells, 50, Direction::LeftToRight, &mut output, false);
            assert!(result.is_ok());
            assert!(String::from_utf8(output).unwrap().contains("File1"));
        }

        #[test]
        fn test_display_grid_left_to_right_quoted_true() {
            let cells = vec![
                Cell {
                    contents: "File1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File2".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File3".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            let result = display_grid(cells, 50, Direction::LeftToRight, &mut output, true);
            assert!(result.is_ok());
            assert!(String::from_utf8(output).unwrap().contains("File1"));
        }

        #[test]
        fn test_display_grid_top_to_bottom() {
            let cells = vec![
                Cell {
                    contents: "File1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File2".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File3".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            let result = display_grid(cells, 50, Direction::TopToBottom, &mut output, false);
            assert!(result.is_ok());
            assert!(String::from_utf8(output).unwrap().contains("File1"));
        }

        #[test]
        fn test_display_grid_top_to_bottom_quoted_true() {
            let cells = vec![
                Cell {
                    contents: "File1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File2".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File3".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            let result = display_grid(cells, 50, Direction::TopToBottom, &mut output, true);
            assert!(result.is_ok());
            assert!(String::from_utf8(output).unwrap().contains("File1"));
        }

        #[test]
        fn test_quoted_display_grid() {
            let cells = vec![
                Cell {
                    contents: "File1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: " File2".to_string(),
                    width: 6,
                }, // Already quoted
            ]
            .into_iter();
            let mut output = Vec::new();
            let result = display_grid(cells, 50, Direction::LeftToRight, &mut output, true);
            assert!(result.is_ok());
            let output_str = String::from_utf8(output).unwrap();
            assert!(output_str.contains(" File1"), "File1 should be quoted");
            assert!(
                output_str.contains("  File2"),
                "File2 should be double-spaced if already quoted"
            );
        }

        #[test]
        fn test_display_grid_empty_cells() {
            let cells = vec![
                Cell {
                    contents: "".to_string(),
                    width: 0,
                },
                Cell {
                    contents: "Data".to_string(),
                    width: 4,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            display_grid(cells, 10, Direction::LeftToRight, &mut output, false).unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), "  Data\n");
        }

        #[test]
        fn test_display_grid_special_characters() {
            let cells = vec![
                Cell {
                    contents: "αβγ".to_string(),
                    width: 3,
                },
                Cell {
                    contents: "123".to_string(),
                    width: 3,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            display_grid(cells, 20, Direction::LeftToRight, &mut output, false).unwrap();
            assert!(String::from_utf8(output).unwrap().contains("αβγ  123"));
        }

        #[test]
        fn test_display_grid_max_width_boundary() {
            let cells = vec![
                Cell {
                    contents: "File1".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File2".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "File3".to_string(),
                    width: 5,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            display_grid(cells, 25, Direction::LeftToRight, &mut output, false).unwrap();
            let output_str = String::from_utf8(output).unwrap();
            assert!(
                output_str.contains("File1  File2  File3"),
                "Should fit exactly within the width"
            );
        }

        #[test]
        fn test_display_grid_insufficient_width() {
            let cells = vec![
                Cell {
                    contents: "LongFileName".to_string(),
                    width: 11,
                },
                Cell {
                    contents: "AnotherLongFileName".to_string(),
                    width: 18,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            display_grid(cells, 10, Direction::LeftToRight, &mut output, false).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "LongFileName\nAnotherLongFileName\n"
            );
        }

        #[test]
        fn test_display_grid_mixed_content_lengths() {
            let cells = vec![
                Cell {
                    contents: "Short".to_string(),
                    width: 5,
                },
                Cell {
                    contents: "VeryVeryLongFileName".to_string(),
                    width: 19,
                },
                Cell {
                    contents: "Mid".to_string(),
                    width: 3,
                },
            ]
            .into_iter();
            let mut output = Vec::new();
            display_grid(cells, 50, Direction::LeftToRight, &mut output, false).unwrap();
            let output_str = String::from_utf8(output).unwrap();
            assert!(
                output_str.contains("Short  VeryVeryLongFileName  Mid"),
                "Should handle mixed lengths well"
            );
        }

        #[test]
        fn test_display_item_long_basic_metadata() {
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let item = default_path_data_by_file_name("File1");

            let config = setup_default_config();
            let padding = default_padding();
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                false,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert!(
                output_str.contains("File1"),
                "Output should include the file name"
            );
        }

        #[test]
        fn test_display_item_long_quoted_names() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let item = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            config.quoting_style = CtQuotingStyle::Literal {
                show_control: false,
            };
            let padding = default_padding();
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_long_quoted_names_dired_false() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.is_dired = false;
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_long_quoted_names_long_owner_false() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.long.is_owner = false;
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_long_quoted_names_long_author_false() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.long.is_author = false;
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_long_quoted_names_long_owner_group_false() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.long.is_owner = false;
            config.long.is_group = false;
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_context_true() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.is_context = true;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_hyperlink_true() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.is_hyperlink = true;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_format_long() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.format = LsFormat::Long;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_format_oneline() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.format = LsFormat::OneLine;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_format_across() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.format = LsFormat::Across;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_format_commas() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.format = LsFormat::Commas;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_format_columns() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.format = LsFormat::Columns;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_indicator_style_none() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.indicator_style = LsIndicatorStyle::None;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_indicator_style_slash() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.indicator_style = LsIndicatorStyle::Slash;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_indicator_style_filetype() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.indicator_style = LsIndicatorStyle::FileType;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_indicator_style_classify() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();
            config.indicator_style = LsIndicatorStyle::Classify;

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_long_group_false() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.long.is_group = false;
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_dereference_none() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.dereference = LsDereference::LsNone;
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_dereference_dirargs() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.dereference = LsDereference::LsDirArgs;
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_dereference_all() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.dereference = LsDereference::LsAll;
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_dereference_args() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.dereference = LsDereference::LsArgs;
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_quoting_style_literal_show_control_true() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.quoting_style = CtQuotingStyle::Literal { show_control: true };
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_quoting_style_literal_show_control_false() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.quoting_style = CtQuotingStyle::Literal {
                show_control: false,
            };
            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_quoting_style_c_quotes_none() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.quoting_style = CtQuotingStyle::C {
                quotes: CtQuotes::None,
            };

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_quoting_style_c_quotes_single() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.quoting_style = CtQuotingStyle::C {
                quotes: CtQuotes::Single,
            };

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_quoting_style_c_quotes_double() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.quoting_style = CtQuotingStyle::C {
                quotes: CtQuotes::Double,
            };

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_quoting_style_shell_escape_false_always_quote_false_show_control_false(
        ) {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.quoting_style = CtQuotingStyle::Shell {
                escape: false,
                always_quote: false,
                show_control: false,
            };

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_quoting_style_shell_escape_true_always_quote_false_show_control_false()
        {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.quoting_style = CtQuotingStyle::Shell {
                escape: true,
                always_quote: false,
                show_control: false,
            };

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_quoting_style_shell_escape_false_always_quote_true_show_control_false()
        {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.quoting_style = CtQuotingStyle::Shell {
                escape: false,
                always_quote: true,
                show_control: false,
            };

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        fn test_display_item_quoting_style_shell_escape_false_always_quote_false_show_control_true()
        {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("File1");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_name = file_path.to_str().unwrap();

            let item = default_path_data_by_file_name(file_name);
            let mut writer = Vec::new();
            let mut dired_output = DiredOutput::default();
            let mut style_manager = StyleManager::new();
            let padding = default_padding();
            let mut config = setup_default_config();

            config.quoting_style = CtQuotingStyle::Shell {
                escape: false,
                always_quote: false,
                show_control: true,
            };

            display_item_long(
                &item,
                &padding,
                &config,
                &mut writer,
                &mut dired_output,
                &mut style_manager,
                true,
            )
            .unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            println!("{}", output_str);
            assert!(
                output_str.contains("File1"),
                "Output should include quoted file name"
            );
        }

        #[test]
        #[cfg(unix)] // This ensures the test only runs on Unix platforms
        fn test_unix_regular_file_block_size() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Bytes;
            config.block_size = 1024;

            let block_size = get_block_size(&metadata, &config);
            assert_eq!(block_size, 4); // Simple check for regular files
        }

        #[test]
        #[cfg(unix)] // This ensures the test only runs on Unix platforms
        fn test_unix_regular_file_block_size_bytes_1() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Bytes;
            config.block_size = 1;

            let block_size = get_block_size(&metadata, &config);
            assert_eq!(block_size, 4096); // Simple check for regular files
        }

        #[test]
        #[cfg(unix)] // This ensures the test only runs on Unix platforms
        fn test_unix_regular_file_block_size_binary_1() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Binary;
            config.block_size = 1;

            let block_size = get_block_size(&metadata, &config);
            assert_eq!(block_size, 4096); // Simple check for regular files
        }

        #[test]
        #[cfg(unix)] // This ensures the test only runs on Unix platforms
        fn test_unix_regular_file_block_size_decimal_1() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Decimal;
            config.block_size = 1;

            let block_size = get_block_size(&metadata, &config);
            assert_eq!(block_size, 4096); // Simple check for regular files
        }

        #[test]
        #[cfg(unix)] // This ensures the test only runs on Unix platforms
        fn test_unix_regular_file_block_size_bytes_10240() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Bytes;
            config.block_size = 10240;

            let block_size = get_block_size(&metadata, &config);
            assert_eq!(block_size, 0); // Simple check for regular files
        }

        #[test]
        #[cfg(unix)] // This ensures the test only runs on Unix platforms
        fn test_unix_regular_file_block_size_binary_10240() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Binary;
            config.block_size = 10240;

            let block_size = get_block_size(&metadata, &config);
            assert_eq!(block_size, 4096); // Simple check for regular files
        }

        #[test]
        #[cfg(unix)] // This ensures the test only runs on Unix platforms
        fn test_unix_regular_file_block_size_decimal_10240() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Decimal;
            config.block_size = 10240;

            let block_size = get_block_size(&metadata, &config);
            assert_eq!(block_size, 4096); // Simple check for regular files
        }

        #[test]
        #[cfg(unix)] // This ensures the test only runs on Unix platforms
        fn test_unix_character_device_block_size() {
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Bytes;
            config.block_size = 1024;

            // Simulating a character device file metadata, usually /dev/null
            let metadata = fs::metadata("/dev/null").unwrap();

            let block_size = get_block_size(&metadata, &config);
            assert_eq!(block_size, 0); // Expecting 0 for character devices
        }

        #[test]
        #[cfg(not(unix))] // This ensures the test only runs on non-Unix platforms
        fn test_non_unix_file_block_size() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Bytes;
            config.block_size = 1024;

            let block_size = get_block_size(&metadata, &config);
            assert_eq!(block_size, metadata.len()); // Expecting file length as block size
        }

        #[test]
        #[cfg(unix)]
        fn test_unix_block_device_block_size() {
            let metadata = fs::metadata("/dev/sda").unwrap(); // Common block device, adjust if necessary
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Binary;
            config.block_size = 512;

            let block_size = get_block_size(&metadata, &config);
            // Check if block size calculation respects the binary size format
            assert_eq!(block_size, 0); // Typically block devices will return 0 blocks unless explicitly mounted and used
        }

        #[test]
        #[cfg(unix)]
        fn test_unix_size_format_variations() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let metadata = fs::metadata(&file_path).unwrap();
            let configs = [
                (LsSizeFormat::Binary, 512),
                (LsSizeFormat::Decimal, 1000),
                (LsSizeFormat::Bytes, 4096),
            ];

            let mut config = setup_default_config();
            for (format, block_size) in configs.iter() {
                config.block_size = *block_size;

                let expected_size = if *format == LsSizeFormat::Bytes {
                    4096
                } else {
                    metadata.blocks() * 512 // Assuming 512 bytes per block for simplicity
                };

                let calculated_size = get_block_size(&metadata, &config);
                assert_eq!(
                    calculated_size, expected_size,
                    "Failed for format {:?}",
                    format
                );
            }
        }

        #[test]
        #[cfg(unix)]
        fn test_sparse_file_block_size() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sparsefile");
            let mut file = File::create(&file_path).unwrap();
            file.write_all(&[0; 1024]).unwrap(); // Allocate 1KB
            file.set_len(1024 * 1024).unwrap(); // Set length to 1MB, but actual blocks used are for 1KB

            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            config.size_format = LsSizeFormat::Binary;
            config.block_size = 512;
            let block_size = get_block_size(&metadata, &config);
            // Expect the calculation to account for actual blocks allocated (not file length)
            let expected_blocks = 8; // Assuming 2 blocks allocated for 1KB on a 512 block size filesystem
            let expected_size = expected_blocks * 512;
            assert_eq!(
                block_size, expected_size,
                "Sparse file block size calculation is incorrect"
            );
        }

        #[test]
        fn test_display_additional_leading_info_inode_false() {
            let dir = tempdir().unwrap();
            // Create a file with some data in it
            let file_path = dir.path().join("testaa1.txt");
            let binding = file_path.clone().into_os_string();
            let file_name = binding.to_str().unwrap();
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let path_data = default_path_data_by_file_name(file_name);
            let config = setup_default_config();
            let mut out = Vec::new();
            let item = path_data;
            let padding = LsPaddingCollection {
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
            let result = display_additional_leading_info(&item, &padding, &config, &mut out);
            assert!(
                result.is_ok(),
                "Error returned for inode and alloc_size options: {:?}",
                result.err()
            );
            let result_str = result.unwrap();
            assert_eq!(
                result_str, "",
                "Incorrect output for inode and alloc_size options: {:?}",
                result_str
            );
        }

        #[test]
        fn test_display_additional_leading_info_inode_true() {
            let dir = tempdir().unwrap();
            // Create a file with some data in it
            let file_path = dir.path().join("testaa1.txt");
            let binding = file_path.clone().into_os_string();
            let file_name = binding.to_str().unwrap();
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let path_data = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            config.is_inode = true;
            let mut out = Vec::new();
            let item = path_data;
            let padding = LsPaddingCollection {
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
            let result = display_additional_leading_info(&item, &padding, &config, &mut out);
            assert!(
                result.is_ok(),
                "Error returned for inode and alloc_size options: {:?}",
                result.err()
            );
            let result_str = result.unwrap();
            assert!(result_str.len() > 0);
        }

        #[test]
        fn test_display_additional_leading_info_nofile_inode_true() {
            let mut config = setup_default_config();
            config.is_inode = true;
            let mut out = Vec::new();
            let item = default_pathdata();
            let padding = LsPaddingCollection {
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
            let result = display_additional_leading_info(&item, &padding, &config, &mut out);
            assert!(
                result.is_ok(),
                "Error returned for inode option: {:?}",
                result.err()
            );
            let result_str = result.unwrap();
            assert_eq!(
                result_str, "? ",
                "Incorrect output for inode and alloc_size options: {:?}",
                result_str
            );
        }

        #[test]
        fn test_display_additional_leading_nofile_info_inode() {
            let config = setup_default_config();
            let mut out = Vec::new();
            let item = default_pathdata();
            let padding = LsPaddingCollection {
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
            let result = display_additional_leading_info(&item, &padding, &config, &mut out);
            assert!(
                result.is_ok(),
                "Error returned for inode option: {:?}",
                result.err()
            );
            let result_str = result.unwrap();
            assert_eq!(
                result_str, "",
                "Incorrect output for inode and alloc_size options: {:?}",
                result_str
            );
        }

        #[test]
        fn test_return_total_single_file() {
            // Create a temporary directory for testing
            let dir = tempdir().unwrap();
            // Create a file with some data in it
            let file_path = dir.path().join("test1.txt");
            let binding = file_path.clone().into_os_string();
            let file_name = binding.to_str().unwrap();
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let path_data = default_path_data_by_file_name(file_name);
            let config = setup_default_config();
            // Test with a single file
            let mut out = Vec::new();
            let result = return_total(&[path_data], &config, &mut out);
            // println!("{:?}",out);
            assert!(
                result.is_ok(),
                "Error returned for single file: {:?}",
                result.err()
            );
            let result_str = result.unwrap();
            assert_eq!(
                result_str, "total 4.1k\n",
                "Incorrect total size for single file: {:?}",
                result_str
            );
        }

        #[test]
        fn test_return_total_multiple_files() {
            let dir = tempdir().unwrap();

            // Create a file with some data in it
            let file_path = dir.path().join("test11.txt");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let binding = file_path.clone().into_os_string();
            let file_name = binding.to_str().unwrap();
            let path_data = default_path_data_by_file_name(file_name);

            // Create another file with some data in it
            let file_path2 = dir.path().join("test22.txt");
            let mut file2 = File::create(&file_path2).unwrap();
            writeln!(file2, "Hello again, world!").unwrap();
            let binding2 = file_path2.clone().into_os_string();
            let file_name2 = binding2.to_str().unwrap();
            let path_data2 = default_path_data_by_file_name(file_name2);
            let config = setup_default_config();

            // Test with multiple files
            let mut out = Vec::new();
            let result = return_total(&[path_data, path_data2], &config, &mut out);
            assert!(
                result.is_ok(),
                "Error returned for single file: {:?}",
                result.err()
            );
            // println!("{:?}",out);
            let result_str = result.unwrap();
            assert_eq!(
                result_str, "total 8.2k\n",
                "Incorrect total size for single file: {:?}",
                result_str
            );
        }

        #[test]
        fn test_return_total_with_hidden() {
            // Create a temporary directory for testing
            let dir = tempdir().unwrap();

            // Create a file with some data in it
            let file_path = dir.path().join(".test333.txt");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let binding = file_path.clone().into_os_string();
            let file_name = binding.to_str().unwrap();
            let path_data = default_path_data_by_file_name(file_name);
            let config = setup_default_config();

            // Test with dired mode enabled
            let mut out = Vec::new();
            let result = return_total(&[path_data], &config, &mut out);
            assert!(
                result.is_ok(),
                "Error returned for dired mode: {:?}",
                result.err()
            );
            let result_str = result.unwrap();
            assert_eq!(
                result_str, "total 4.1k\n",
                "Incorrect output for dired mode: {:?}",
                result_str
            );
        }

        #[test]
        fn test_pad_left() {
            // Test with a small input string and small padding count
            assert_eq!(pad_left("abc", 5), "  abc");

            // Test with a small input string and large padding count
            assert_eq!(pad_left("abc", 10), "       abc");

            // Test with a large input string and small padding count
            assert_eq!(pad_left("abcdefghijklm", 5), "abcdefghijklm");

            // Test with a large input string and large padding count
            assert_eq!(pad_left("abcdefghijklm", 20), "       abcdefghijklm");

            // Test with an empty input string and small padding count
            assert_eq!(pad_left("", 5), "     ");

            // Test with an empty input string and large padding count
            assert_eq!(pad_left("", 20), "                    ");
        }

        #[test]
        fn test_pad_left_with_unicode_characters() {
            // Test with a small input string and small padding count
            assert_eq!(pad_left("中文", 5), "   中文");

            // Test with a small input string and large padding count
            assert_eq!(pad_left("中文", 10), "        中文");

            // Test with a large input string and small padding count
            assert_eq!(pad_left("中文語文言", 5), "中文語文言");

            // Test with a large input string and large padding count
            assert_eq!(pad_left("中文語文言", 20), "               中文語文言");

            // Test with an empty input string and small padding count
            assert_eq!(pad_left("", 5), "     ");

            // Test with an empty input string and large padding count
            assert_eq!(pad_left("", 20), "                    ");
        }

        #[test]
        fn test_pad_right() {
            // Test with a small input string and small padding count
            assert_eq!(pad_right("abc", 5), "abc  ");

            // Test with a small input string and large padding count
            assert_eq!(pad_right("abc", 10), "abc       ");

            // Test with a large input string and small padding count
            assert_eq!(pad_right("abcdefghijklm", 5), "abcdefghijklm");

            // Test with a large input string and large padding count
            assert_eq!(pad_right("abcdefghijklm", 20), "abcdefghijklm       ");

            // Test with an empty input string and small padding count
            assert_eq!(pad_right("", 5), "     ");

            // Test with an empty input string and large padding count
            assert_eq!(pad_right("", 20), "                    ");
        }

        #[test]
        fn test_pad_right_with_russian_characters() {
            // Test with a small input string and small padding count
            assert_eq!(pad_right("роза", 5), "роза ");

            // Test with a small input string and large padding count
            assert_eq!(pad_right("роза", 10), "роза      ");

            // Test with a large input string and small padding count
            assert_eq!(pad_right("розавыпуклость", 5), "розавыпуклость");

            // Test with a large input string and large padding count
            assert_eq!(pad_right("розавыпуклость", 20), "розавыпуклость      ");

            // Test with an empty input string and small padding count
            assert_eq!(pad_right("", 5), "     ");

            // Test with an empty input string and large padding count
            assert_eq!(pad_right("", 20), "                    ");
        }

        #[test]
        fn test_display_dir_entry_size_null() {
            let path = default_path_data_by_file_name("testfile.tmp.null");

            let config = setup_default_config();
            let mut output = Vec::new();

            let sizes = display_dir_entry_size(&path, &config, &mut output);
            assert_eq!(sizes, (0, 0, 0, 0, 0, 0));
        }

        #[test]
        fn test_display_dir_entry_size() {
            let path = default_pathdata();
            let mut file = File::create("testfile.tmp").unwrap();
            file.write_all(b"testfile.tmp!").unwrap();
            let config = setup_default_config();
            let mut output = Vec::new();

            let sizes = display_dir_entry_size(&path, &config, &mut output);

            assert_eq!(sizes, (1, 1, 1, 1, 0, 0));
            // 删除文件
            fs::remove_file("testfile.tmp").unwrap();
        }

        #[test]
        fn test_get_metadata_with_deref_opt() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");

            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"Hello, world!").unwrap();

            let metadata_dereferenced = get_metadata_with_deref_opt(&file_path, true).unwrap();
            let metadata_not_dereferenced = get_metadata_with_deref_opt(&file_path, false).unwrap();

            assert_eq!(metadata_dereferenced.len(), metadata_not_dereferenced.len());
        }

        #[test]
        fn test_get_metadata_with_deref_opt_handles_symlinks() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");

            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"Hello, world!").unwrap();

            let symlink_path = dir.path().join("test_file_symlink.txt");
            std::os::unix::fs::symlink(&file_path, &symlink_path).unwrap();

            let metadata_dereferenced = get_metadata_with_deref_opt(&symlink_path, true).unwrap();
            let _metadata_not_dereferenced =
                get_metadata_with_deref_opt(&symlink_path, false).unwrap();

            assert_eq!(metadata_dereferenced.len(), 13);

            dir.close().unwrap();
        }

        #[test]
        fn test_get_metadata_with_deref_opt_handles_missing_files() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("missing_file.txt");

            let result = get_metadata_with_deref_opt(&file_path, true);
            assert!(result.is_err());

            let result = get_metadata_with_deref_opt(&file_path, false);
            assert!(result.is_err());

            dir.close().unwrap();
        }

        #[test]
        fn test_get_metadata_with_deref_opt_handles_permission_denied() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");

            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"Hello, world!").unwrap();

            let file_permissions = file_path.metadata().unwrap().permissions();
            let mut new_permissions = file_permissions.clone();
            new_permissions.set_readonly(true);
            fs::set_permissions(&file_path, new_permissions).unwrap();

            let result = get_metadata_with_deref_opt(&file_path, true);
            assert!(result.is_ok());

            let result = get_metadata_with_deref_opt(&file_path, false);
            assert!(result.is_ok());

            dir.close().unwrap();
        }

        #[test]
        fn test_single_file_listing() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile.txt");
            File::create(&file_path).unwrap();

            let read_dir = fs::read_dir(dir.path()).unwrap();
            let mut config = setup_default_config();
            config.files = LsFiles::LsAll;

            let mut output = Vec::new();
            let mut listed_ancestors = HashSet::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            let path_data = default_pathdata();

            enter_directory(
                &path_data,
                read_dir,
                &config,
                &mut output,
                &mut listed_ancestors,
                &mut dired,
                &mut style_manager,
            )
            .unwrap();

            let output_str = String::from_utf8(output).unwrap();
            // println!("{:?}", output_str);
            assert!(output_str.contains("testfile.txt"));
        }

        #[test]
        fn test_empty_directory() {
            let dir = tempdir().unwrap();
            let read_dir = fs::read_dir(dir.path()).unwrap();

            let config = setup_default_config();
            let mut output = Vec::new();
            let mut listed_ancestors = HashSet::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            let path_data = default_pathdata();

            enter_directory(
                &path_data,
                read_dir,
                &config,
                &mut output,
                &mut listed_ancestors,
                &mut dired,
                &mut style_manager,
            )
            .unwrap();

            let output_str = String::from_utf8(output).unwrap();
            assert!(
                output_str.is_empty(),
                "Output should be empty for an empty directory"
            );
        }

        #[test]
        fn test_hidden_files() {
            let dir = tempdir().unwrap();
            let hidden_file_path = dir.path().join(".hiddenfile");
            File::create(&hidden_file_path).unwrap();

            let read_dir = fs::read_dir(dir.path()).unwrap();
            let mut config = setup_default_config();
            config.files = LsFiles::LsNormal; // Adjust to not show hidden files

            let mut output = Vec::new();
            let mut listed_ancestors = HashSet::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            let path_data = default_pathdata();

            enter_directory(
                &path_data,
                read_dir,
                &config,
                &mut output,
                &mut listed_ancestors,
                &mut dired,
                &mut style_manager,
            )
            .unwrap();

            let output_str = String::from_utf8(output).unwrap();
            assert!(
                !output_str.contains(".hiddenfile"),
                "Hidden files should not be listed with Files::Normal"
            );
        }

        #[test]
        fn test_recursive_listing() {
            let dir = tempdir().unwrap();
            let sub_dir1 = dir.path().join("subdir1");
            let sub_dir2 = dir.path().join("subdir2");
            fs::create_dir(&sub_dir1).unwrap();
            fs::create_dir(&sub_dir2).unwrap();

            let file1 = sub_dir1.join("file1.txt");
            let file2 = sub_dir1.join("file2.txt");
            let file3 = sub_dir2.join("file3.txt");
            File::create(&file1).unwrap();
            File::create(&file2).unwrap();
            File::create(&file3).unwrap();

            let read_dir = fs::read_dir(dir.path()).unwrap();
            let mut config = setup_default_config();
            config.is_recursive = true; // Enable recursive listing

            let mut output = Vec::new();
            let mut listed_ancestors = HashSet::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            let path_data = default_pathdata();

            enter_directory(
                &path_data,
                read_dir,
                &config,
                &mut output,
                &mut listed_ancestors,
                &mut dired,
                &mut style_manager,
            )
            .unwrap();

            let output_str = String::from_utf8(output).unwrap();
            // println!("{}", output_str);
            assert!(
                output_str.contains("subdir1:"),
                "Output should include 'subdir1/'"
            );
            assert!(
                output_str.contains("subdir2:"),
                "Output should include 'subdir2/'"
            );
            assert!(
                output_str.contains("file1.txt"),
                "Output should include 'file1.txt'"
            );
            assert!(
                output_str.contains("file2.txt"),
                "Output should include 'file2.txt'"
            );
            assert!(
                output_str.contains("file3.txt"),
                "Output should include 'file3.txt'"
            );
        }

        #[test]
        fn test_ignore_pattern() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("ignore_this.txt");
            File::create(&file_path).unwrap();

            let read_dir = fs::read_dir(dir.path()).unwrap();
            let mut config = setup_default_config();
            config
                .ignore_patterns
                .push(Pattern::new("ignore_this*").unwrap()); // Assuming glob patterns

            let mut output = Vec::new();
            let mut listed_ancestors = HashSet::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            let path_data = default_pathdata();

            enter_directory(
                &path_data,
                read_dir,
                &config,
                &mut output,
                &mut listed_ancestors,
                &mut dired,
                &mut style_manager,
            )
            .unwrap();

            let output_str = String::from_utf8(output).unwrap();
            assert!(
                !output_str.contains("ignore_this.txt"),
                "Files matching ignore patterns should not be listed"
            );
        }

        #[test]
        fn test_directory_read_permissions() {
            let dir = tempdir().unwrap();
            let inaccessible_path = dir.path().join("inaccessible_dir");
            fs::create_dir(&inaccessible_path).unwrap();
            let _ = fs::set_permissions(
                &inaccessible_path,
                std::os::unix::fs::PermissionsExt::from_mode(0o000),
            ); // Make directory unreadable

            let read_dir = fs::read_dir(dir.path()).unwrap();
            let config = setup_default_config();

            let mut output = Vec::new();
            let mut listed_ancestors = HashSet::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            let path_data = default_pathdata();

            let result = enter_directory(
                &path_data,
                read_dir,
                &config,
                &mut output,
                &mut listed_ancestors,
                &mut dired,
                &mut style_manager,
            );

            assert!(result.is_ok());
        }

        #[test]
        fn test_should_display_hidden_file() {
            let dir = tempdir().unwrap();
            let hidden_file_path = dir.path().join(".hiddenfile");
            File::create(&hidden_file_path).unwrap();

            let hidden_entry = fs::read_dir(dir.path())
                .unwrap()
                .find(|e| e.as_ref().unwrap().path() == hidden_file_path)
                .unwrap()
                .unwrap();
            let mut config = setup_default_config();
            config.files = LsFiles::LsNormal;
            config.ignore_patterns = vec![];

            assert!(!should_display(&hidden_entry, &config));
        }

        #[test]
        fn test_should_display_with_ignore_pattern() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("ignoreme.txt");
            File::create(&file_path).unwrap();

            let entry = fs::read_dir(dir.path())
                .unwrap()
                .find(|e| e.as_ref().unwrap().path() == file_path)
                .unwrap()
                .unwrap();
            let mut config = setup_default_config();
            config.files = LsFiles::LsNormal;
            config.ignore_patterns = vec![Pattern::new("ignore*").unwrap()];

            assert!(!should_display(&entry, &config));
        }

        #[test]
        fn test_should_display_visible_file_no_ignore() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("visiblefile.txt");
            File::create(&file_path).unwrap();

            let entry = fs::read_dir(dir.path())
                .unwrap()
                .find(|e| e.as_ref().unwrap().path() == file_path)
                .unwrap()
                .unwrap();
            let mut config = setup_default_config();
            config.files = LsFiles::LsNormal;
            config.ignore_patterns = vec![];

            assert!(should_display(&entry, &config));
        }

        #[test]
        fn test_should_display_all_files() {
            let dir = tempdir().unwrap();
            let hidden_file_path = dir.path().join(".alwaysvisible");
            File::create(&hidden_file_path).unwrap();

            let hidden_entry = fs::read_dir(dir.path())
                .unwrap()
                .find(|e| e.as_ref().unwrap().path() == hidden_file_path)
                .unwrap()
                .unwrap();
            let mut config = setup_default_config();
            config.files = LsFiles::LsAll;
            config.ignore_patterns = vec![];

            assert!(
                should_display(&hidden_entry, &config),
                "Hidden files should be displayed with Files::All config"
            );
        }

        #[test]
        fn test_should_display_with_multiple_ignore_patterns() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("ignore_this_file.txt");
            File::create(&file_path).unwrap();

            let entry = fs::read_dir(dir.path())
                .unwrap()
                .find(|e| e.as_ref().unwrap().path() == file_path)
                .unwrap()
                .unwrap();
            let mut config = setup_default_config();
            config.files = LsFiles::LsNormal;
            config.ignore_patterns = vec![
                Pattern::new("ignore*").unwrap(),
                Pattern::new("*.txt").unwrap(),
            ];

            assert!(
                !should_display(&entry, &config),
                "File should not be displayed due to matching ignore pattern"
            );
        }

        #[test]
        fn test_case_sensitivity() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("CaseSensitive.TXT");
            File::create(&file_path).unwrap();

            let entry = fs::read_dir(dir.path())
                .unwrap()
                .find(|e| e.as_ref().unwrap().path() == file_path)
                .unwrap()
                .unwrap();
            let mut config = setup_default_config();
            config.files = LsFiles::LsNormal;
            config.ignore_patterns = vec![Pattern::new("*.txt").unwrap()];

            assert!(
                should_display(&entry, &config),
                "File should be displayed because the ignore pattern is case sensitive"
            );
        }

        #[test]
        fn test_non_ascii_filenames() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("файл.txt");
            File::create(&file_path).unwrap();

            let entry = fs::read_dir(dir.path())
                .unwrap()
                .find(|e| e.as_ref().unwrap().path() == file_path)
                .unwrap()
                .unwrap();
            let mut config = setup_default_config();
            config.files = LsFiles::LsNormal;
            config.ignore_patterns = vec![];

            assert!(
                should_display(&entry, &config),
                "Non-ASCII filenames should be displayed normally"
            );
        }

        #[test]
        fn test_is_hidden_in_unix() {
            let dir = tempdir().unwrap();
            let hidden_file_path = dir.path().join(".test_file.txt");
            File::create(&hidden_file_path).unwrap();

            let hidden_dir_entry = fs::read_dir(dir.path())
                .unwrap()
                .find(|e| e.as_ref().unwrap().path() == hidden_file_path)
                .unwrap()
                .unwrap();
            assert!(is_hidden(&hidden_dir_entry));
        }

        #[test]
        fn test_is_no_hidden_in_unix() {
            let dir = tempdir().unwrap();
            let visible_file_path = dir.path().join("test_file.txt");
            File::create(&visible_file_path).unwrap();

            let visible_dir_entry = fs::read_dir(dir.path())
                .unwrap()
                .find(|e| e.as_ref().unwrap().path() == visible_file_path)
                .unwrap()
                .unwrap();
            assert!(!is_hidden(&visible_dir_entry));
        }

        #[test]
        fn test_list_directory_true_recursive_false_dired_false_hyperlink_false_quoting_style_literal(
        ) {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");
            File::create(&file_path).unwrap();

            let locs = vec![dir.path()];
            let mut config = setup_default_config();
            config.is_directory = true;
            config.is_recursive = false;
            config.is_dired = false;
            config.is_hyperlink = false;
            config.quoting_style = CtQuotingStyle::Literal { show_control: true };

            let result = list(locs, &config);
            assert!(result.is_ok());
        }

        #[test]
        fn test_list_directory_false_recursive_false_dired_false_hyperlink_false_quoting_style_literal(
        ) {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");
            File::create(&file_path).unwrap();

            let locs = vec![dir.path()];
            let mut config = setup_default_config();
            config.is_directory = false;
            config.is_recursive = false;
            config.is_dired = false;
            config.is_hyperlink = false;
            config.quoting_style = CtQuotingStyle::Literal { show_control: true };

            let result = list(locs, &config);
            assert!(result.is_ok());
        }

        #[test]
        fn test_list_directory_true_recursive_true_dired_false_hyperlink_false_quoting_style_literal(
        ) {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");
            File::create(&file_path).unwrap();

            let locs = vec![dir.path()];
            let mut config = setup_default_config();
            config.is_directory = true;
            config.is_recursive = true;
            config.is_dired = false;
            config.is_hyperlink = false;
            config.quoting_style = CtQuotingStyle::Literal { show_control: true };

            let result = list(locs, &config);
            assert!(result.is_ok());
        }

        #[test]
        fn test_list_directory_true_recursive_true_dired_true_hyperlink_false_quoting_style_literal(
        ) {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");
            File::create(&file_path).unwrap();

            let locs = vec![dir.path()];
            let mut config = setup_default_config();
            config.is_directory = true;
            config.is_recursive = true;
            config.is_dired = true;
            config.is_hyperlink = false;
            config.quoting_style = CtQuotingStyle::Literal { show_control: true };

            let result = list(locs, &config);
            assert!(result.is_ok());
        }

        #[test]
        fn test_list_directory_true_recursive_true_dired_true_hyperlink_true_quoting_style_literal()
        {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");
            File::create(&file_path).unwrap();

            let locs = vec![dir.path()];
            let mut config = setup_default_config();
            config.is_directory = true;
            config.is_recursive = true;
            config.is_dired = true;
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::Literal { show_control: true };

            let result = list(locs, &config);
            assert!(result.is_ok());
        }

        #[test]
        fn test_display_items_long_format() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile.txt");
            File::create(&file_path).unwrap();
            let metadata = fs::metadata(&file_path).unwrap();
            let items = vec![PathData {
                md: OnceCell::from(Some(metadata)),
                ft: OnceCell::new(),
                de: None,
                display_name: file_path.as_os_str().to_os_string(),
                p_buf: file_path.clone(),
                is_must_dereference: false,
                security_context: "unconfined_u:unconfined_r:unconfined_t:s0".to_string(),
                is_command_line: true,
            }];

            let config = LsConfig {
                format: LsFormat::Long,
                files: LsFiles::LsAll,
                sort: LsSort::None,
                is_recursive: false,
                is_reverse: false,
                dereference: LsDereference::LsNone,
                ignore_patterns: vec![],
                size_format: LsSizeFormat::Decimal,
                is_directory: false,
                time: LsTime::LsModification,
                is_inode: true,
                color: None,
                long: LsLongFormat {
                    is_author: true,
                    is_group: true,
                    is_owner: true,
                    #[cfg(unix)]
                    is_numeric_uid_gid: true,
                },
                is_alloc_size: false,
                file_size_block_size: 1024,
                block_size: 1024,
                width: 80,
                quoting_style: CtQuotingStyle::Literal { show_control: true },
                indicator_style: LsIndicatorStyle::None,
                time_style: LsTimeStyle::LsIso,
                is_context: true,
                is_selinux_supported: true,
                is_group_directories_first: false,
                line_ending: CtLineEnding::Newline,
                is_dired: false,
                is_hyperlink: false,
            };

            let mut output = Vec::new();
            let mut dired = DiredOutput {
                dired_positions: Vec::new(),
                subdired_positions: Vec::new(),
                padding: 0,
            };
            let mut style_manager = StyleManager {
                current_style: None,
            };

            display_items(&items, &config, &mut output, &mut dired, &mut style_manager).unwrap();

            let output_str = String::from_utf8(output).unwrap();
            assert!(
                output_str.contains("testfile.txt"),
                "The output should include the file name"
            );
            assert!(
                output_str.contains("unconfined_u:unconfined_r:unconfined_t:s0"),
                "The SELinux context should be displayed"
            );
        }

        #[test]
        fn test_display_items_columns_format() {
            let dir = tempdir().unwrap();
            let file1 = dir.path().join("file1.txt");
            let file2 = dir.path().join("file2.txt");
            File::create(&file1).unwrap();
            File::create(&file2).unwrap();

            let metadata1 = fs::metadata(&file1).unwrap();
            let metadata2 = fs::metadata(&file2).unwrap();
            let items = vec![
                PathData {
                    md: OnceCell::from(Some(metadata1)),
                    ft: OnceCell::new(),
                    de: None,
                    display_name: file1.as_os_str().to_os_string(),
                    p_buf: file1,
                    is_must_dereference: false,
                    security_context: String::new(),
                    is_command_line: true,
                },
                PathData {
                    md: OnceCell::from(Some(metadata2)),
                    ft: OnceCell::new(),
                    de: None,
                    display_name: file2.as_os_str().to_os_string(),
                    p_buf: file2,
                    is_must_dereference: false,
                    security_context: String::new(),
                    is_command_line: true,
                },
            ];

            let mut config = setup_default_config();
            config.format = LsFormat::Columns;

            let mut output = Vec::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            display_items(&items, &config, &mut output, &mut dired, &mut style_manager).unwrap();

            let output_str = String::from_utf8(output).unwrap();
            assert!(output_str.contains("file1.txt"));
            assert!(output_str.contains("file2.txt"));
        }

        #[test]
        fn test_display_items_columns_format_commas() {
            let dir = tempdir().unwrap();
            let file1 = dir.path().join("file1.txt");
            let file2 = dir.path().join("file2.txt");
            File::create(&file1).unwrap();
            File::create(&file2).unwrap();

            let metadata1 = fs::metadata(&file1).unwrap();
            let metadata2 = fs::metadata(&file2).unwrap();
            let items = vec![
                PathData {
                    md: OnceCell::from(Some(metadata1)),
                    ft: OnceCell::new(),
                    de: None,
                    display_name: file1.as_os_str().to_os_string(),
                    p_buf: file1,
                    is_must_dereference: false,
                    security_context: String::new(),
                    is_command_line: true,
                },
                PathData {
                    md: OnceCell::from(Some(metadata2)),
                    ft: OnceCell::new(),
                    de: None,
                    display_name: file2.as_os_str().to_os_string(),
                    p_buf: file2,
                    is_must_dereference: false,
                    security_context: String::new(),
                    is_command_line: true,
                },
            ];

            let mut config = setup_default_config();
            config.format = LsFormat::Commas;

            let mut output = Vec::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            display_items(&items, &config, &mut output, &mut dired, &mut style_manager).unwrap();

            let output_str = String::from_utf8(output).unwrap();
            assert!(output_str.contains("file1.txt"));
            assert!(output_str.contains("file2.txt"));
        }

        #[test]
        fn test_display_items_columns_format_long() {
            let dir = tempdir().unwrap();
            let file1 = dir.path().join("file1.txt");
            let file2 = dir.path().join("file2.txt");
            File::create(&file1).unwrap();
            File::create(&file2).unwrap();

            let metadata1 = fs::metadata(&file1).unwrap();
            let metadata2 = fs::metadata(&file2).unwrap();
            let items = vec![
                PathData {
                    md: OnceCell::from(Some(metadata1)),
                    ft: OnceCell::new(),
                    de: None,
                    display_name: file1.as_os_str().to_os_string(),
                    p_buf: file1,
                    is_must_dereference: false,
                    security_context: String::new(),
                    is_command_line: true,
                },
                PathData {
                    md: OnceCell::from(Some(metadata2)),
                    ft: OnceCell::new(),
                    de: None,
                    display_name: file2.as_os_str().to_os_string(),
                    p_buf: file2,
                    is_must_dereference: false,
                    security_context: String::new(),
                    is_command_line: true,
                },
            ];

            let mut config = setup_default_config();
            config.format = LsFormat::Long;

            let mut output = Vec::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            display_items(&items, &config, &mut output, &mut dired, &mut style_manager).unwrap();

            let output_str = String::from_utf8(output).unwrap();
            assert!(output_str.contains("file1.txt"));
            assert!(output_str.contains("file2.txt"));
        }

        #[test]
        fn test_display_items_columns_format_across() {
            let dir = tempdir().unwrap();
            let file1 = dir.path().join("file1.txt");
            let file2 = dir.path().join("file2.txt");
            File::create(&file1).unwrap();
            File::create(&file2).unwrap();

            let metadata1 = fs::metadata(&file1).unwrap();
            let metadata2 = fs::metadata(&file2).unwrap();
            let items = vec![
                PathData {
                    md: OnceCell::from(Some(metadata1)),
                    ft: OnceCell::new(),
                    de: None,
                    display_name: file1.as_os_str().to_os_string(),
                    p_buf: file1,
                    is_must_dereference: false,
                    security_context: String::new(),
                    is_command_line: true,
                },
                PathData {
                    md: OnceCell::from(Some(metadata2)),
                    ft: OnceCell::new(),
                    de: None,
                    display_name: file2.as_os_str().to_os_string(),
                    p_buf: file2,
                    is_must_dereference: false,
                    security_context: String::new(),
                    is_command_line: true,
                },
            ];

            let mut config = setup_default_config();
            config.format = LsFormat::Across;

            let mut output = Vec::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            display_items(&items, &config, &mut output, &mut dired, &mut style_manager).unwrap();

            let output_str = String::from_utf8(output).unwrap();
            assert!(output_str.contains("file1.txt"));
            assert!(output_str.contains("file2.txt"));
        }

        #[test]
        fn test_display_items_with_selinux_and_error() {
            let dir = tempdir().unwrap();
            let file = dir.path().join("errorfile.txt");
            File::create(&file).unwrap();

            // Assuming `get_metadata` can simulate an error, perhaps by setting `md` to `None`.
            let items = vec![PathData {
                md: OnceCell::from(None), // Simulate metadata retrieval failure
                ft: OnceCell::new(),
                de: None,
                display_name: file.as_os_str().to_os_string(),
                p_buf: file,
                is_must_dereference: false,
                security_context: "unconfined_u:unconfined_r:unconfined_t:s0".to_string(),
                is_command_line: true,
            }];

            let mut config = setup_default_config();
            config.is_context = true; // Enable SELinux context display

            let mut output = Vec::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            let result =
                display_items(&items, &config, &mut output, &mut dired, &mut style_manager);

            assert!(result.is_ok());
        }

        #[test]
        fn test_display_items_recursive_directory_listing() {
            let dir = tempdir().unwrap();
            let sub_dir = dir.path().join("subdir");
            fs::create_dir(&sub_dir).unwrap();
            let file_in_subdir = sub_dir.join("subfile.txt");
            File::create(&file_in_subdir).unwrap();

            let items = vec![PathData {
                md: OnceCell::from(Some(fs::metadata(&sub_dir).unwrap())),
                ft: OnceCell::new(),
                de: None,
                display_name: sub_dir.as_os_str().to_os_string(),
                p_buf: sub_dir,
                is_must_dereference: false,
                security_context: String::new(),
                is_command_line: true,
            }];

            let mut config = setup_default_config();
            config.is_recursive = true;

            let mut output = Vec::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            display_items(&items, &config, &mut output, &mut dired, &mut style_manager).unwrap();

            let output_str = String::from_utf8(output).unwrap();
            println!("{}", output_str);
            assert!(output_str.contains("subdir"));
        }

        #[test]
        fn test_display_items_hyperlink_listing() {
            let dir = tempdir().unwrap();
            let sub_dir = dir.path().join("subdir");
            fs::create_dir(&sub_dir).unwrap();
            let file_in_subdir = sub_dir.join("subfile.txt");
            File::create(&file_in_subdir).unwrap();

            let items = vec![PathData {
                md: OnceCell::from(Some(fs::metadata(&sub_dir).unwrap())),
                ft: OnceCell::new(),
                de: None,
                display_name: sub_dir.as_os_str().to_os_string(),
                p_buf: sub_dir,
                is_must_dereference: false,
                security_context: String::new(),
                is_command_line: true,
            }];

            let mut config = setup_default_config();
            config.is_hyperlink = true;

            let mut output = Vec::new();
            let mut dired = DiredOutput::default();
            let mut style_manager = StyleManager::new();

            display_items(&items, &config, &mut output, &mut dired, &mut style_manager).unwrap();

            let output_str = String::from_utf8(output).unwrap();
            println!("{}", output_str);
            assert!(output_str.contains("subdir"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_false() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = false;

            show_dir_name(&path_data, &mut out, &config);
            let expected = "testfile.tmp:";

            assert_eq!(out.into_inner(), expected.as_bytes());
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_literal_true() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::Literal { show_control: true };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_literal_false() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::Literal { show_control: true };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_c_quotes_none() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::C {
                quotes: CtQuotes::None,
            };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_c_quotes_single() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::C {
                quotes: CtQuotes::Single,
            };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_c_quotes_double() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::C {
                quotes: CtQuotes::Double,
            };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_c_quotes_shell_true_true_true() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::Shell {
                escape: true,
                always_quote: true,
                show_control: true,
            };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_c_quotes_shell_true_true_false() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::Shell {
                escape: true,
                always_quote: true,
                show_control: false,
            };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_c_quotes_shell_true_false_false() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::Shell {
                escape: true,
                always_quote: false,
                show_control: false,
            };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_c_quotes_shell_false_false_false() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::Shell {
                escape: false,
                always_quote: false,
                show_control: false,
            };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_c_quotes_shell_false_true_false() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::Shell {
                escape: false,
                always_quote: true,
                show_control: false,
            };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_show_dir_name_hyperlink_true_quoting_style_c_quotes_shell_false_false_true() {
            let mut out = Cursor::new(Vec::new());

            let path_data = default_pathdata();
            let mut config = setup_default_config();
            config.is_hyperlink = true;
            config.quoting_style = CtQuotingStyle::Shell {
                escape: false,
                always_quote: false,
                show_control: true,
            };

            show_dir_name(&path_data, &mut out, &config);
            let result = String::from_utf8(out.into_inner()).expect("Invalid UTF-8");
            assert!(result.contains("testfile.tmp"));
        }

        #[test]
        fn test_file_type_regular_file() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file");
            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"Hello, world!").unwrap();

            let config = setup_default_config();
            let p_buf = PathBuf::from(&file_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            let mut out = Vec::new();
            let file_type = path_data.file_type(&mut out);
            assert!(file_type.is_some());
            // println!("{:?}", file_type);
            // assert_eq!(file_type.unwrap(), &FileType::Regular);
        }

        #[test]
        fn test_file_type_directory() {
            let dir = tempdir().unwrap();
            let dir_path = dir.path();

            let config = setup_default_config();
            let p_buf = PathBuf::from(&dir_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            let mut out = Vec::new();
            let file_type = path_data.file_type(&mut out);
            assert!(file_type.is_some());
            // println!("{:?}", file_type);
            // assert_eq!(file_type.unwrap(), &FileType::Directory);
        }

        #[test]
        fn test_file_type_symlink() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file");
            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"Hello, world!").unwrap();

            let symlink_path = dir.path().join("test_file_symlink");
            std::os::unix::fs::symlink(&file_path, &symlink_path).unwrap();

            let config = setup_default_config();
            let p_buf = PathBuf::from(&symlink_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            let mut out = Vec::new();
            let file_type = path_data.file_type(&mut out);
            assert!(file_type.is_some());
            // println!("{:?}", file_type.unwrap());
            // assert_eq!(file_type.unwrap(), FileType);
        }

        #[test]
        fn test_new_valid_input() {
            let p_buf = PathBuf::from("/tmp");
            let dir_entry = Some(Ok(fs::read_dir("/tmp").unwrap().next().unwrap().unwrap()));
            let file_name = Some(OsString::from("tmp"));
            let mut config = setup_default_config();
            config.dereference = LsDereference::LsAll;
            config.is_context = true;

            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            assert_eq!(path_data.display_name, OsString::from("tmp"));
            assert_eq!(path_data.is_must_dereference, true);
            assert_eq!(path_data.security_context, String::from("?")); // assuming get_security_context returns a string "context_string"
            assert_eq!(path_data.is_command_line, false);
        }

        #[test]
        fn test_get_metadata() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file");
            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"Hello, world!").unwrap();

            let config = setup_default_config();
            let p_buf = PathBuf::from(&file_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            let mut out = Vec::new();
            let metadata = path_data.get_metadata(&mut out);
            assert!(metadata.is_some());
            assert_eq!(metadata.unwrap().len(), file_path.metadata().unwrap().len());

            let file_type = path_data.file_type(&mut out);
            assert!(file_type.is_some());
            assert_eq!(
                file_type.unwrap(),
                &file_path.metadata().unwrap().file_type()
            );
        }

        #[test]
        fn test_get_metadata_symlink() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file");
            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"Hello, world!").unwrap();

            let symlink_path = dir.path().join("test_file_symlink");
            std::os::unix::fs::symlink(&file_path, &symlink_path).unwrap();

            let config = setup_default_config();
            let p_buf = PathBuf::from(&symlink_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            let mut out = Vec::new();
            let metadata = path_data.get_metadata(&mut out);
            assert!(metadata.is_some());
            assert_eq!(metadata.unwrap().len(), 25);

            let file_type = path_data.file_type(&mut out);
            assert!(file_type.is_some());
        }

        #[test]
        fn test_get_metadata_directory() {
            let dir = tempdir().unwrap();
            let dir_path = dir.path();

            let config = setup_default_config();
            let p_buf = PathBuf::from(&dir_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            let mut out = Vec::new();
            let metadata = path_data.get_metadata(&mut out);
            assert!(metadata.is_some());
            assert_eq!(metadata.unwrap().len(), dir_path.metadata().unwrap().len());

            let file_type = path_data.file_type(&mut out);
            assert!(file_type.is_some());
            assert_eq!(
                file_type.unwrap(),
                &dir_path.metadata().unwrap().file_type()
            );
        }

        #[test]
        fn test_get_metadata_nonexistent_path() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("nonexistent_file");

            let config = setup_default_config();
            let p_buf = PathBuf::from(&file_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            let mut out = Vec::new();
            let metadata = path_data.get_metadata(&mut out);
            assert!(metadata.is_none());
        }

        #[test]
        fn test_get_metadata_permission_denied() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file");
            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"Hello, world!").unwrap();

            let config = setup_default_config();
            let p_buf = PathBuf::from(&file_path);
            let dir_entry = None;
            let file_name = None;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            // Change the permissions of the file to be read-only
            fs::set_permissions(
                &file_path,
                std::os::unix::fs::PermissionsExt::from_mode(0o444),
            )
            .unwrap();

            let mut out = Vec::new();
            let metadata = path_data.get_metadata(&mut out);
            // println!("{:?} {}", metadata, std::str::from_utf8(&out).unwrap());
            assert!(metadata.is_some());
        }

        #[test]
        fn test_new_no_file_name() {
            let p_buf = PathBuf::from("/tmp");
            let dir_entry = Some(Ok(fs::read_dir("/tmp").unwrap().next().unwrap().unwrap()));
            let file_name = None;
            let mut config = setup_default_config();
            config.dereference = LsDereference::LsAll;
            config.is_context = true;

            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            assert_eq!(path_data.display_name, OsString::from("tmp"));
            assert_eq!(path_data.is_must_dereference, true);
            assert_eq!(path_data.security_context, String::from("?")); // assuming get_security_context returns a string "context_string"
            assert_eq!(path_data.is_command_line, false);
        }

        #[test]
        fn test_new_command_line_true() {
            let p_buf = PathBuf::from("/tmp");
            let dir_entry = Some(Ok(fs::read_dir("/tmp").unwrap().next().unwrap().unwrap()));
            let file_name = Some(OsString::from("tmp"));
            let mut config = setup_default_config();
            config.dereference = LsDereference::LsAll;
            config.is_context = true;
            let command_line = true;

            let path_data =
                PathData::new(p_buf.clone(), dir_entry, file_name, &config, command_line);

            assert_eq!(path_data.display_name, OsString::from("tmp"));
            assert_eq!(path_data.is_must_dereference, true);
            assert_eq!(path_data.security_context, String::from("?")); // assuming get_security_context returns a string "context_string"
            assert_eq!(path_data.is_command_line, true);
        }

        #[test]
        fn test_new_invalid_input() {
            let p_buf = PathBuf::from("/tmp");
            let dir_entry = Some(Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "File not found",
            )));
            let file_name = Some(OsString::from("tmp"));
            let mut config = setup_default_config();
            config.dereference = LsDereference::LsAll;
            config.is_context = true;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            assert_eq!(path_data.display_name, OsString::from("tmp"));
            assert_eq!(path_data.is_must_dereference, true);
            assert_eq!(path_data.security_context, String::from("?"));
            assert_eq!(path_data.is_command_line, false);
        }

        #[test]
        fn test_new_get_security_context_error() {
            let p_buf = PathBuf::from("/tmp");
            let dir_entry = Some(Ok(fs::read_dir("/tmp").unwrap().next().unwrap().unwrap()));
            let file_name = Some(OsString::from("tmp"));
            let mut config = setup_default_config();
            config.dereference = LsDereference::LsAll;
            config.is_context = true;
            let _command_line = false;
            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            assert_eq!(path_data.display_name, OsString::from("tmp"));
            assert_eq!(path_data.is_must_dereference, true);
            assert_eq!(path_data.security_context, String::from("?"));
            assert_eq!(path_data.is_command_line, false);
        }

        #[test]
        fn test_new_dereference_none() {
            let p_buf = PathBuf::from("/tmp/symlink");
            let dir_entry = Some(Ok(fs::read_dir("/tmp").unwrap().next().unwrap().unwrap()));
            let file_name = Some(OsString::from("symlink"));
            let mut config = setup_default_config();
            config.dereference = LsDereference::LsNone;
            config.is_context = true;

            let command_line = false;

            let path_data = PathData::new(p_buf, dir_entry, file_name, &config, command_line);

            assert_eq!(path_data.display_name, OsString::from("symlink"));
            assert_eq!(path_data.is_must_dereference, false);
            assert_eq!(path_data.security_context, String::from("?"));
            assert_eq!(path_data.is_command_line, false);
        }

        #[test]
        fn test_config_from_base() {
            // Create the ArgMatches for testing
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--indicator-style=none"];
            let matches = command.try_get_matches_from(args).unwrap();
            // Call the `from` method with the test matches
            let config = LsConfig::from(&matches).unwrap();

            if stdout().is_terminal() {
                assert_eq!(config.format, LsFormat::Columns);
            } else {
                assert_eq!(config.format, LsFormat::OneLine);
            }
            assert_eq!(config.files, LsFiles::LsNormal);
            assert_eq!(config.sort, LsSort::Name);
        }

        #[test]
        fn test_config_from_format_one_line() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=single-column"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.format, LsFormat::OneLine);
        }

        #[test]
        fn test_config_from_format_verbose() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=verbose"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.format, LsFormat::Long);
        }

        #[test]
        fn test_config_from_format_vertical() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=vertical"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.format, LsFormat::Columns);
        }

        #[test]
        fn test_config_from_format_horizontal() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=horizontal"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.format, LsFormat::Across);
        }

        #[test]
        fn test_config_from_format_commas() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=commas"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.format, LsFormat::Commas);
        }

        #[test]
        fn test_config_from_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--long"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.format, LsFormat::Long);
        }

        #[test]
        fn test_config_from_x() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-x"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.format, LsFormat::Across);
        }

        #[test]
        fn test_config_from_m() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-m"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.format, LsFormat::Commas);
        }

        #[test]
        fn test_config_from_uppercase_c() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-C"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.format, LsFormat::Columns);
        }

        #[test]
        fn test_config_from_files_normal() {
            // Create the ArgMatches for testing files option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name()];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.files, LsFiles::LsNormal);
        }

        #[test]
        fn test_config_from_files_almost_all() {
            // Create the ArgMatches for testing files option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--almost-all"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.files, LsFiles::LsAlmostAll);
        }

        #[test]
        fn test_config_from_files() {
            let args = vec![ctcore::ct_util_name(), "--all"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.files, LsFiles::LsAll);
        }

        #[test]
        fn test_config_from_sort_name() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=name"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::Name);
        }

        #[test]
        fn test_config_from_sort_none() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=none"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::None);
        }

        #[test]
        fn test_config_from_sort_time() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=time"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::Time);
        }

        #[test]
        fn test_config_from_sort_version() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=version"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::Version);
        }

        #[test]
        fn test_config_from_sort_extension() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=extension"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::Extension);
        }

        #[test]
        fn test_config_from_sort_size() {
            let args = vec![ctcore::ct_util_name(), "--sort=size"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::Size);
        }

        #[test]
        fn test_config_from_sort_width() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=width"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::Width);
        }

        #[test]
        fn test_config_from_sort_t() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-t"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::Time);
        }

        #[test]
        fn test_config_from_sort_uppercase_s() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-S"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::Size);
        }

        #[test]
        fn test_config_from_sort_uppercase_u() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-U"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::None);
        }

        #[test]
        fn test_config_from_sort_uppercase_x() {
            // Create the ArgMatches for testing sort option
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-X"];
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.sort, LsSort::Extension);
        }

        #[test]
        fn test_config_from_recursive_true() {
            let args = vec![ctcore::ct_util_name(), "--recursive"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_recursive);
        }

        #[test]
        fn test_config_from_recursive_false() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_recursive);
        }

        #[test]
        fn test_config_from_reverse_true() {
            let args = vec![ctcore::ct_util_name(), "--reverse"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_reverse);
        }

        #[test]
        fn test_config_from_reverse_false() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_reverse);
        }

        #[cfg(unix)]
        #[test]
        fn test_config_from_inode_true() {
            let args = vec![ctcore::ct_util_name(), "--inode"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_inode);
        }

        #[cfg(unix)]
        #[test]
        fn test_config_from_inode_false() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_inode);
        }

        #[test]
        fn test_config_from_dereference() {
            let args = vec![ctcore::ct_util_name(), "--dereference"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.dereference, LsDereference::LsAll);
        }

        #[test]
        fn test_config_from_dereference_command_line() {
            let args = vec![ctcore::ct_util_name(), "--dereference-command-line"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.dereference, LsDereference::LsArgs);
        }

        #[test]
        fn test_config_from_dereference_command_line_symlink_to_dir() {
            let args = vec![
                ctcore::ct_util_name(),
                "--dereference-command-line-symlink-to-dir",
            ];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.dereference, LsDereference::LsDirArgs);
        }

        #[test]
        fn test_config_from_dereference_directory() {
            let args = vec![ctcore::ct_util_name(), "--directory"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.dereference, LsDereference::LsNone);
        }

        #[test]
        fn test_config_from_dereference_default() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.dereference, LsDereference::LsDirArgs);
        }

        #[test]
        fn test_config_from_size_formathuman_readable() {
            let args = vec![ctcore::ct_util_name(), "--block-size=human-readable"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.size_format, LsSizeFormat::Binary);
        }

        #[test]
        fn test_config_from_size_format_si() {
            let args = vec![ctcore::ct_util_name(), "--block-size=si"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.size_format, LsSizeFormat::Decimal);
        }

        #[test]
        fn test_config_from_si() {
            let args = vec![ctcore::ct_util_name(), "--si"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.size_format, LsSizeFormat::Decimal);
        }

        #[test]
        fn test_config_from_file_size_block_size() {
            let args = vec![ctcore::ct_util_name(), "--block-size=4096"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.file_size_block_size, 4096);
        }

        #[test]
        fn test_config_from_directory_true() {
            let args = vec![ctcore::ct_util_name(), "--directory"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_directory);
        }

        #[test]
        fn test_config_from_directory_false() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_directory);
        }

        #[test]
        fn test_config_from_time_atime() {
            let args = vec![ctcore::ct_util_name(), "--time=atime"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsAccess);
        }

        #[test]
        fn test_config_from_time_ctime() {
            let args = vec![ctcore::ct_util_name(), "--time=ctime"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsChange);
        }

        #[test]
        fn test_config_from_time_status() {
            let args = vec![ctcore::ct_util_name(), "--time=status"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsChange);
        }

        #[test]
        fn test_config_from_time_access() {
            let args = vec![ctcore::ct_util_name(), "--time=status"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsChange);
        }

        #[test]
        fn test_config_from_time_use() {
            let args = vec![ctcore::ct_util_name(), "--time=use"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsAccess);
        }

        #[test]
        fn test_config_from_time_birth() {
            let args = vec![ctcore::ct_util_name(), "--time=birth"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsBirth);
        }

        #[test]
        fn test_config_from_time_creation() {
            let args = vec![ctcore::ct_util_name(), "--time=creation"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsBirth);
        }

        #[test]
        fn test_config_from_time_u() {
            let args = vec![ctcore::ct_util_name(), "-u"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsAccess);
        }

        #[test]
        fn test_config_from_time_c() {
            let args = vec![ctcore::ct_util_name(), "-c"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsChange);
        }

        #[test]
        fn test_config_from_time_default() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time, LsTime::LsModification);
        }

        #[test]
        fn test_config_from_long_default() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(
                config.long,
                LsLongFormat {
                    is_author: false,
                    is_group: true,
                    is_owner: true,
                    is_numeric_uid_gid: false,
                }
            );
        }

        #[test]
        fn test_config_from_long_author() {
            let args = vec![ctcore::ct_util_name(), "--author"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(
                config.long,
                LsLongFormat {
                    is_author: true,
                    is_group: true,
                    is_owner: true,
                    is_numeric_uid_gid: false,
                }
            );
        }

        #[test]
        fn test_config_from_long_no_group() {
            let args = vec![ctcore::ct_util_name(), "--no-group"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(
                config.long,
                LsLongFormat {
                    is_author: false,
                    is_group: false,
                    is_owner: true,
                    is_numeric_uid_gid: false,
                }
            );
        }

        #[test]
        fn test_config_from_long_g() {
            let args = vec![ctcore::ct_util_name(), "-g"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(
                config.long,
                LsLongFormat {
                    is_author: false,
                    is_group: true,
                    is_owner: false,
                    is_numeric_uid_gid: false,
                }
            );
        }

        #[test]
        fn test_config_from_long_numeric_uid_gid() {
            let args = vec![ctcore::ct_util_name(), "--numeric-uid-gid"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(
                config.long,
                LsLongFormat {
                    is_author: false,
                    is_group: true,
                    is_owner: true,
                    is_numeric_uid_gid: true,
                }
            );
        }

        #[test]
        fn test_config_from_alloc_size_true() {
            let args = vec![ctcore::ct_util_name(), "--size"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_alloc_size);
        }

        #[test]
        fn test_config_from_alloc_size_false() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_alloc_size);
        }

        #[test]
        fn test_config_width_80() {
            let args = vec![ctcore::ct_util_name(), "--width=80"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.width, 80);
        }

        #[test]
        fn test_config_width_default_by_env_columns_60() {
            if !stdout().is_terminal() {
                let args = vec![ctcore::ct_util_name()];
                let command = ct_app();
                let matches = command.try_get_matches_from(args).unwrap();
                let config = LsConfig::from(&matches).unwrap();

                assert_eq!(config.width, 80);

                std::env::set_var("COLUMNS", "60");
                let command2 = ct_app();
                let args2 = vec![ctcore::ct_util_name()];
                let matches = command2.try_get_matches_from(args2).unwrap();
                let config = LsConfig::from(&matches).unwrap();
                assert_eq!(config.width, 60);
                std::env::remove_var("COLUMNS");
            }
        }

        #[test]
        fn test_config_from_quoting_style_escape() {
            let args = vec![ctcore::ct_util_name(), "--quoting-style=escape"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(
                config.quoting_style,
                CtQuotingStyle::C {
                    quotes: CtQuotes::None
                }
            );
        }

        #[test]
        fn test_config_from_quoting_style_literal() {
            let args = vec![ctcore::ct_util_name(), "--quoting-style=literal"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Literal {
                        show_control: false
                    }
                );
            } else {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Literal { show_control: true }
                );
            }
        }

        #[test]
        fn test_config_from_quoting_style_shell() {
            let args = vec![ctcore::ct_util_name(), "--quoting-style=shell"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Shell {
                        escape: false,
                        always_quote: false,
                        show_control: false,
                    }
                );
            } else {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Shell {
                        escape: false,
                        always_quote: false,
                        show_control: true,
                    }
                );
            }
        }

        #[test]
        fn test_config_from_quoting_style_shell_always() {
            let args = vec![ctcore::ct_util_name(), "--quoting-style=shell-always"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            if stdout().is_terminal() {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Shell {
                        escape: false,
                        always_quote: true,
                        show_control: false,
                    }
                );
            } else {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Shell {
                        escape: false,
                        always_quote: true,
                        show_control: true,
                    }
                );
            }
        }

        #[test]
        fn test_config_from_quoting_style_shell_escape() {
            let args = vec![ctcore::ct_util_name(), "--quoting-style=shell-escape"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            if stdout().is_terminal() {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Shell {
                        escape: true,
                        always_quote: false,
                        show_control: false,
                    }
                );
            } else {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Shell {
                        escape: true,
                        always_quote: false,
                        show_control: true,
                    }
                );
            }
        }

        #[test]
        fn test_config_from_quoting_style_shell_escape_always() {
            let args = vec![
                ctcore::ct_util_name(),
                "--quoting-style=shell-escape-always",
            ];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Shell {
                        escape: true,
                        always_quote: true,
                        show_control: false,
                    }
                );
            } else {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Shell {
                        escape: true,
                        always_quote: true,
                        show_control: true,
                    }
                );
            }
        }

        #[test]
        fn test_config_from_quoting_style_c() {
            let args = vec![ctcore::ct_util_name(), "--quoting-style=c"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(
                config.quoting_style,
                CtQuotingStyle::C {
                    quotes: CtQuotes::Double
                }
            );
        }

        #[test]
        fn test_config_from_literal() {
            let args = vec![ctcore::ct_util_name(), "--literal"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Literal {
                        show_control: false
                    }
                );
            } else {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Literal { show_control: true }
                );
            }
        }

        #[test]
        fn test_config_from_escape() {
            let args = vec![ctcore::ct_util_name(), "--escape"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(
                config.quoting_style,
                CtQuotingStyle::C {
                    quotes: CtQuotes::None
                }
            );
        }

        #[test]
        fn test_config_from_quote_name() {
            let args = vec![ctcore::ct_util_name(), "--quote-name"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(
                config.quoting_style,
                CtQuotingStyle::C {
                    quotes: CtQuotes::Double
                }
            );
        }

        #[test]
        fn test_config_from_quoting_style_dired() {
            let args = vec![ctcore::ct_util_name(), "--dired", "--format=long"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Literal {
                        show_control: false
                    }
                );
            } else {
                assert_eq!(
                    config.quoting_style,
                    CtQuotingStyle::Literal { show_control: true }
                );
            }
        }

        #[test]
        fn test_config_from_indicator_style_slash() {
            let args = vec![ctcore::ct_util_name(), "--indicator-style=slash"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::Slash);
        }

        #[test]
        fn test_config_from_indicator_style_none() {
            let args = vec![ctcore::ct_util_name(), "--indicator-style=none"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::None);
        }

        #[test]
        fn test_config_from_indicator_style_file_type() {
            let args = vec![ctcore::ct_util_name(), "--indicator-style=file-type"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::FileType);
        }

        #[test]
        fn test_config_from_indicator_style_classify() {
            let args = vec![ctcore::ct_util_name(), "--indicator-style=classify"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_config_from_indicator_style_default() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::None);
        }

        #[test]
        fn test_config_from_classify_default() {
            let args = vec![ctcore::ct_util_name(), "--classify"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_config_from_classify_never() {
            let args = vec![ctcore::ct_util_name(), "--classify=never"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::None);
        }

        #[test]
        fn test_config_from_classify_no() {
            let args = vec![ctcore::ct_util_name(), "--classify=no"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::None);
        }

        #[test]
        fn test_config_from_classify_none() {
            let args = vec![ctcore::ct_util_name(), "--classify=none"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::None);
        }

        #[test]
        fn test_config_from_classify_always() {
            let args = vec![ctcore::ct_util_name(), "--classify=always"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_config_from_classify_yes() {
            let args = vec![ctcore::ct_util_name(), "--classify=yes"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_config_from_classify_force() {
            let args = vec![ctcore::ct_util_name(), "--classify=force"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_config_from_classify_auto() {
            let args = vec![ctcore::ct_util_name(), "--classify=auto"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert_eq!(config.indicator_style, LsIndicatorStyle::Classify);
            } else {
                assert_eq!(config.indicator_style, LsIndicatorStyle::None);
            }
        }

        #[test]
        fn test_config_from_classify_if_tty() {
            let args = vec![ctcore::ct_util_name(), "--classify=if-tty"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert_eq!(config.indicator_style, LsIndicatorStyle::Classify);
            } else {
                assert_eq!(config.indicator_style, LsIndicatorStyle::None);
            }
        }

        #[test]
        fn test_config_from_classify_tty() {
            let args = vec![ctcore::ct_util_name(), "--classify=tty"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert_eq!(config.indicator_style, LsIndicatorStyle::Classify);
            } else {
                assert_eq!(config.indicator_style, LsIndicatorStyle::None);
            }
        }

        #[test]
        fn test_config_from_indicator_style_p() {
            let args = vec![ctcore::ct_util_name(), "-p"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::Slash);
        }

        #[test]
        fn test_config_from_indicator_style_only_file_type() {
            let args = vec![ctcore::ct_util_name(), "--file-type"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.indicator_style, LsIndicatorStyle::FileType);
        }

        #[test]
        fn test_config_from_time_style_long_iso() {
            let args = vec![ctcore::ct_util_name(), "--time-style=long-iso"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time_style, LsTimeStyle::LsLongIso);
        }

        #[test]
        fn test_config_from_time_style_full_iso() {
            let args = vec![ctcore::ct_util_name(), "--time-style=full-iso"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time_style, LsTimeStyle::LsFullIso);
        }

        #[test]
        fn test_config_from_time_style_iso() {
            let args = vec![ctcore::ct_util_name(), "--time-style=iso"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time_style, LsTimeStyle::LsIso);
        }

        #[test]
        fn test_config_from_time_style_locale() {
            let args = vec![ctcore::ct_util_name(), "--time-style=locale"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time_style, LsTimeStyle::LsLocale);
        }

        #[test]
        fn test_config_from_time_style_full_time() {
            let args = vec![ctcore::ct_util_name(), "--full-time"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time_style, LsTimeStyle::LsFullIso);
        }

        #[test]
        fn test_config_from_time_style_default() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.time_style, LsTimeStyle::LsLocale);
        }

        #[test]
        fn test_config_from_context_true() {
            let args = vec![ctcore::ct_util_name(), "--context"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_context);
        }

        #[test]
        fn test_config_from_context_false() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_context);
        }

        #[test]
        fn test_config_from_selinux_supported() {
            let args = vec![ctcore::ct_util_name(), "-Z"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_selinux_supported);
        }

        #[test]
        fn test_config_from_group_directories_first_true() {
            let args = vec![ctcore::ct_util_name(), "--group-directories-first"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_group_directories_first);
        }

        #[test]
        fn test_config_from_group_directories_first_false() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_group_directories_first);
        }

        #[test]
        fn test_config_from_line_ending() {
            let args = vec![ctcore::ct_util_name(), "--zero"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.line_ending, CtLineEnding::Nul);
        }

        #[test]
        fn test_config_from_line_ending_default() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert_eq!(config.line_ending, CtLineEnding::Newline);
        }

        #[test]
        fn test_config_from_dired_false() {
            let args = vec![ctcore::ct_util_name(), "--format=long"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_dired);
        }

        #[test]
        fn test_config_from_hyperlink_default() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_hyperlink);
        }

        #[test]
        fn test_config_from_hyperlink_default_false() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_hyperlink);
        }

        #[test]
        fn test_config_from_hyperlink_always() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink=always"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_hyperlink);
        }

        #[test]
        fn test_config_from_hyperlink_yes() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink=yes"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_hyperlink);
        }

        #[test]
        fn test_config_from_hyperlink_force() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink=force"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(config.is_hyperlink);
        }

        #[test]
        fn test_config_from_hyperlink_auto() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink=auto"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert!(config.is_hyperlink);
            } else {
                assert!(!config.is_hyperlink);
            }
        }

        #[test]
        fn test_config_from_hyperlink_tty() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink=tty"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert!(config.is_hyperlink);
            } else {
                assert!(!config.is_hyperlink);
            }
        }

        #[test]
        fn test_config_from_hyperlink_if_tty() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink=if-tty"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert!(config.is_hyperlink);
            } else {
                assert!(!config.is_hyperlink);
            }
        }

        #[test]
        fn test_config_from_hyperlink_never() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink=never"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_hyperlink);
        }

        #[test]
        fn test_config_from_hyperlink_no() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink=no"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_hyperlink);
        }

        #[test]
        fn test_config_from_hyperlink_none() {
            let args = vec![ctcore::ct_util_name(), "--hyperlink=none"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            assert!(!config.is_hyperlink);
        }

        #[test]
        fn test_config_from_ignore_patterns() {
            let args = vec![ctcore::ct_util_name(), "--ignore=*.log,*.tmp"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();

            let expected_patterns = vec!["*.log,*.tmp".to_string()];
            let result = config
                .ignore_patterns
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<String>>();

            assert_eq!(result, expected_patterns);
        }

        #[test]
        fn test_config_from_color_always() {
            let args = vec![ctcore::ct_util_name(), "--color=always"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            assert!(config.color.is_some());
        }

        #[test]
        fn test_config_from_color_yes() {
            let args = vec![ctcore::ct_util_name(), "--color=yes"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            assert!(config.color.is_some());
        }

        #[test]
        fn test_config_from_color_force() {
            let args = vec![ctcore::ct_util_name(), "--color=force"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            assert!(config.color.is_some());
        }

        #[test]
        fn test_config_from_color_auto() {
            let args = vec![ctcore::ct_util_name(), "--color=auto"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert!(!config.color.is_none());
            } else {
                assert!(config.color.is_none());
            }
        }

        #[test]
        fn test_config_from_color_tty() {
            let args = vec![ctcore::ct_util_name(), "--color=tty"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert!(!config.color.is_none());
            } else {
                assert!(config.color.is_none());
            }
        }

        #[test]
        fn test_config_from_color_if_tty() {
            let args = vec![ctcore::ct_util_name(), "--color=if-tty"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            if stdout().is_terminal() {
                assert!(!config.color.is_none());
            } else {
                assert!(config.color.is_none());
            }
        }

        #[test]
        fn test_config_from_color_no() {
            let args = vec![ctcore::ct_util_name(), "--color=no"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            assert!(config.color.is_none());
        }

        #[test]
        fn test_config_from_color_none() {
            let args = vec![ctcore::ct_util_name(), "--color=none"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();
            let config = LsConfig::from(&matches).unwrap();
            println!("{:?}", config.color);
            assert!(config.color.is_none());
        }

        fn get_indicator_style_test_command() -> Command {
            Command::new("indicatortest")
                 .version("1.0.0")
                 .about("test")
                 .override_usage("test")
                 .infer_long_args(true)
                 .args_override_self(true)
                 .disable_help_flag(true)
                 .args(
                     vec![Arg::new(ls_flags::LS_INDICATOR_STYLE)
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
                              .action(ArgAction::SetTrue), ])
        }

        #[test]
        fn test_extract_indicator_style_none() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--indicator-style=none"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::None);
        }

        #[test]
        fn test_extract_indicator_style_slash() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--indicator-style=slash"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::Slash);
        }

        #[test]
        fn test_extract_indicator_style_file_type() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--indicator-style=file-type"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::FileType);
        }

        #[test]
        fn test_extract_indicator_style_classify() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--indicator-style=classify"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_extract_indicator_style_classify_always() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_extract_indicator_style_classify_yes_force() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify=yes"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_extract_indicator_style_classify_never_no_none() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify=never"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::None);
        }

        #[test]
        fn test_extract_indicator_style_classify_force() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify=force"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_extract_indicator_style_classify_tty() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify=tty"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            if stdout().is_terminal() {
                assert_eq!(result, LsIndicatorStyle::Classify);
            } else {
                assert_eq!(result, LsIndicatorStyle::None);
            }
        }

        #[test]
        fn test_extract_indicator_style_classify_if_tty() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify=if-tty"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            if stdout().is_terminal() {
                assert_eq!(result, LsIndicatorStyle::Classify);
            } else {
                assert_eq!(result, LsIndicatorStyle::None);
            }
        }

        #[test]
        fn test_extract_indicator_style_classify_param_none() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify=none"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::None);
        }

        #[test]
        fn test_extract_indicator_style_classify_param_no() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify=no"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::None);
        }

        #[test]
        fn test_extract_indicator_style_classify_param_always() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify=always"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::Classify);
        }

        #[test]
        fn test_extract_indicator_style_classify_auto() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "--classify=auto"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            if stdout().is_terminal() {
                assert_eq!(result, LsIndicatorStyle::Classify);
            } else {
                assert_eq!(result, LsIndicatorStyle::None);
            }
        }

        #[test]
        fn test_extract_indicator_style_slash_p() {
            let command = get_indicator_style_test_command();
            let args = vec!["indicatortest", "-p"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::Slash);
        }

        #[test]
        fn test_extract_indicator_style_mutually_exclusive_options() {
            // 示例：同时指定`--indicator-style`和`--classify`
            let command = get_indicator_style_test_command();
            let args = vec![
                "indicatortest",
                "--indicator-style=classify",
                "--classify=always",
            ];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_indicator_style(&matches);
            assert_eq!(result, LsIndicatorStyle::Classify); // 验证`--indicator-style`值被正确解析
        }

        fn get_quoting_test_command() -> Command {
            Command::new("quotingtest")
                .version("1.0.0")
                .about("test")
                .override_usage("test")
                .infer_long_args(true)
                .args_override_self(true)
                .disable_help_flag(true)
                .args(vec![
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
                    Arg::new(ls_flags::LS_DIRED)
                        .long(ls_flags::LS_DIRED)
                        .short('D')
                        .help("generate output designed for Emacs' dired (Directory Editor) mode")
                        .action(ArgAction::SetTrue),
                ])
        }

        #[test]
        fn test_extract_quoting_style_literal_false_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=literal"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, false);
            assert_eq!(
                result,
                CtQuotingStyle::Literal {
                    show_control: false
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_literal_true_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=literal"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, true);
            assert_eq!(result, CtQuotingStyle::Literal { show_control: true });
        }

        #[test]
        fn test_extract_quoting_style_escape_false_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=escape"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, false);
            assert_eq!(
                result,
                CtQuotingStyle::C {
                    quotes: CtQuotes::None
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_escape_true_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=escape"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, true);
            assert_eq!(
                result,
                CtQuotingStyle::C {
                    quotes: CtQuotes::None
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_c_false_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=c"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, false);
            assert_eq!(
                result,
                CtQuotingStyle::C {
                    quotes: CtQuotes::Double
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_uppercase_q_true_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "-Q"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, true);
            assert_eq!(
                result,
                CtQuotingStyle::C {
                    quotes: CtQuotes::Double
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_uppercase_q_false_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "-Q"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, false);
            assert_eq!(
                result,
                CtQuotingStyle::C {
                    quotes: CtQuotes::Double
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_uppercase_n_false_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "-N"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, false);
            assert_eq!(
                result,
                CtQuotingStyle::Literal {
                    show_control: false
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_uppercase_n_true_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "-N"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, true);
            assert_eq!(result, CtQuotingStyle::Literal { show_control: true });
        }

        #[test]
        fn test_extract_quoting_style_b_false_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "-b"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, false);
            assert_eq!(
                result,
                CtQuotingStyle::C {
                    quotes: CtQuotes::None
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_b_true_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "-b"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, true);
            assert_eq!(
                result,
                CtQuotingStyle::C {
                    quotes: CtQuotes::None
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_shell_escape_false_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=shell-escape"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, false);
            assert_eq!(
                result,
                CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: false,
                    show_control: false,
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_shell_escape_true_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=shell-escape"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, true);
            assert_eq!(
                result,
                CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: false,
                    show_control: true,
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_shell_escape_always_false_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=shell-escape-always"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, false);
            assert_eq!(
                result,
                CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: false,
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_shell_escape_always_true_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=shell-escape-always"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, true);
            assert_eq!(
                result,
                CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: true,
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_shell_always_false_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=shell-always"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, false);
            assert_eq!(
                result,
                CtQuotingStyle::Shell {
                    escape: false,
                    always_quote: true,
                    show_control: false,
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_shell_always_true_show_control() {
            let command = get_quoting_test_command();

            let args = vec!["quotingtest", "--quoting-style=shell-always"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, true);
            assert_eq!(
                result,
                CtQuotingStyle::Shell {
                    escape: false,
                    always_quote: true,
                    show_control: true,
                }
            );
        }

        #[test]
        fn test_extract_quoting_style_env_variable_valid_show_control() {
            let command = get_quoting_test_command();

            // Set the QUOTING_STYLE environment variable
            std::env::set_var("QUOTING_STYLE", "shell-escape");

            let args = vec!["quotingtest", "--quoting-style=shell-escape"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_quoting_style(&matches, true);
            println!("{:?}", result);
            assert_eq!(
                result,
                CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: false,
                    show_control: true,
                }
            );

            // Clear the environment variable after the test
            std::env::remove_var("QUOTING_STYLE");
        }

        #[test]
        fn test_extract_quoting_style_env_variable_invalid_show_control() {
            let command = get_quoting_test_command();

            // Set the QUOTING_STYLE environment variable with an invalid value
            std::env::set_var("QUOTING_STYLE", "invalid_style");

            let args = vec!["quotingtest"];
            let matches = command.try_get_matches_from(args).unwrap();

            let _result = extract_quoting_style(&matches, true);
            // The function should print an error message and fallback to the default style

            // Clear the environment variable after the test
            std::env::remove_var("QUOTING_STYLE");
        }

        #[test]
        fn test_match_quoting_style_name_literal() {
            assert_eq!(
                match_quoting_style_name("literal", true),
                Some(CtQuotingStyle::Literal { show_control: true })
            );
        }

        #[test]
        fn test_match_quoting_style_name_shell() {
            assert_eq!(
                match_quoting_style_name("shell", false),
                Some(CtQuotingStyle::Shell {
                    escape: false,
                    always_quote: false,
                    show_control: false,
                })
            );
        }

        #[test]
        fn test_match_quoting_style_name_shell_always() {
            assert_eq!(
                match_quoting_style_name("shell-always", true),
                Some(CtQuotingStyle::Shell {
                    escape: false,
                    always_quote: true,
                    show_control: true,
                })
            );
        }

        #[test]
        fn test_match_quoting_style_name_shell_escape() {
            assert_eq!(
                match_quoting_style_name("shell-escape", false),
                Some(CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: false,
                    show_control: false,
                })
            );
        }

        #[test]
        fn test_match_quoting_style_name_shell_escape_always() {
            assert_eq!(
                match_quoting_style_name("shell-escape-always", true),
                Some(CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: true,
                })
            );
        }

        #[test]
        fn test_match_quoting_style_name_c() {
            assert_eq!(
                match_quoting_style_name("c", true),
                Some(CtQuotingStyle::C {
                    quotes: ct_quoting_style::CtQuotes::Double
                })
            );
        }

        #[test]
        fn test_match_quoting_style_name_escape() {
            assert_eq!(
                match_quoting_style_name("escape", false),
                Some(CtQuotingStyle::C {
                    quotes: CtQuotes::None
                })
            );
        }

        #[test]
        fn test_match_quoting_style_name_invalid() {
            assert_eq!(match_quoting_style_name("invalid", true), None);
        }

        #[test]
        fn test_extract_color_positive() {
            let args = vec![ctcore::ct_util_name(), "--color=always"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            assert_eq!(extract_color(&options), true);
        }

        #[test]
        fn test_extract_color_negative() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            assert_eq!(extract_color(&options), false);
        }

        #[test]
        fn test_extract_color_auto() {
            let args = vec![ctcore::ct_util_name(), "--color=auto"];
            let command = ct_app();
            let options = command.get_matches_from(args);
            if stdout().is_terminal() {
                assert_eq!(extract_color(&options), true);
            } else {
                assert_eq!(extract_color(&options), false);
            }
        }

        #[test]
        fn test_extract_color_tty() {
            let args = vec![ctcore::ct_util_name(), "--color=tty"];
            let command = ct_app();
            let options = command.get_matches_from(args);
            if stdout().is_terminal() {
                assert_eq!(extract_color(&options), true);
            } else {
                assert_eq!(extract_color(&options), false);
            }
        }

        #[test]
        fn test_extract_color_never() {
            let args = vec![ctcore::ct_util_name(), "--color=never"];
            let command = ct_app();
            let options = command.get_matches_from(args);
            assert_eq!(extract_color(&options), false);
        }

        #[test]
        fn test_extract_color_no() {
            let args = vec![ctcore::ct_util_name(), "--color=no"];
            let command = ct_app();
            let options = command.get_matches_from(args);
            assert_eq!(extract_color(&options), false);
        }

        #[test]
        fn test_extract_color_none() {
            let args = vec![ctcore::ct_util_name(), "--color=none"];
            let command = ct_app();
            let options = command.get_matches_from(args);
            assert_eq!(extract_color(&options), false);
        }

        #[test]
        fn test_extract_color_default() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let options = command.get_matches_from(args);
            assert_eq!(extract_color(&options), false);
        }

        #[test]
        fn test_ls_error_block_size_parse_error() {
            let error = LsError::LsBlockSizeParseError("1024".to_string());
            let mut buffer = String::new();
            write!(buffer, "{}", error).unwrap();
            assert_eq!("invalid --block-size argument '1024'", buffer);
        }

        #[test]
        fn test_ls_error_conflicting_argument_dired() {
            let error = LsError::LsConflictingArgumentDired;
            let mut buffer = String::new();
            write!(buffer, "{}", error).unwrap();
            assert_eq!("--dired requires --format=long", buffer);
        }

        #[test]
        fn test_ls_error_dired_and_zero_are_incompatible() {
            let error = LsError::LsDiredAndZeroAreIncompatible;
            let mut buffer = String::new();
            write!(buffer, "{}", error).unwrap();
            assert_eq!("--dired and --zero are incompatible", buffer);
        }

        #[test]
        fn test_ls_error_time_style_parse_error() {
            let error = LsError::LsTimeStyleParseError(
                "custom".to_string(),
                vec![
                    "default".to_string(),
                    "long".to_string(),
                    "full".to_string(),
                ],
            );
            let mut buffer = String::new();
            write!(buffer, "{}", error).unwrap();
            assert_eq!(
                 "invalid --time-style argument 'custom'\nPossible values are: [\"default\", \"long\", \"full\"]\n\nFor more information try --help",
                 buffer
             );
        }

        #[test]
        fn test_ls_error_invalid_line_width() {
            let error = LsError::LsInvalidLineWidth("500".to_string());
            let mut buffer = String::new();
            write!(buffer, "{}", error).unwrap();
            assert_eq!("invalid line width: '500'", buffer);
        }

        #[test]
        fn test_ls_error_io_error() {
            let error =
                LsError::LsIOError(std::io::Error::new(std::io::ErrorKind::Other, "io error"));
            let mut buffer = String::new();
            write!(buffer, "{}", error).unwrap();
            assert_eq!("general io error: io error", buffer);
        }

        #[test]
        fn test_ls_error_io_error_context_no_access_true() {
            let error = LsError::LsIOErrorContext(
                std::io::Error::new(PermissionDenied, "Operation not permitted"),
                PathBuf::from("/path/to/file"),
                true,
            );
            let mut buffer = String::new();
            write!(buffer, "{}", error).unwrap();
            assert_eq!(
                "cannot access '/path/to/file': Operation not permitted",
                buffer
            );
        }

        #[test]
        fn test_ls_error_io_error_context_false() {
            let error = LsError::LsIOErrorContext(
                std::io::Error::new(PermissionDenied, "Operation not permitted"),
                PathBuf::from("/path/to/file"),
                false,
            );
            let mut buffer = String::new();
            write!(buffer, "{}", error).unwrap();
            assert_eq!(
                "cannot access '/path/to/file': Operation not permitted",
                buffer
            );
        }

        #[test]
        fn test_ls_error_already_listed_error() {
            let error = LsError::LsAlreadyListedError(PathBuf::from("/path/to/directory"));
            let mut buffer = String::new();
            write!(buffer, "{}", error).unwrap();
            assert_eq!(
                "/path/to/directory: not listing already-listed directory",
                buffer
            );
        }

        fn get_hyperlink_test_command() -> Command {
            Command::new("hyperlinktest")
                .version("1.0.0")
                .about("test")
                .override_usage("test")
                .infer_long_args(true)
                .args_override_self(true)
                .arg(
                    Arg::new(ls_flags::LS_HYPERLINK)
                        .long(ls_flags::LS_HYPERLINK)
                        .help("hyperlink file names WHEN")
                        .value_parser([
                            "always", "yes", "force", "auto", "tty", "if-tty", "never", "no",
                            "none",
                        ])
                        .require_equals(true)
                        .num_args(0..=1)
                        .default_missing_value("always")
                        .default_value("never")
                        .value_name("WHEN"),
                )
        }

        #[test]
        fn test_extract_hyperlink_always() {
            let command = get_hyperlink_test_command();

            let args = vec!["program", "--hyperlink=always"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_hyperlink(&matches);
            assert_eq!(result, true);
        }

        #[test]
        fn test_extract_hyperlink_yes() {
            let command = get_hyperlink_test_command();
            let args = vec!["program", "--hyperlink=always"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_hyperlink(&matches);
            assert_eq!(result, true);
        }

        #[test]
        fn test_extract_hyperlink_force() {
            let command = get_hyperlink_test_command();
            let args = vec!["program", "--hyperlink=force"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_hyperlink(&matches);
            assert_eq!(result, true);
        }

        #[test]
        fn test_extract_hyperlink_auto() {
            let command = get_hyperlink_test_command();
            let args = vec!["program", "--hyperlink=auto"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_hyperlink(&matches);
            if stdout().is_terminal() {
                assert_eq!(result, true);
            } else {
                assert_eq!(result, false);
            }
        }

        #[test]
        fn test_extract_hyperlink_tty() {
            let command = get_hyperlink_test_command();
            let args = vec!["program", "--hyperlink=tty"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_hyperlink(&matches);
            if stdout().is_terminal() {
                assert_eq!(result, true);
            } else {
                assert_eq!(result, false);
            }
        }

        #[test]
        fn test_extract_hyperlink_if_tty() {
            let command = get_hyperlink_test_command();
            let args = vec!["program", "--hyperlink=if-tty"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_hyperlink(&matches);
            if stdout().is_terminal() {
                assert_eq!(result, true);
            } else {
                assert_eq!(result, false);
            }
        }

        #[test]
        fn test_extract_hyperlink_never() {
            let command = get_hyperlink_test_command();
            let args = vec!["program", "--hyperlink=never"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_hyperlink(&matches);
            assert_eq!(result, false);
        }

        #[test]
        fn test_extract_hyperlink_no() {
            let command = get_hyperlink_test_command();
            let args = vec!["program", "--hyperlink=no"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_hyperlink(&matches);
            assert_eq!(result, false);
        }

        #[test]
        fn test_extract_hyperlink_none() {
            let command = get_hyperlink_test_command();
            let args = vec!["program", "--hyperlink=none"];
            let matches = command.try_get_matches_from(args).unwrap();

            let result = extract_hyperlink(&matches);
            assert_eq!(result, false);
        }

        #[test]
        fn test_extract_hyperlink_unreachable() {
            let command = get_hyperlink_test_command();
            let args = vec!["program", "--hyperlink=invalid"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
        }

        #[test]
        fn test_is_color_compatible_term() {
            // Test case 1: TERM and COLORTERM are not set
            {
                std::env::remove_var("TERM");
                std::env::remove_var("COLORTERM");

                assert!(is_color_compatible_term());
            }

            // Test case 2: TERM is set but empty
            {
                std::env::set_var("TERM", "");
                std::env::remove_var("COLORTERM");

                assert!(is_color_compatible_term());
            }

            // Test case 3: TERM is set and non-empty, but does not match
            {
                std::env::set_var("TERM", "xterm");
                std::env::remove_var("COLORTERM");

                assert!(is_color_compatible_term());
            }

            // Test case 4: TERM is set and non-empty, and matches
            {
                std::env::set_var("TERM", "xterm-color");
                std::env::remove_var("COLORTERM");

                assert!(is_color_compatible_term());
            }

            // Test case 5: TERM is set and non-empty, and matches with wildcard
            {
                std::env::set_var("TERM", "xterm-256color");
                std::env::remove_var("COLORTERM");

                assert!(is_color_compatible_term());
            }

            // Test case 6: TERM and COLORTERM are set and non-empty, but TERM does not match
            {
                std::env::set_var("TERM", "vt100");
                std::env::set_var("COLORTERM", "truecolor");

                assert!(is_color_compatible_term());
            }

            // Test case 7: TERM and COLORTERM are set and non-empty, and TERM matches
            {
                std::env::set_var("TERM", "screen");
                std::env::set_var("COLORTERM", "truecolor");

                assert!(is_color_compatible_term());
            }
        }

        fn get_format_test_command() -> Command {
            Command::new("formattest")
                .version("1.0.0")
                .about("test")
                .override_usage("test")
                .infer_long_args(true)
                .args_override_self(true)
                .disable_help_flag(true)
                .args(vec![
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
                    Arg::new(ls_flags::LS_DIRED)
                        .long(ls_flags::LS_DIRED)
                        .short('D')
                        .help("generate output designed for Emacs' dired (Directory Editor) mode")
                        .action(ArgAction::SetTrue),
                    Arg::new(ls_flags::LS_HYPERLINK)
                        .long(ls_flags::LS_HYPERLINK)
                        .help("hyperlink file names WHEN")
                        .value_parser([
                            "always", "yes", "force", "auto", "tty", "if-tty", "never", "no",
                            "none",
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
                ])
        }

        #[test]
        fn test_extract_format_with_format_option() {
            let args = vec!["formattest", "--format=long"];
            let command = get_format_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let (format, flag) = ls_extract_format(&options);
            assert_eq!(format, LsFormat::Long);
            assert_eq!(flag, Some(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_extract_format_with_long_flag() {
            let args = vec!["formattest", "--long"];
            let command = get_format_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let (format, flag) = ls_extract_format(&options);
            assert_eq!(format, LsFormat::Long);
            assert_eq!(flag, Some(ls_flags::format::LS_LONG));
        }

        #[test]
        fn test_extract_format_with_across_flag() {
            let args = vec!["formattest", "-x"];
            let command = get_format_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let (format, flag) = ls_extract_format(&options);
            assert_eq!(format, LsFormat::Across);
            assert_eq!(flag, Some(ls_flags::format::LS_ACROSS));
        }

        #[test]
        fn test_extract_format_with_commas_flag() {
            let args = vec!["formattest", "-m"];
            let command = get_format_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let (format, flag) = ls_extract_format(&options);
            assert_eq!(format, LsFormat::Commas);
            assert_eq!(flag, Some(ls_flags::format::LS_COMMAS));
        }

        #[test]
        fn test_extract_format_with_columns_flag() {
            let args = vec!["formattest", "-C"];
            let command = get_format_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let (format, flag) = ls_extract_format(&options);
            assert_eq!(format, LsFormat::Columns);
            assert_eq!(flag, Some(ls_flags::format::LS_COLUMNS));
        }

        #[test]
        fn test_extract_format_without_format_option_and_flags() {
            let args = vec!["formattest"];
            let command = get_format_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let (format, flag) = ls_extract_format(&options);
            if stdout().is_terminal() {
                assert_eq!(format, LsFormat::Columns);
            } else {
                assert_eq!(format, LsFormat::OneLine);
            }
            assert_eq!(flag, None);
        }

        fn get_time_test_command() -> Command {
            let args = vec![
                Arg::new(ls_flags::LS_TIME_STYLE)
                    .long(ls_flags::LS_TIME_STYLE)
                    .help("time/date format with -l; see LS_TIME_STYLE below")
                    .value_name("LS_TIME_STYLE")
                    .env("LS_TIME_STYLE")
                    .value_parser(NonEmptyStringValueParser::new())
                    .overrides_with_all([ls_flags::LS_TIME_STYLE]),
                Arg::new(ls_flags::LS_FULL_TIME)
                    .long(ls_flags::LS_FULL_TIME)
                    .overrides_with(ls_flags::LS_FULL_TIME)
                    .help("like -l --time-style=full-iso")
                    .action(ArgAction::SetTrue),
            ];

            Command::new("timetest")
                .version("1.0.0")
                .about("test")
                .override_usage("test")
                .infer_long_args(true)
                .args_override_self(true)
                .args(args)
        }

        #[test]
        fn test_parse_time_style_full_iso() {
            let args = vec!["timetest", "--time-style=full-iso"];
            let command = get_time_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let result = ls_parse_time_style(&options);

            assert!(result.is_ok());
            let style = result.unwrap();
            assert_eq!(style, LsTimeStyle::LsFullIso);
        }

        #[test]
        fn test_parse_time_style_long_iso() {
            // let args = vec!["--time-style".to_string() + "=long-iso"];
            let args = vec!["timetest", "--time-style=long-iso"];
            let command = get_time_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let result = ls_parse_time_style(&options);

            assert!(result.is_ok());
            let style = result.unwrap();
            assert_eq!(style, LsTimeStyle::LsLongIso);
        }

        #[test]
        fn test_parse_time_style_iso() {
            let args = vec!["timetest", "--time-style=iso"];
            let command = get_time_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let result = ls_parse_time_style(&options);

            assert!(result.is_ok());
            let style = result.unwrap();
            assert_eq!(style, LsTimeStyle::LsIso);
        }

        #[test]
        fn test_parse_time_style_locale() {
            let args = vec!["timetest", "--time-style=locale"];
            let command = get_time_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let result = ls_parse_time_style(&options);

            assert!(result.is_ok());
            let style = result.unwrap();
            assert_eq!(style, LsTimeStyle::LsLocale);
        }

        #[test]
        fn test_parse_time_style_format() {
            let args = vec!["timetest", "--time-style=+%H:%M"];
            let command = get_time_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let result = ls_parse_time_style(&options);

            assert!(result.is_ok());
            let style = result.unwrap();
            assert_eq!(style, LsTimeStyle::LsFormat(String::from("%H:%M")));
        }

        #[test]
        fn test_parse_time_style_invalid() {
            let args = vec!["timetest", "--time-style=invalid"];
            let command = get_time_test_command();
            let options = command.try_get_matches_from(args).unwrap();

            let result = ls_parse_time_style(&options);

            assert!(result.is_err());
            let error = format!("{}", result.unwrap_err());
            let expected = format!(
                "{}",
                LsError::LsTimeStyleParseError(
                    "invalid".to_string(),
                    vec![
                        "full-iso".to_string(),
                        "long-iso".to_string(),
                        "iso".to_string(),
                        "locale".to_string(),
                        "+FORMAT (e.g., +%H:%M) for a 'date'-style format".to_string(),
                    ],
                )
            );
            assert_eq!(error, expected);
        }

        #[test]
        fn test_parse_time_style_full_time_flag() {
            let args = vec!["--full-time".to_string()];
            let command = get_time_test_command();
            let options = command.try_get_matches_from(args).unwrap();
            let result = ls_parse_time_style(&options);

            assert!(result.is_ok());
            let style = result.unwrap();
            assert_eq!(style, LsTimeStyle::LsLocale);
        }

        #[test]
        fn test_parse_time_style_no_flags() {
            let args: Vec<String> = vec![];
            let command = get_time_test_command();
            let options = command.try_get_matches_from(args).unwrap();
            let result = ls_parse_time_style(&options);

            assert!(result.is_ok());
            let style = result.unwrap();
            assert_eq!(style, LsTimeStyle::LsLocale);
        }

        #[test]
        fn test_create_hyperlink() {
            let name = "example";
            let path = default_pathdata();

            let _expected = "\x1b]8;example\x07";

            assert!(create_hyperlink(name, &path).contains("example"));
        }

        #[test]
        fn test_apply_style_with_same_style() {
            let style = Style::from_ansi_sequence("38;2;255;0;100;1;4").unwrap();
            let mut style_manager = StyleManager::new();

            let result = style_manager.apply_style(&style, "test");

            // Assert the expected output
            // Replace with the expected output based on the `apply_style` logic
            let expected = "\u{1b}[0m\u{1b}[01;04;38;2;255;0;100mtest\u{1b}[0m";
            assert_eq!(expected, result);
        }

        #[test]
        fn test_apply_style_with_new_style() {
            let style1 = Style::from_ansi_sequence("38;2;255;0;100;1;4").unwrap();
            let style2 = Style::from_ansi_sequence("36;5;48;5;100;1;4").unwrap();
            let mut style_manager = StyleManager::new();

            let _result = style_manager.apply_style(&style1, "test1");
            let result = style_manager.apply_style(&style2, "test2");

            // Assert the expected output
            // Replace with the expected output based on the `apply_style` logic
            let expected = "\u{1b}[0m\u{1b}[01;04;05;48;5;100;36mtest2\u{1b}[0m"; // Replace with the expected output
            assert_eq!(expected, result);
        }

        #[test]
        fn test_apply_style_based_on_metadata_with_no_md_option() {
            let path = default_pathdata();
            let mut style_manager = StyleManager::new();
            let md_option = None;
            let ls_colors = default_ls_colors();

            let result = apply_style_based_on_metadata(
                &path,
                md_option,
                &ls_colors,
                &mut style_manager,
                "test",
            );

            assert_eq!("\u{1b}[0m\u{1b}[01;32mtest\u{1b}[0m", result);
        }

        #[test]
        fn test_apply_style_based_on_metadata_with_md_option() {
            let path = default_pathdata();
            let mut style_manager = StyleManager::new();
            let binding = fs::metadata("/dev/null").unwrap();
            let md_option = Some(&binding);
            let ls_colors = default_ls_colors();

            let result = apply_style_based_on_metadata(
                &path,
                md_option,
                &ls_colors,
                &mut style_manager,
                "test",
            );

            assert_eq!("\u{1b}[0m\u{1b}[01;33mtest\u{1b}[0m", result);
        }

        #[test]
        fn test_extract_files_all_flag() {
            let args = vec![ctcore::ct_util_name(), "--all"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_files(&options);
            assert_eq!(result, LsFiles::LsAll);
        }

        #[test]
        fn test_extract_files_almost_all_flag() {
            let args = vec![ctcore::ct_util_name(), "--almost-all"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_files(&options);
            assert_eq!(result, LsFiles::LsAlmostAll);
        }

        #[test]
        fn test_extract_files_no_flag() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_files(&options);
            assert_eq!(result, LsFiles::LsNormal);
        }

        #[test]
        fn test_extract_sort_with_sort_flag() {
            let args = vec![ctcore::ct_util_name(), "--sort=name"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_sort(&options);
            assert_eq!(result, LsSort::Name);
        }

        #[test]
        fn test_extract_sort_with_time_flag() {
            let args = vec![ctcore::ct_util_name(), "--sort=time"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_sort(&options);
            assert_eq!(result, LsSort::Time);
        }

        #[test]
        fn test_extract_sort_with_none_flag() {
            let args = vec![ctcore::ct_util_name(), "--sort=none"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_sort(&options);
            assert_eq!(result, LsSort::None);
        }

        #[test]
        fn test_extract_sort_with_size_flag() {
            let args = vec![ctcore::ct_util_name(), "--sort=size"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_sort(&options);
            assert_eq!(result, LsSort::Size);
        }

        #[test]
        fn test_extract_sort_with_verison_flag() {
            let args = vec![ctcore::ct_util_name(), "--sort=version"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_sort(&options);
            assert_eq!(result, LsSort::Version);
        }

        #[test]
        fn test_extract_sort_with_extension_flag() {
            let args = vec![ctcore::ct_util_name(), "--sort=extension"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_sort(&options);
            assert_eq!(result, LsSort::Extension);
        }

        #[test]
        fn test_extract_sort_with_width_flag() {
            let args = vec![ctcore::ct_util_name(), "--sort=width"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let result = extract_sort(&options);
            assert_eq!(result, LsSort::Width);
        }

        #[test]
        fn test_extract_time_with_ct_time_ctime() {
            let args = vec![ctcore::ct_util_name(), "--time=ctime"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let time = extract_time(&options);
            assert_eq!(time, LsTime::LsChange);
        }

        #[test]
        fn test_extract_time_with_status_ctime() {
            let args = vec![ctcore::ct_util_name(), "--time=status"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let time = extract_time(&options);
            assert_eq!(time, LsTime::LsChange);
        }

        #[test]
        fn test_extract_time_with_ct_time_access() {
            let args = vec![ctcore::ct_util_name(), "--time=access"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let time = extract_time(&options);
            assert_eq!(time, LsTime::LsAccess);
        }

        #[test]
        fn test_extract_time_with_ct_time_atime() {
            let args = vec![ctcore::ct_util_name(), "--time=atime"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let time = extract_time(&options);
            assert_eq!(time, LsTime::LsAccess);
        }

        #[test]
        fn test_extract_time_with_ct_time_birth() {
            let args = vec![ctcore::ct_util_name(), "--time=birth"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let time = extract_time(&options);
            assert_eq!(time, LsTime::LsBirth);
        }

        #[test]
        fn test_extract_time_with_ct_time_creation() {
            let args = vec![ctcore::ct_util_name(), "--time=creation"];
            let command = ct_app();
            let options = command.try_get_matches_from(args).unwrap();

            let time = extract_time(&options);
            assert_eq!(time, LsTime::LsBirth);
        }

        #[test]
        fn test_display_item_no_file_name_base() {
            let file_name = "testfile.tmp1";
            let path = default_path_data_by_file_name(file_name);
            let config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());

            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp1"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_hyperlink_false() {
            let file_name = "testfile.tmp1";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());

            config.is_hyperlink = false;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp1"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_hyperlink_true() {
            let file_name = "testfile.tmp1";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.is_hyperlink = true;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp1"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_color_none() {
            let file_name = "testfile.tmp1";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.color = None;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp1"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_color_default() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.color = Some(LsColors::default());
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_color_set() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.color = Some(LsColors::from_string("*.tmp=01;32"));
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_format_long() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::Long;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_format_oneline() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::OneLine;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_format_columns() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::Columns;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_format_commas() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::Commas;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_format_across() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::Across;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_indicator_style_none() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.indicator_style = LsIndicatorStyle::None;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_indicator_style_classify() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.indicator_style = LsIndicatorStyle::Classify;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_indicator_style_slash() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.indicator_style = LsIndicatorStyle::Slash;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_name_no_file_indicator_style_filetype() {
            let file_name = "testfile.tmp";
            let path = default_path_data_by_file_name(file_name);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.indicator_style = LsIndicatorStyle::FileType;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name.len() + 2);
        }

        #[test]
        fn test_display_item_file_name_base() {
            let file_name = "testfile.tmp1";

            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());

            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp1"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_hyperlink_false() {
            let file_name = "testfile.tmp1";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());

            config.is_hyperlink = false;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp1"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_hyperlink_true() {
            let file_name = "testfile.tmp1";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.is_hyperlink = true;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp1"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_color_none() {
            let file_name = "testfile.tmp1";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.color = None;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp1"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_color_default() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.color = Some(LsColors::default());
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_color_set() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.color = Some(LsColors::from_string("*.tmp=01;32"));
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_format_long() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::Long;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_format_oneline() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::OneLine;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_format_columns() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::Columns;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_format_commas() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::Commas;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_format_across() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.format = LsFormat::Across;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_indicator_style_none() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.indicator_style = LsIndicatorStyle::None;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_indicator_style_classify() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.indicator_style = LsIndicatorStyle::Classify;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_indicator_style_slash() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.indicator_style = LsIndicatorStyle::Slash;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_display_item_name_file_indicator_style_filetype() {
            let file_name = "testfile.tmp";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(file_name);
            let _tmp_file = File::create(&file_path).unwrap();
            let file_name_full_path = file_path.to_str().unwrap();

            let path = default_path_data_by_file_name(file_name_full_path);
            let mut config = setup_default_config();
            let mut style_manager = StyleManager::new();
            let mut out = Cursor::new(Vec::new());
            config.indicator_style = LsIndicatorStyle::FileType;
            let result = display_item_name(
                &path,
                &config,
                None,
                String::new(),
                &mut out,
                &mut style_manager,
            );

            assert!(result.contents.contains("testfile.tmp"));
            assert_eq!(result.width, file_name_full_path.len() + 2);
        }

        #[test]
        fn test_color_name_base() {
            let path_data = default_pathdata();
            let ls_colors = default_ls_colors();
            // Create a buffer to capture the output
            let mut output = Cursor::new(Vec::new());
            let mut style_manager = StyleManager::new();
            // Call the color_name function
            let result = color_name(
                "test".to_string(),
                &path_data,
                &ls_colors,
                &mut style_manager,
                &mut output,
                None,
            );

            // Assert the expected output
            assert_eq!("\u{1b}[0m\u{1b}[01;32mtest\u{1b}[0m", result);

            // Assert the output buffer is empty (no extra writes)
            assert_eq!(0, output.get_ref().len());
        }

        #[test]
        fn test_color_name_base_command_line_true() {
            let mut path_data = default_pathdata();
            let ls_colors = default_ls_colors();

            path_data.is_command_line = true;
            // Create a buffer to capture the output
            let mut output = Cursor::new(Vec::new());
            let mut style_manager = StyleManager::new();
            // Call the color_name function
            let result = color_name(
                "test".to_string(),
                &path_data,
                &ls_colors,
                &mut style_manager,
                &mut output,
                None,
            );

            // Assert the expected output
            assert_eq!("\u{1b}[0m\u{1b}[01;32mtest\u{1b}[0m", result);

            // Assert the output buffer is empty (no extra writes)
            assert_eq!(0, output.get_ref().len());
        }

        #[test]
        fn test_color_name_must_dereference_true() {
            let mut path_data = default_pathdata();
            let ls_colors = default_ls_colors();

            path_data.is_must_dereference = true;

            // Create a buffer to capture the output
            let mut output = Cursor::new(Vec::new());
            let mut style_manager = StyleManager::new();
            // Call the color_name function
            let result = color_name(
                "test".to_string(),
                &path_data,
                &ls_colors,
                &mut style_manager,
                &mut output,
                None,
            );

            // Assert the expected output
            assert_eq!("\u{1b}[0m\u{1b}[01;32mtest\u{1b}[0m", result);

            // Assert the output buffer is empty (no extra writes)
            assert_eq!(0, output.get_ref().len());
        }

        #[test]
        fn test_color_name_security_context() {
            let mut path_data = default_pathdata();
            let ls_colors = default_ls_colors();

            path_data.security_context = "testwithcontext".to_string();

            // Create a buffer to capture the output
            let mut output = Cursor::new(Vec::new());
            let mut style_manager = StyleManager::new();
            // Call the color_name function
            let result = color_name(
                "test".to_string(),
                &path_data,
                &ls_colors,
                &mut style_manager,
                &mut output,
                None,
            );

            // Assert the expected output
            assert_eq!("\u{1b}[0m\u{1b}[01;32mtest\u{1b}[0m", result);

            // Assert the output buffer is empty (no extra writes)
            assert_eq!(0, output.get_ref().len());
        }

        #[test]
        fn test_color_name_display_name() {
            let mut path_data = default_pathdata();
            let ls_colors = default_ls_colors();

            path_data.display_name = OsString::from("testfile.help");
            // Create a buffer to capture the output
            let mut output = Cursor::new(Vec::new());
            let mut style_manager = StyleManager::new();
            // Call the color_name function
            let result = color_name(
                "test".to_string(),
                &path_data,
                &ls_colors,
                &mut style_manager,
                &mut output,
                None,
            );

            // Assert the expected output
            assert_eq!("\u{1b}[0m\u{1b}[01;32mtest\u{1b}[0m", result);

            // Assert the output buffer is empty (no extra writes)
            assert_eq!(0, output.get_ref().len());
        }

        #[test]
        fn test_color_name_p_buf() {
            let mut path_data = default_pathdata();
            let ls_colors = default_ls_colors();

            path_data.p_buf = PathBuf::from("testfile.hello");
            // Create a buffer to capture the output
            let mut output = Cursor::new(Vec::new());
            let mut style_manager = StyleManager::new();
            // Call the color_name function
            let result = color_name(
                "test".to_string(),
                &path_data,
                &ls_colors,
                &mut style_manager,
                &mut output,
                None,
            );

            // Assert the expected output
            assert_eq!("test", result);

            // Assert the output buffer is empty (no extra writes)
            assert_eq!(0, output.get_ref().len());
        }

        #[test]
        fn test_color_name_md() {
            let mut path_data = default_pathdata();
            let ls_colors = default_ls_colors();

            path_data.md = OnceCell::from(Some(fs::metadata("/dev/null").unwrap()));
            // Create a buffer to capture the output
            let mut output = Cursor::new(Vec::new());
            let mut style_manager = StyleManager::new();
            // Call the color_name function
            let result = color_name(
                "test".to_string(),
                &path_data,
                &ls_colors,
                &mut style_manager,
                &mut output,
                None,
            );

            // Assert the expected output
            assert_eq!("\u{1b}[0m\u{1b}[01;33mtest\u{1b}[0m", result);

            // Assert the output buffer is empty (no extra writes)
            assert_eq!(0, output.get_ref().len());
        }

        #[test]
        fn test_color_name_ft() {
            let mut path_data = default_pathdata();
            let ls_colors = default_ls_colors();

            path_data.ft = OnceCell::from(Some(fs::metadata("/dev/null").unwrap().file_type()));
            // Create a buffer to capture the output
            let mut output = Cursor::new(Vec::new());
            let mut style_manager = StyleManager::new();
            // Call the color_name function
            let result = color_name(
                "test".to_string(),
                &path_data,
                &ls_colors,
                &mut style_manager,
                &mut output,
                None,
            );

            // Assert the expected output
            assert_eq!("\u{1b}[0m\u{1b}[01;32mtest\u{1b}[0m", result);

            // Assert the output buffer is empty (no extra writes)
            assert_eq!(0, output.get_ref().len());
        }

        #[test]
        fn test_color_name_tmpfile_color() {
            // 创建测试文件路径
            let path = PathBuf::from("/tmp/testfile.tmp");

            // 构造 PathData
            let path_data = PathData {
                md: OnceCell::new(),
                ft: OnceCell::new(),
                de: None,
                display_name: OsString::from("testfile.tmp"), // 明确指定扩展名以匹配颜色规则
                p_buf: path.clone(),
                is_must_dereference: false,
                security_context: String::new(),
                is_command_line: false,
            };

            // 设定 LsColors
            let ls_colors = LsColors::from_string("*.tmp=01;32"); // 确保这个规则被正确解析
            let mut style_manager = StyleManager::new();
            let mut output = vec![];

            let colored_name = color_name(
                "testfile.tmp".to_string(),
                &path_data,
                &ls_colors,
                &mut style_manager,
                &mut output,
                None,
            );

            // 验证输出是否包含了正确的 ANSI 颜色代码，接受以额外的重置代码开头的结果
            assert!(
                colored_name == "\x1b[01;32mtestfile.tmp\x1b[0m"
                    || colored_name == "\x1b[0m\x1b[01;32mtestfile.tmp\x1b[0m"
            );
        }

        #[test]
        fn test_display_date_various_style() {
            let tmp_dir = TempDir::new().unwrap();
            let file_path = tmp_dir.path().join("testfile");
            File::create(&file_path).unwrap();
            let metadata = fs::metadata(&file_path).unwrap();
            let mut config = setup_default_config();
            let time = metadata.modified().unwrap_or(UNIX_EPOCH);

            // 测试题 TimeStyle::FullIso
            config.time_style = LsTimeStyle::LsFullIso;
            let display_time = display_date(&metadata, &config);
            let expected_time = DateTime::<Local>::from(time)
                .format("%Y-%m-%d %H:%M:%S.%f %z")
                .to_string();
            assert_eq!(display_time, expected_time, "Failed for time style");

            // 测试题 TimeStyle::LongIso
            config.time_style = LsTimeStyle::LsLongIso;
            let display_time = display_date(&metadata, &config);
            let expected_time = DateTime::<Local>::from(time)
                .format("%Y-%m-%d %H:%M")
                .to_string();
            assert_eq!(display_time, expected_time, "Failed for time style");

            // 测试题 TimeStyle::Iso
            config.time_style = LsTimeStyle::LsIso;
            let display_time = display_date(&metadata, &config);

            let expected_time = DateTime::<Local>::from(time)
                .format("%m-%d %H:%M")
                .to_string();
            assert_eq!(display_time, expected_time, "Failed for time style");

            // 测试题 TimeStyle::Locale
            config.time_style = LsTimeStyle::LsLocale;
            let display_time = display_date(&metadata, &config);
            let expected_time = DateTime::<Local>::from(time)
                .format("%b %e %H:%M")
                .to_string();
            assert_eq!(display_time, expected_time, "Failed for time style");

            // 测试题 TimeStyle::Format
            config.time_style = LsTimeStyle::LsFormat("%Y-%m-%d %H:%M:%S.%f %z".to_string());
            let display_time = display_date(&metadata, &config);
            let expected_time = DateTime::<Local>::from(time)
                .format("%Y-%m-%d %H:%M:%S.%f %z")
                .to_string();
            assert_eq!(display_time, expected_time, "Failed for time style");
        }

        // 模拟创建PathData
        fn setup_path_data(name: &str, _is_dir: bool) -> PathData {
            let dir_path = PathBuf::from("/fake/directory/").join(name);
            let path_data = PathData {
                md: OnceCell::new(),
                ft: OnceCell::new(),
                de: None,
                display_name: OsString::from(name),
                p_buf: dir_path.clone(),
                is_must_dereference: false,
                security_context: String::new(),
                is_command_line: false,
            };

            // 模拟 Metadata 和 FileType
            let _ = path_data
                .md
                .set(Some(Metadata::from(fs::metadata("/dev/null").unwrap())));
            let _ = path_data.ft.set(Some(FileType::from(
                fs::metadata("/dev/null").unwrap().file_type(),
            )));

            path_data
        }

        #[test]
        fn test_padding_calculation() {
            let config = setup_default_config();
            let items = [
                setup_path_data("file1.txt", false),
                setup_path_data("file2.txt", false),
                setup_path_data("dir1", true),
            ];
            let mut out = BufWriter::new(Cursor::new(Vec::new()));
            let padding = calculate_padding_collection(&items, &config, &mut out);
            println!(
                "link_count {}, uname {}, group {}",
                padding.link_count, padding.uname, padding.group
            );
            assert_eq!(padding.link_count, 1); // 假设每个文件的链接计数为1
            assert_eq!(padding.uname, 1); // 应该基于用户名长度计算
            assert_eq!(padding.group, 1); // 同上
        }

        #[test]
        fn test_with_inode_and_alloc_size_enabled() {
            let temp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let base_path = temp_dir.path();

            // 创建文件和目录
            let file_path = base_path.join("file.txt");
            fs::write(&file_path, "This is a test file.").unwrap();

            let config = LsConfig {
                is_inode: true,
                is_alloc_size: true,
                ..setup_default_config()
            };
            let items = [setup_path_data(file_path.to_str().unwrap(), false)];
            let mut out = BufWriter::new(Vec::new());
            let padding = calculate_padding_collection(&items, &config, &mut out);

            // 检查 inode 和 alloc_size 是否影响了 padding 计算
            println!(
                "link_count {}, uname {}, group {}",
                padding.link_count, padding.uname, padding.group
            );
            assert!(padding.inode >= 1);
            assert!(padding.block_size >= 1);
            assert!(padding.uname >= 1);
        }

        #[test]
        fn test_extremely_long_contexts() {
            let temp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let base_path = temp_dir.path();
            let long_filename = "a".repeat(255); // 创建一个非常长的文件名
            let file_path = base_path.join(&long_filename);
            File::create(&file_path).unwrap();
            let file_context = "This is a test file with a very long name.";
            fs::write(&file_path, file_context).unwrap();

            let config = setup_default_config();
            let items = [setup_path_data(file_path.to_str().unwrap(), false)];
            let mut out = BufWriter::new(Vec::new());
            let padding = calculate_padding_collection(&items, &config, &mut out);
            println!("context {}, len {}, ", padding.context, file_context.len());
            // 验证是否正确处理了长文件名
            assert!(padding.context >= 1);
        }

        #[test]
        fn test_symbolic_links() {
            let temp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let base_path = temp_dir.path();

            let target_path = base_path.join("target.txt");
            let link_path = base_path.join("link.txt");
            File::create(&target_path).unwrap();
            fs::write(&target_path, "Link target").unwrap();
            std::os::unix::fs::symlink(&target_path, &link_path).unwrap();

            let config = setup_default_config();
            let items = [setup_path_data(link_path.to_str().unwrap(), false)];
            let mut out = BufWriter::new(Vec::new());
            let padding = calculate_padding_collection(&items, &config, &mut out);
            println!(
                "link_count {}, uname {}, group {}",
                padding.link_count, padding.uname, padding.group
            );
            // 检查链接是否被正确处理
            assert!(padding.uname >= 1);
            assert!(padding.inode >= 1);
            assert!(padding.block_size >= 1);
        }

        #[test]
        fn test_display_inode() {
            let temp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let base_path = temp_dir.path();

            let target_path = base_path.join("target.txt");
            File::create(&target_path).unwrap();
            let metadata = fs::metadata(&target_path).expect("Failed to get metadata");

            let expected_output = get_inode(&metadata).to_string();
            let actual_output = display_inode(&metadata);

            assert_eq!(expected_output, actual_output);
        }

        #[test]
        fn test_display_symlink_count() {
            // Test case 1: Normal case
            let temp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let base_path = temp_dir.path();

            let target_path = base_path.join("target.txt");
            File::create(&target_path).unwrap();
            let metadata = fs::metadata(&target_path).expect("Failed to get metadata");

            let expected_output = metadata.nlink().to_string();
            let actual_output = display_symlink_count(&metadata);

            assert_eq!(expected_output, actual_output);
        }

        #[test]
        fn test_display_date() {
            let content = "hello world\nhello rust\n";
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            let mut file = File::create(&test_file_path).unwrap();
            file.write_all(content.as_bytes()).unwrap();

            let metadata = fs::metadata(temp_dir_path).unwrap();
            let config = LsConfig {
                // 创建一个具体的 Config 实例
                format: LsFormat::Columns,
                files: LsFiles::LsNormal,
                sort: LsSort::Name,
                is_recursive: true,
                is_reverse: false,
                dereference: LsDereference::LsNone,
                ignore_patterns: Vec::new(),
                size_format: LsSizeFormat::Decimal,
                is_directory: false,
                time: LsTime::LsAccess,
                is_inode: false,
                color: None,
                long: LsLongFormat {
                    is_author: true,
                    is_group: true,
                    is_owner: true,
                    #[cfg(unix)]
                    is_numeric_uid_gid: true,
                },
                is_alloc_size: false,
                file_size_block_size: 512,
                block_size: 4096,
                width: 80,
                quoting_style: CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: true,
                },
                indicator_style: LsIndicatorStyle::None,
                time_style: LsTimeStyle::LsLocale,
                is_context: false,
                is_selinux_supported: false,
                is_group_directories_first: false,
                line_ending: CtLineEnding::Newline,
                is_dired: true,
                is_hyperlink: false,
            };

            // 获取文件的修改时间，确保它是当前的时间
            if let Ok(modified) = metadata.modified() {
                // 将 SystemTime 转换为 DateTime<Local>
                let time: DateTime<Local> = modified.into();

                // 因为使用了 Locale，我们应该根据你的本地环境设定预期输出
                // 注意，这种预期输出依赖于你的系统设置和时区
                let formatted_time = time.format("%b %e %H:%M").to_string();
                let display_time = display_date(&metadata, &config);

                assert_eq!(display_time, formatted_time);
            }
        }

        #[test]
        fn test_display_size() {
            // SizeFormat::Binary
            let mut config = LsConfig {
                // 创建一个具体的 Config 实例
                format: LsFormat::Columns,
                files: LsFiles::LsNormal,
                sort: LsSort::Name,
                is_recursive: true,
                is_reverse: false,
                dereference: LsDereference::LsNone,
                ignore_patterns: Vec::new(),
                size_format: LsSizeFormat::Binary,
                is_directory: false,
                time: LsTime::LsAccess,
                is_inode: false,
                color: None,
                long: LsLongFormat {
                    is_author: true,
                    is_group: true,
                    is_owner: true,
                    #[cfg(unix)]
                    is_numeric_uid_gid: true,
                },
                is_alloc_size: false,
                file_size_block_size: 512,
                block_size: 4096,
                width: 80,
                quoting_style: CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: true,
                },
                indicator_style: LsIndicatorStyle::None,
                time_style: LsTimeStyle::LsLocale,
                is_context: false,
                is_selinux_supported: false,
                is_group_directories_first: false,
                line_ending: CtLineEnding::Newline,
                is_dired: true,
                is_hyperlink: false,
            };
            // 测试二进制前缀
            assert_eq!(display_size(999, &config), "999");
            assert_eq!(display_size(1000, &config), "1000");
            assert_eq!(display_size(1023, &config), "1023");
            assert_eq!(display_size(1024, &config), "1.0K");
            assert_eq!(display_size(2048, &config), "2.0K");
            assert_eq!(display_size(999_999, &config), "977K");
            assert_eq!(display_size(1_000_000, &config), "977K");
            assert_eq!(display_size(1_048_576, &config), "1.0M");
            assert_eq!(display_size(999_999_999, &config), "954M");
            assert_eq!(display_size(1_000_000_000, &config), "954M");
            assert_eq!(display_size(1_073_741_824, &config), "1.0G");
            assert_eq!(display_size(999_999_999_999, &config), "932G");
            assert_eq!(display_size(1_000_000_000_000, &config), "932G");
            assert_eq!(display_size(1_099_511_627_776, &config), "1.0T");
            // SizeFormat::Decima 测试
            config.size_format = LsSizeFormat::Decimal;
            // 测试十进制前缀
            assert_eq!(display_size(999, &config), "999");
            assert_eq!(display_size(1000, &config), "1.0k");
            assert_eq!(display_size(1023, &config), "1.1k");
            assert_eq!(display_size(1024, &config), "1.1k");
            assert_eq!(display_size(2048, &config), "2.1k");
            assert_eq!(display_size(999_999, &config), "1000k");
            assert_eq!(display_size(1_000_000, &config), "1.0M");
            assert_eq!(display_size(1_048_576, &config), "1.1M");
            assert_eq!(display_size(999_999_999, &config), "1000M");
            assert_eq!(display_size(1_000_000_000, &config), "1.0G");
            assert_eq!(display_size(1_073_741_824, &config), "1.1G");
            assert_eq!(display_size(999_999_999_999, &config), "1000G");
            assert_eq!(display_size(1_000_000_000_000, &config), "1.0T");
            assert_eq!(display_size(1_099_511_627_776, &config), "1.1T");

            // SizeFormat::Binary 测试
            config.size_format = LsSizeFormat::Binary;
            // 测试bin前缀
            assert_eq!(display_size(999, &config), "999");
            assert_eq!(display_size(1000, &config), "1000");
            assert_eq!(display_size(1023, &config), "1023");
            assert_eq!(display_size(1024, &config), "1.0K");
            assert_eq!(display_size(2048, &config), "2.0K");
            assert_eq!(display_size(999_999, &config), "977K");
            assert_eq!(display_size(1_000_000, &config), "977K");
            assert_eq!(display_size(1_048_576, &config), "1.0M");
            assert_eq!(display_size(999_999_999, &config), "954M");
            assert_eq!(display_size(1_000_000_000, &config), "954M");
            assert_eq!(display_size(1_073_741_824, &config), "1.0G");
            assert_eq!(display_size(999_999_999_999, &config), "932G");
            assert_eq!(display_size(1_000_000_000_000, &config), "932G");
            assert_eq!(display_size(1_099_511_627_776, &config), "1.0T");
        }

        #[test]
        fn test_inode_and_uname_display() {
            let content = "hello world\nhello rust\n";
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            let mut file = File::create(&test_file_path).unwrap();
            file.write_all(content.as_bytes()).unwrap();

            let metadata = fs::metadata(temp_dir_path).unwrap();
            let mut config = LsConfig {
                // 创建一个具体的 Config 实例
                format: LsFormat::Columns,
                files: LsFiles::LsNormal,
                sort: LsSort::Name,
                is_recursive: true,
                is_reverse: false,
                dereference: LsDereference::LsNone,
                ignore_patterns: Vec::new(),
                size_format: LsSizeFormat::Decimal,
                is_directory: false,
                time: LsTime::LsAccess,
                is_inode: false,
                color: None,
                long: LsLongFormat {
                    is_author: true,
                    is_group: true,
                    is_owner: true,
                    #[cfg(unix)]
                    is_numeric_uid_gid: true,
                },
                is_alloc_size: false,
                file_size_block_size: 512,
                block_size: 4096,
                width: 80,
                quoting_style: CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: true,
                },
                indicator_style: LsIndicatorStyle::None,
                time_style: LsTimeStyle::LsLocale,
                is_context: false,
                is_selinux_supported: false,
                is_group_directories_first: false,
                line_ending: CtLineEnding::Newline,
                is_dired: true,
                is_hyperlink: false,
            };
            assert_eq!(
                display_uname(&metadata, &config),
                metadata.uid().to_string()
            );
            assert_eq!(get_inode(&metadata), metadata.ino().to_string());

            // 修改配置测试username显示
            config.long.is_numeric_uid_gid = false;
            if let Some(user) = get_user_by_uid(metadata.uid()) {
                let username = user.name().to_string_lossy().into_owned();
                assert_eq!(display_uname(&metadata, &config), username);
            } else {
                panic!("无法找到 UID 为 {} 的用户", metadata.uid());
            }
        }

        #[test]
        fn test_display_group_inode() {
            let content = "hello world\nhello rust\n";
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            let mut file = File::create(&test_file_path).unwrap();
            file.write_all(content.as_bytes()).unwrap();

            let metadata = fs::metadata(temp_dir_path).unwrap();
            let mut config = LsConfig {
                // 创建一个具体的 Config 实例
                format: LsFormat::Columns,
                files: LsFiles::LsNormal,
                sort: LsSort::Name,
                is_recursive: true,
                is_reverse: false,
                dereference: LsDereference::LsNone,
                ignore_patterns: Vec::new(),
                size_format: LsSizeFormat::Decimal,
                is_directory: false,
                time: LsTime::LsAccess,
                is_inode: false,
                color: None,
                long: LsLongFormat {
                    is_author: true,
                    is_group: true,
                    is_owner: true,
                    #[cfg(unix)]
                    is_numeric_uid_gid: true,
                },
                is_alloc_size: false,
                file_size_block_size: 512,
                block_size: 4096,
                width: 80,
                quoting_style: CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: true,
                },
                indicator_style: LsIndicatorStyle::None,
                time_style: LsTimeStyle::LsLocale,
                is_context: false,
                is_selinux_supported: false,
                is_group_directories_first: false,
                line_ending: CtLineEnding::Newline,
                is_dired: true,
                is_hyperlink: false,
            };
            assert_eq!(
                display_group(&metadata, &config),
                metadata.uid().to_string()
            );
            assert_eq!(get_inode(&metadata), metadata.ino().to_string());

            // 修改配置测试username显示
            config.long.is_numeric_uid_gid = false;
            if let Some(user) = get_user_by_uid(metadata.uid()) {
                let username = user.name().to_string_lossy().into_owned();
                assert_eq!(display_group(&metadata, &config), username);
            } else {
                panic!("无法找到 UID 为 {} 的用户组", metadata.uid());
            }
        }

        #[test]
        fn test_display_date_with_various_type() {
            use std::time::UNIX_EPOCH;
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let test_file_path = tmp_dir.path().join("date_test_file");
            let _ = fs::write(&test_file_path, b"content");
            let metadata = fs::metadata(&test_file_path).unwrap();

            // 测试 Time::Access
            let mut config = LsConfig {
                format: LsFormat::Columns,
                files: LsFiles::LsNormal,
                sort: LsSort::Name,
                is_recursive: true,
                is_reverse: false,
                dereference: LsDereference::LsNone,
                ignore_patterns: Vec::new(),
                size_format: LsSizeFormat::Decimal,
                is_directory: false,
                time: LsTime::LsAccess,
                is_inode: false,
                color: None,
                long: LsLongFormat {
                    is_author: true,
                    is_group: true,
                    is_owner: true,
                    is_numeric_uid_gid: true,
                },
                is_alloc_size: false,
                file_size_block_size: 512,
                block_size: 4096,
                width: 80,
                quoting_style: CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: true,
                },
                indicator_style: LsIndicatorStyle::None,
                time_style: LsTimeStyle::LsFullIso,
                is_context: false,
                is_selinux_supported: false,
                is_group_directories_first: false,
                line_ending: CtLineEnding::Newline,
                is_dired: true,
                is_hyperlink: false,
            };

            // 根据配置的时间类型获取相应的时间
            let mut time = metadata.accessed().unwrap_or(UNIX_EPOCH);
            // 将 SystemTime 转换为 DateTime<Local>
            let expected_time = DateTime::<Local>::from(time)
                .format("%Y-%m-%d %H:%M:%S.%f %z")
                .to_string();
            let display_time = display_date(&metadata, &config);
            assert_eq!(display_time, expected_time, "Failed for time format");

            // 测试 Time::Modification
            config.time = LsTime::LsModification;
            time = metadata.modified().unwrap_or(UNIX_EPOCH);
            let expected_time = DateTime::<Local>::from(time)
                .format("%Y-%m-%d %H:%M:%S.%f %z")
                .to_string();
            let display_time = display_date(&metadata, &config);
            assert_eq!(display_time, expected_time, "Failed for time format");

            // 测试 Time::Change
            config.time = LsTime::LsChange;
            time = metadata.modified().unwrap_or(UNIX_EPOCH);
            let expected_time = DateTime::<Local>::from(time)
                .format("%Y-%m-%d %H:%M:%S.%f %z")
                .to_string();
            let display_time = display_date(&metadata, &config);
            assert_eq!(display_time, expected_time, "Failed for time format");

            config.time = LsTime::LsBirth;
            if metadata.created().is_err() {
                let expected_time = "???".to_string();
                let display_time = display_date(&metadata, &config);

                assert_eq!(display_time, expected_time, "Failed for time format");
            }
        }

        #[test]
        fn test_format_usage() {
            let usage = ct_format_usage("Usage: ls [OPTION]... [FILE]...");
            assert!(usage.contains("Usage: ls [OPTION]... [FILE]..."));
        }

        #[test]
        fn test_response_to_nonexistent_file() {
            let non_existent_path = PathBuf::from("/tmp/nonexistent_file_or_dir");
            let config = LsConfig {
                format: LsFormat::Columns,
                files: LsFiles::LsNormal,
                sort: LsSort::Name,
                is_recursive: true,
                is_reverse: false,
                dereference: LsDereference::LsNone,
                ignore_patterns: Vec::new(),
                size_format: LsSizeFormat::Decimal,
                is_directory: false,
                time: LsTime::LsAccess,
                is_inode: false,
                color: None,
                long: LsLongFormat {
                    is_author: true,
                    is_group: true,
                    is_owner: true,
                    is_numeric_uid_gid: true,
                },
                is_alloc_size: false,
                file_size_block_size: 512,
                block_size: 4096,
                width: 80,
                quoting_style: CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: true,
                    show_control: true,
                },
                indicator_style: LsIndicatorStyle::None,
                time_style: LsTimeStyle::LsLocale,
                is_context: false,
                is_selinux_supported: false,
                is_group_directories_first: false,
                line_ending: CtLineEnding::Newline,
                is_dired: true,
                is_hyperlink: false,
            };

            let metadata_result = fs::metadata(&non_existent_path);
            assert!(
                metadata_result.is_err(),
                "Metadata should not be available for nonexistent files."
            );

            let display_result = match metadata_result {
                Ok(md) => display_date(&md, &config),
                Err(_) => "File not found".to_string(),
            };
            assert_eq!(
                display_result, "File not found",
                "The function should respond correctly to nonexistent files."
            );
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::fs::File;
        use std::io::Write;

        use tempfile::TempDir;

        use super::*;

        #[test]
        fn test_ctmain_input_err_no_app_name_v() {
            let args = ["--version", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            println!("{}", result);
            assert_eq!(result, 2);
        }

        #[test]
        fn test_ctmain_input_err_no_app_name_uppercase_v() {
            let args = ["-V", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            //println!("{}", result);
            assert_eq!(result, 2);
        }

        #[test]
        fn test_ctmain_return() {
            let args = vec![ctcore::ct_util_name()];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                }
            }
        }

        #[test]
        fn test_ctmain_ls_dir_return() {
            let args = vec![ctcore::ct_util_name(), "./"];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                }
            }
        }

        // vdir 文件测试
        #[test]
        fn test_ct_main_with_ls_file() {
            let content = "hello world\nhello rust\n";
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            let mut file = File::create(&test_file_path).unwrap();
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), test_file_path.to_str().unwrap()];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    assert!(!file_vec.is_empty());
                    assert!(dir_vec.is_empty());
                }
            }
        }

        #[test]
        fn test_ct_main_execution_a() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-a", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_execution_all() {
            // 创建临时目录结构
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-all", dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_execution_block_size() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--block-size=1", dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_support_missing_argument() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--block-size=1", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_format_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--format=long", dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_columns_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-C", dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-l", dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_across_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-x", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_tab_size_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-T", "4", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_tab_size_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--tabsize=8", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_commas_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-m", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_one_line_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-1", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_no_group_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-o", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_no_owner_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-g", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_numeric_uid_gid_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-n", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_numeric_uid_gid_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--numeric-uid-gid", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_quoting_style_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--quoting-style=literal", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_literal_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-N", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_escape_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-b", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_c_quoting_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-Q", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_hide_control_chars_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-q", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_show_control_chars_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--show-control-chars", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_time_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--time=access", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_hide_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--hide=*", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_ignore_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-I", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_ignore_backups_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-B", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_change_short_c() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-c", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_access_short_u() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-u", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_ignore_short_uppercase_i() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-I", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_sort_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--sort=size", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_size_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-S", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_time_sort_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-t", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_extension_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-X", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_none_sort_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-U", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_dereference_all_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-L", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_dereference_dir_args_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                "--dereference-command-line-symlink-to-dir",
                &dir_name,
            ];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_dereference_args_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-H", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_no_group_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-G", dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                    assert_eq!(0, file_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_all_files_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-a", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_almost_all_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-A", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_directory_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-d", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(!file_vec.is_empty());
                    assert!(dir_vec.is_empty());
                    assert_eq!(0, dir_vec.len());
                    assert_eq!(1, file_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_kibibytes_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-k", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_si_long() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--si", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_inode_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-i", dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                    assert_eq!(0, file_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_reverse_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-r", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_recursive_short() {
            let file_name = "test_ls_file";
            let tmp_dir = TempDir::with_prefix("test_ls-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-R", &dir_name];
            let result = ct_main(args.iter().map(|s| OsString::from(s)));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {}", output)
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // vdir 接口: vdir [OPTION]... [FILE]...
        // List information about the FILEs (the current directory by default).
        // Sort entries alphabetically if none of -cftuvSUX nor --sort is specified.
        //
        // Mandatory arguments to long options are mandatory for short options too.
        //   -a, --all                  do not ignore entries starting with .
        //   -A, --almost-all           do not list implied . and ..
        //       --author               with -l, print the author of each file
        //   -b, --escape               print C-style escapes for nongraphic characters
        //       --block-size=SIZE      with -l, scale sizes by SIZE when printing them;
        //                                e.g., '--block-size=M'; see SIZE format below
        //   -B, --ignore-backups       do not list implied entries ending with ~
        //   -c                         with -lt: sort by, and show, ctime (time of last
        //                                modification of file status information);
        //                                with -l: show ctime and sort by name;
        //                                otherwise: sort by ctime, newest first
        //   -C                         list entries by columns
        //       --color[=WHEN]         colorize the output; WHEN can be 'always' (default
        //                                if omitted), 'auto', or 'never'; more info below
        //   -d, --directory            list directories themselves, not their contents
        //   -D, --dired                generate output designed for Emacs' dired mode
        //   -f                         list all entries in directory order
        //   -F, --classify[=WHEN]      append indicator (one of */=>@|) to entries;
        //                                WHEN can be 'always' (default if omitted),
        //                                'auto', or 'never'
        //       --file-type            likewise, except do not append '*'
        //       --format=WORD          across -x, commas -m, horizontal -x, long -l,
        //                                single-column -1, verbose -l, vertical -C
        //       --full-time            like -l --time-style=full-iso
        //   -g                         like -l, but do not list owner
        //       --group-directories-first
        //                              group directories before files;
        //                                can be augmented with a --sort option, but any
        //                                use of --sort=none (-U) disables grouping
        //   -G, --no-group             in a long listing, don't print group names
        //   -h, --human-readable       with -l and -s, print sizes like 1K 234M 2G etc.
        //       --si                   likewise, but use powers of 1000 not 1024
        //   -H, --dereference-command-line
        //                              follow symbolic links listed on the command line
        //       --dereference-command-line-symlink-to-dir
        //                              follow each command line symbolic link
        //                                that points to a directory
        //       --hide=PATTERN         do not list implied entries matching shell PATTERN
        //                                (overridden by -a or -A)
        //       --hyperlink[=WHEN]     hyperlink file names; WHEN can be 'always'
        //                                (default if omitted), 'auto', or 'never'
        //       --indicator-style=WORD  append indicator with style WORD to entry names:
        //                                none (default), slash (-p),
        //                                file-type (--file-type), classify (-F)
        //   -i, --inode                print the index number of each file
        //   -I, --ignore=PATTERN       do not list implied entries matching shell PATTERN
        //   -k, --kibibytes            default to 1024-byte blocks for file system usage;
        //                                used only with -s and per directory totals
        //   -l                         use a long listing format
        //   -L, --dereference          when showing file information for a symbolic
        //                                link, show information for the file the link
        //                                references rather than for the link itself
        //   -m                         fill width with a comma separated list of entries
        //   -n, --numeric-uid-gid      like -l, but list numeric user and group IDs
        //   -N, --literal              print entry names without quoting
        //   -o                         like -l, but do not list group information
        //   -p, --indicator-style=slash
        //                              append / indicator to directories
        //   -q, --hide-control-chars   print ? instead of nongraphic characters
        //       --show-control-chars   show nongraphic characters as-is (the default,
        //                                unless program is 'ls' and output is a terminal)
        //   -Q, --quote-name           enclose entry names in double quotes
        //       --quoting-style=WORD   use quoting style WORD for entry names:
        //                                literal, locale, shell, shell-always,
        //                                shell-escape, shell-escape-always, c, escape
        //                                (overrides QUOTING_STYLE environment variable)
        //   -r, --reverse              reverse order while sorting
        //   -R, --recursive            list subdirectories recursively
        //   -s, --size                 print the allocated size of each file, in blocks
        //   -S                         sort by file size, largest first
        //       --sort=WORD            sort by WORD instead of name: none (-U), size (-S),
        //                                time (-t), version (-v), extension (-X), width
        //       --time=WORD            change the default of using modification times;
        //                                access time (-u): atime, access, use;
        //                                change time (-c): ctime, status;
        //                                birth time: birth, creation;
        //                              with -l, WORD determines which time to show;
        //                              with --sort=time, sort by WORD (newest first)
        //       --time-style=TIME_STYLE  time/date format with -l; see TIME_STYLE below
        //   -t                         sort by time, newest first; see --time
        //   -T, --tabsize=COLS         assume tab stops at each COLS instead of 8
        //   -u                         with -lt: sort by, and show, access time;
        //                                with -l: show access time and sort by name;
        //                                otherwise: sort by access time, newest first
        //   -U                         do not sort; list entries in directory order
        //   -v                         natural sort of (version) numbers within text
        //   -w, --width=COLS           set output width to COLS.  0 means no limit
        //   -x                         list entries by lines instead of by columns
        //   -X                         sort alphabetically by entry extension
        //   -Z, --context              print any security context of each file
        //       --zero                 end each output line with NUL, not newline
        //   -1                         list one file per line
        //       --help     display this help and exit
        //       --version  output version information and exit

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "--version"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "-V"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            // 测试用例3：验证当提供未知参数时是否正确报错
            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            // 测试用例4：验证当缺少必需的参数时是否正确报错
            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_format_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=long"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::LS_FORMAT));
            assert!(matches.unwrap().contains_id(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_ct_app_format_verbose_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=verbose"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::LS_FORMAT));
            assert!(matches.unwrap().contains_id(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_ct_app_format_single_column_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=single-column"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::LS_FORMAT));
            assert!(matches.unwrap().contains_id(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_ct_app_format_columns_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=columns"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::LS_FORMAT));
            assert!(matches.unwrap().contains_id(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_ct_app_format_vertical_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=vertical"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::LS_FORMAT));
            assert!(matches.unwrap().contains_id(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_ct_app_format_across_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=across"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::LS_FORMAT));
            assert!(matches.unwrap().contains_id(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_ct_app_format_horizontal_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=horizontal"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::LS_FORMAT));
            assert!(matches.unwrap().contains_id(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_ct_app_format_commas_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=commas"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::LS_FORMAT));
            assert!(matches.unwrap().contains_id(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_ct_app_columns_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-C"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_COLUMNS));
        }

        #[test]
        fn test_ct_app_long_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-l"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_LONG));
        }

        #[test]
        fn test_ct_app_long_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--long"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_LONG));
        }

        #[test]
        fn test_ct_app_across_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-x"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_ACROSS));
        }

        #[test]
        fn test_ct_app_tab_size_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-T", "4"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert_eq!(
                matches
                    .unwrap()
                    .get_one::<String>(ls_flags::format::LS_TAB_SIZE)
                    .unwrap(),
                "4"
            );
        }

        #[test]
        fn test_ct_app_tab_size_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--tabsize=8"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert_eq!(
                matches
                    .unwrap()
                    .get_one::<String>(ls_flags::format::LS_TAB_SIZE)
                    .unwrap(),
                "8"
            );
        }

        #[test]
        fn test_ct_app_commas_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-m"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_COMMAS));
        }

        #[test]
        fn test_ct_app_zero_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--zero"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_COMMAS));
        }

        #[test]
        fn test_ct_app_one_line_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-1"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_ONE_LINE));
        }

        #[test]
        fn test_ct_app_long_no_group_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-o"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::format::LS_LONG_NO_GROUP));
        }

        #[test]
        fn test_ct_app_long_no_owner_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-g"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::format::LS_LONG_NO_OWNER));
        }

        #[test]
        fn test_ct_app_long_no_owner_short_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-g", "--help"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
            assert_eq!(
                matches.unwrap_err().kind(),
                clap::error::ErrorKind::DisplayHelp
            );
        }

        #[test]
        fn test_ct_app_long_numeric_uid_gid_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-n"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::format::LS_LONG_NUMERIC_UID_GID));
        }

        #[test]
        fn test_ct_app_long_numeric_uid_gid_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--numeric-uid-gid"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::format::LS_LONG_NUMERIC_UID_GID));
        }

        #[test]
        fn test_ct_app_quoting_style_literal_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--quoting-style=literal"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_QUOTING_STYLE));
        }

        #[test]
        fn test_ct_app_quoting_style_escape_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--quoting-style=escape"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_QUOTING_STYLE));
        }

        #[test]
        fn test_ct_app_quoting_style_c_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--quoting-style=c"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_QUOTING_STYLE));
        }

        #[test]
        fn test_ct_app_quoting_style_shell_escape_always_long() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--quoting-style=shell-escape-always",
            ];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_QUOTING_STYLE));
        }

        #[test]
        fn test_ct_app_quoting_style_shell_escape_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--quoting-style=shell-escape"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_QUOTING_STYLE));
        }

        #[test]
        fn test_ct_app_quoting_style_shell_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--quoting-style=shell"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_QUOTING_STYLE));
        }

        #[test]
        fn test_ct_app_quoting_style_shell_always_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--quoting-style=shell-always"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_QUOTING_STYLE));
        }

        #[test]
        fn test_ct_app_literal_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-N"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::quoting::LS_LITERAL));
        }

        #[test]
        fn test_ct_app_literal_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--literal"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::quoting::LS_LITERAL));
        }

        #[test]
        fn test_ct_app_escape_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-b"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::quoting::LS_ESCAPE));
        }

        #[test]
        fn test_ct_app_escape_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--escape"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::quoting::LS_ESCAPE));
        }

        #[test]
        fn test_ct_app_c_quoting_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-Q"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::quoting::LS_C));
        }

        #[test]
        fn test_ct_app_c_quoting_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--quote-name"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::quoting::LS_C));
        }

        #[test]
        fn test_ct_app_hide_control_chars_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-q"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::LS_HIDE_CONTROL_CHARS));
        }

        #[test]
        fn test_ct_app_hide_control_chars_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hide-control-chars"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::LS_HIDE_CONTROL_CHARS));
        }

        #[test]
        fn test_ct_app_show_control_chars_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--show-control-chars"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::LS_SHOW_CONTROL_CHARS));
        }

        #[test]
        fn test_ct_app_time_access_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time=access"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_TIME));
        }

        #[test]
        fn test_ct_app_time_atime_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time=atime"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_TIME));
        }

        #[test]
        fn test_ct_app_time_use_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time=use"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_TIME));
        }

        #[test]
        fn test_ct_app_time_ctime_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time=ctime"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_TIME));
        }

        #[test]
        fn test_ct_app_time_status_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time=status"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_TIME));
        }

        #[test]
        fn test_ct_app_time_birth_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time=birth"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_TIME));
        }

        #[test]
        fn test_ct_app_time_creation_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time=creation"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_TIME));
        }

        #[test]
        fn test_ct_app_change_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-c"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::time::LS_CHANGE));
        }

        #[test]
        fn test_ct_app_access_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-u"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::time::LS_ACCESS));
        }

        #[test]
        fn test_ct_app_hide_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hide=*"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HIDE));
        }

        #[test]
        fn test_ct_app_ignore_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-I", "*.tmp"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_IGNORE));
        }

        #[test]
        fn test_ct_app_ignore_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--ignore", "*.tmp"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_IGNORE));
        }

        #[test]
        fn test_ct_app_ignore_backups_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-B"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_IGNORE_BACKUPS));
        }

        #[test]
        fn test_ct_app_ignore_backups_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--ignore-backups"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_IGNORE_BACKUPS));
        }

        #[test]
        fn test_ct_app_change_short_c_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-c", "--help"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
            assert_eq!(matches.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_access_short_u_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-u", "--help"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
            assert_eq!(matches.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_ignore_short_uppercase_i_txt() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-I", "*.txt"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_IGNORE));
        }

        #[test]
        fn test_ct_app_sort_size_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=size"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_SORT));
        }

        #[test]
        fn test_ct_app_sort_name_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=name"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_SORT));
        }

        #[test]
        fn test_ct_app_sort_none_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=none"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_SORT));
        }

        #[test]
        fn test_ct_app_sort_time_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=time"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_SORT));
        }

        #[test]
        fn test_ct_app_sort_extension_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=extension"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_SORT));
        }

        #[test]
        fn test_ct_app_sort_width_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=width"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_SORT));
        }

        #[test]
        fn test_ct_app_size_uppercase_s_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-S"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_SIZE));
        }

        #[test]
        fn test_ct_app_time_sort_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-t"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_TIME));
        }

        #[test]
        fn test_ct_app_version_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-v"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_VERSION));
        }

        #[test]
        fn test_ct_app_extension_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-X"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_EXTENSION));
        }

        #[test]
        fn test_ct_app_none_sort_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-U"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_NONE));
        }

        #[test]
        fn test_ct_app_dereference_all_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-L"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::dereference::LS_ALL));
        }

        #[test]
        fn test_ct_app_dereference_all_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--dereference"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::dereference::LS_ALL));
        }

        #[test]
        fn test_ct_app_dereference_dir_args_long() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--dereference-command-line-symlink-to-dir",
            ];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::dereference::LS_DIR_ARGS));
        }

        #[test]
        fn test_ct_app_dereference_args_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-H"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::dereference::LS_ARGS));
        }

        #[test]
        fn test_ct_app_dereference_args_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--dereference-command-line"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::dereference::LS_ARGS));
        }

        #[test]
        fn test_ct_app_author_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--author"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_NO_GROUP));
        }

        #[test]
        fn test_ct_app_no_group_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-G"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_NO_GROUP));
        }

        #[test]
        fn test_ct_app_no_group_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--no-group"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_NO_GROUP));
        }

        #[test]
        fn test_ct_app_all_files_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-a"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::files::LS_ALL));
        }

        #[test]
        fn test_ct_app_all_files_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--all"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::files::LS_ALL));
        }

        #[test]
        fn test_ct_app_almost_all_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-A"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::files::LS_ALMOST_ALL));
        }

        #[test]
        fn test_ct_app_almost_all_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--almost-all"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::files::LS_ALMOST_ALL));
        }

        #[test]
        fn test_ct_app_directory_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_DIRECTORY));
        }

        #[test]
        fn test_ct_app_directory_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--directory"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_DIRECTORY));
        }

        #[test]
        fn test_ct_app_human_readable_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::size::LS_HUMAN_READABLE));
        }

        #[test]
        fn test_ct_app_human_readable_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--human-readable"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::size::LS_HUMAN_READABLE));
        }

        #[test]
        fn test_ct_app_kibibytes_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-k"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_KIBIBYTES));
        }

        #[test]
        fn test_ct_app_kibibytes_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--kibibytes"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_KIBIBYTES));
        }

        #[test]
        fn test_ct_app_si_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--si"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_SI));
        }

        #[test]
        fn test_ct_app_k_si_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-k", "--si"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_SI));
        }

        #[test]
        fn test_ct_app_k_block_size_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-k", "--block-size=128"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_SI));
        }

        #[test]
        fn test_ct_app_k_si_block_size_long() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-k",
                "-si",
                "--block-size=102400000",
            ];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_SI));
        }

        #[test]
        fn test_ct_app_block_size_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--block-size=1024"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_BLOCK_SIZE));
        }

        #[test]
        fn test_ct_app_inode_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-i"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_INODE));
        }

        #[test]
        fn test_ct_app_inode_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--inode"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_INODE));
        }

        #[test]
        fn test_ct_app_reverse_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-r"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_REVERSE));
        }

        #[test]
        fn test_ct_app_reverse_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--reverse"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_REVERSE));
        }

        #[test]
        fn test_ct_app_recursive_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-R"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_RECURSIVE));
        }

        #[test]
        fn test_ct_app_recursive_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--recursive"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_RECURSIVE));
        }

        #[test]
        fn test_ct_app_columns_wide_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-w", "11"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_WIDTH));
        }

        #[test]
        fn test_ct_app_columns_wide_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--width=11"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_WIDTH));
        }

        #[test]
        fn test_ct_app_size_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-s"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::size::LS_ALLOCATION_SIZE));
        }

        #[test]
        fn test_ct_app_size_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--size"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::size::LS_ALLOCATION_SIZE));
        }

        #[test]
        fn test_ct_app_size_color_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_color_none_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color=none"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_size_color_always_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color=always"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_color_yes_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color=yes"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_color_force_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color=force"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_color_auto_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color=auto"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_color_tty_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color=tty"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_color_if_tty_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color=if-tty"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_color_never_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color=never"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_color_no_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--color=no"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_COLOR));
        }

        #[test]
        fn test_ct_app_indicator_style_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--indicator-style"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
            assert_eq!(matches.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_indicator_style_none_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--indicator-style=none"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_INDICATOR_STYLE));
        }

        #[test]
        fn test_ct_app_indicator_style_slash_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--indicator-style=slash"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_INDICATOR_STYLE));
        }

        #[test]
        fn test_ct_app_indicator_style_file_type_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--indicator-style=file-type"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_INDICATOR_STYLE));
        }

        #[test]
        fn test_ct_app_indicator_style_classify_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--indicator-style=classify"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_INDICATOR_STYLE));
        }

        #[test]
        fn test_ct_app_classify_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-F"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_none_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify=none"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_no_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify=no"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_never_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify=never"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_if_tty_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify=if-tty"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_tty_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify=tty"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_auto_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify=auto"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_force_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify=force"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_yes_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify=yes"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_classify_always_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--classify=always"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_CLASSIFY));
        }

        #[test]
        fn test_ct_app_file_types_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file-type"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_FILE_TYPE));
        }

        #[test]
        fn test_ct_app_indicator_directories_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-p"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::indicator_style::LS_SLASH));
        }

        #[test]
        fn test_ct_app_time_style_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time-style"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
            assert_eq!(matches.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_time_style_full_iso_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time-style=full-iso"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_FULL_TIME));
        }

        #[test]
        fn test_ct_app_time_style_long_iso_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time-style=long-iso"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_FULL_TIME));
        }

        #[test]
        fn test_ct_app_time_style_iso_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time-style=iso"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_FULL_TIME));
        }

        #[test]
        fn test_ct_app_time_style_locale_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time-style=locale"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_FULL_TIME));
        }

        #[test]
        fn test_ct_app_full_time_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--full-time"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_FULL_TIME));
        }

        #[test]
        fn test_ct_app_context_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-Z"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_CONTEXT));
        }

        #[test]
        fn test_ct_app_context_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--context"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_CONTEXT));
        }

        #[test]
        fn test_ct_app_group_directories_first_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--group-directories-first"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches
                .unwrap()
                .contains_id(ls_flags::LS_GROUP_DIRECTORIES_FIRST));
        }

        #[test]
        fn test_ct_app_dired_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--dired"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_DIRED));
        }

        #[test]
        fn test_ct_app_dired_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-D"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_DIRED));
        }

        #[test]
        fn test_ct_app_hyperlink_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }

        #[test]
        fn test_ct_app_hyperlink_always_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink=always"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }

        #[test]
        fn test_ct_app_hyperlink_yes_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink=yes"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }

        #[test]
        fn test_ct_app_hyperlink_force_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink=force"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }

        #[test]
        fn test_ct_app_hyperlink_auto_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink=auto"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }

        #[test]
        fn test_ct_app_hyperlink_tty_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink=tty"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }

        #[test]
        fn test_ct_app_hyperlink_if_tty_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink=if-tty"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }

        #[test]
        fn test_ct_app_hyperlink_never_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink=never"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }

        #[test]
        fn test_ct_app_hyperlink_no_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink=no"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }

        #[test]
        fn test_ct_app_hyperlink_none_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hyperlink=none"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HYPERLINK));
        }
    }
}
