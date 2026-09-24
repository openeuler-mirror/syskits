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

//! touch 用于更新文件或目录访问和修改时间戳的命令行工具。
//! 如果指定的文件不存在，touch命令会创建一个空文件。
//! 主要用于以下几种情况：
//! 1.创建新文件：当仅需创建一个空文件时，无需编辑内容，直接使用touch命令即可。
//! 2.更新时间戳：可以用来更新文件的访问时间和修改时间（atime和mtime），使之看起来像是最近被访问或修改过。

extern crate rust_i18n;
use rust_i18n::t;
use std::borrow::Cow;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
rust_i18n::i18n!("locales", fallback = "en-US");
use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone, Timelike,
};
use clap::builder::ValueParser;
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command, crate_version};
use filetime::{FileTime, set_file_times, set_symlink_file_times};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

use ctcore::ct_display::{Quotable, locale_quote_marks};
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo, strip_errno};
use ctcore::ct_parse_datetime;
use ctcore::ct_posix::{
    GnuGetoptCommandExt, MODERN, TRADITIONAL, ct_posix_version, posixly_correct,
};
use ctcore::ct_quoting_style::gnu_quote_shell;
use ctcore::{Tool, ct_show};

pub mod touch_flags {
    // 需要SOURCES和sources，因为我们需要能够引用ArgGroup。
    pub static TOUCH_SOURCES: &str = "sources";
    pub mod sources {
        pub static TOUCH_DATE: &str = "date";
        pub static TOUCH_REFERENCE: &str = "reference";
        pub static TOUCH_TIMESTAMP: &str = "timestamp";
    }

    pub static TOUCH_HELP: &str = "help";
    pub static TOUCH_ACCESS: &str = "access";
    pub static TOUCH_MODIFICATION: &str = "modification";
    pub static TOUCH_NO_CREATE: &str = "no-create";
    pub static TOUCH_NO_DEREF: &str = "no-dereference";
    pub static TOUCH_TIME: &str = "time";
    pub static TOUCH_FORCE: &str = "force";
}

static TOUCH_ARG_FILES: &str = "files";

const TOUCH_LONG_OPTIONS: &[(&str, bool)] = &[
    ("time", true),
    ("no-create", false),
    ("date", true),
    ("reference", true),
    ("no-dereference", false),
    ("help", false),
    ("version", false),
];
const TOUCH_SHORT_OPTIONS: &[u8] = b"acdfhmrtV";
const TOUCH_TIME_ACCESS_WORDS: &[&str] = &["atime", "access", "use"];
const TOUCH_TIME_MODIFICATION_WORDS: &[&str] = &["mtime", "modify"];

enum TouchLongOptionMatch {
    None,
    Recognized {
        canonical: &'static str,
        takes_value: bool,
    },
    Ambiguous(Vec<&'static str>),
}

#[derive(Clone, Copy)]
enum TouchPendingValue {
    Long(&'static str),
    Short(u8),
}

impl TouchPendingValue {
    fn requires_time_word_validation(self) -> bool {
        matches!(self, Self::Long("time"))
    }

    fn missing_value_diagnostic(self) -> Vec<u8> {
        match self {
            Self::Long(option) => format!("option '--{option}' requires an argument").into_bytes(),
            Self::Short(option) => {
                format!("option requires an argument -- '{}'", option as char).into_bytes()
            }
        }
    }
}

#[derive(Debug)]
struct TouchUsageError {
    message: Vec<u8>,
}

impl TouchUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

impl Display for TouchUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for TouchUsageError {}

impl CTError for TouchUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage(&self) -> bool {
        true
    }
}

fn touch_match_long_option(name: &[u8]) -> TouchLongOptionMatch {
    if let Some((canonical, takes_value)) = TOUCH_LONG_OPTIONS
        .iter()
        .find(|(option, _)| option.as_bytes() == name)
    {
        return TouchLongOptionMatch::Recognized {
            canonical,
            takes_value: *takes_value,
        };
    }

    let matches = TOUCH_LONG_OPTIONS
        .iter()
        .filter(|(option, _)| option.as_bytes().starts_with(name))
        .map(|(option, _)| *option)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => TouchLongOptionMatch::None,
        [canonical] => {
            let takes_value = TOUCH_LONG_OPTIONS
                .iter()
                .find(|(option, _)| option == canonical)
                .expect("matched long option must be declared")
                .1;
            TouchLongOptionMatch::Recognized {
                canonical,
                takes_value,
            }
        }
        _ => TouchLongOptionMatch::Ambiguous(matches),
    }
}

fn touch_prepare_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    touch_prepare_args_with_mode(args, posixly_correct())
}

fn touch_prepare_args_with_mode(
    args: impl ctcore::Args,
    posixly_correct: bool,
) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut parse_options = true;
    let mut pending_value: Option<TouchPendingValue> = None;

    for argument in args.iter().skip(1) {
        if let Some(option) = pending_value.take() {
            if option.requires_time_word_validation() {
                touch_validate_time_word(argument.as_encoded_bytes())?;
            }
            continue;
        }

        let bytes = argument.as_encoded_bytes();
        if !parse_options {
            continue;
        }
        if bytes == b"--" {
            parse_options = false;
            continue;
        }
        if bytes.len() <= 1 || bytes[0] != b'-' {
            if posixly_correct {
                parse_options = false;
            }
            continue;
        }

        if let Some(long) = bytes.strip_prefix(b"--") {
            let separator = long.iter().position(|byte| *byte == b'=');
            let name = &long[..separator.unwrap_or(long.len())];
            match touch_match_long_option(name) {
                TouchLongOptionMatch::None => {
                    let mut message = b"unrecognized option '".to_vec();
                    message.extend_from_slice(bytes);
                    message.push(b'\'');
                    return Err(TouchUsageError::boxed(message));
                }
                TouchLongOptionMatch::Ambiguous(matches) => {
                    let possibilities = matches
                        .into_iter()
                        .map(|option| format!("'--{option}'"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let mut message = b"option '".to_vec();
                    message.extend_from_slice(bytes);
                    message.extend_from_slice(b"' is ambiguous; possibilities: ");
                    message.extend_from_slice(possibilities.as_bytes());
                    return Err(TouchUsageError::boxed(message));
                }
                TouchLongOptionMatch::Recognized {
                    canonical,
                    takes_value: false,
                } if separator.is_some() => {
                    return Err(TouchUsageError::boxed(
                        format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
                    ));
                }
                TouchLongOptionMatch::Recognized {
                    canonical: "help" | "version",
                    takes_value: false,
                } => {
                    return Ok(args
                        .first()
                        .into_iter()
                        .cloned()
                        .chain(std::iter::once(argument.clone()))
                        .collect());
                }
                TouchLongOptionMatch::Recognized {
                    canonical,
                    takes_value: true,
                } if separator.is_none() => {
                    pending_value = Some(TouchPendingValue::Long(canonical));
                }
                TouchLongOptionMatch::Recognized {
                    canonical: "time",
                    takes_value: true,
                } => {
                    let value = &long[separator.expect("time option has an attached value") + 1..];
                    touch_validate_time_word(value)?;
                }
                TouchLongOptionMatch::Recognized { .. } => {}
            }
            continue;
        }

        let short_options = &bytes[1..];
        for (index, option) in short_options.iter().enumerate() {
            if matches!(option, b'd' | b'r' | b't') {
                if index + 1 == short_options.len() {
                    pending_value = Some(TouchPendingValue::Short(*option));
                }
                break;
            }
            if !TOUCH_SHORT_OPTIONS.contains(option) {
                let mut message = b"invalid option -- '".to_vec();
                message.push(*option);
                message.push(b'\'');
                return Err(TouchUsageError::boxed(message));
            }
        }
    }

    if let Some(option) = pending_value {
        return Err(TouchUsageError::boxed(option.missing_value_diagnostic()));
    }

    Ok(args)
}

fn touch_parse_time_word(value: &str) -> Result<String, String> {
    match touch_match_time_word(value.as_bytes()) {
        Ok("access") => Ok("access".to_string()),
        Ok("modify") => Ok("modify".to_string()),
        Ok(_) => unreachable!("time word matcher only returns known modes"),
        Err(_) => Err(format!("invalid time type {value:?}")),
    }
}

fn touch_validate_time_word(value: &[u8]) -> CTResult<()> {
    let Err(ambiguous) = touch_match_time_word(value) else {
        return Ok(());
    };

    let kind = if ambiguous { "ambiguous" } else { "invalid" };
    Err(TouchUsageError::boxed(
        touch_time_word_diagnostic_message(value, kind).into_bytes(),
    ))
}

fn touch_match_time_word(value: &[u8]) -> Result<&'static str, bool> {
    let access = TOUCH_TIME_ACCESS_WORDS
        .iter()
        .any(|candidate| candidate.as_bytes().starts_with(value));
    let modification = TOUCH_TIME_MODIFICATION_WORDS
        .iter()
        .any(|candidate| candidate.as_bytes().starts_with(value));

    match (access, modification) {
        (true, false) => Ok("access"),
        (false, true) => Ok("modify"),
        (true, true) => Err(true),
        (false, false) => Err(false),
    }
}

fn touch_time_word_diagnostic_message(value: &[u8], kind: &str) -> String {
    let (left_quote, right_quote) = locale_quote_marks();
    touch_time_word_diagnostic_message_with_marks(value, kind, left_quote, right_quote)
}

fn touch_time_word_diagnostic_message_with_marks(
    value: &[u8],
    kind: &str,
    left_quote: &str,
    right_quote: &str,
) -> String {
    let quote = |value| touch_quote_argmatch_bytes_with_marks(value, left_quote, right_quote);
    format!(
        "{kind} argument {} for {}\nValid arguments are:\n  - {}, {}, {}\n  - {}, {}",
        quote(value),
        quote(b"--time"),
        quote(b"atime"),
        quote(b"access"),
        quote(b"use"),
        quote(b"mtime"),
        quote(b"modify"),
    )
}

fn touch_quote_argmatch_bytes_with_marks(
    value: &[u8],
    left_quote: &str,
    right_quote: &str,
) -> String {
    let mut quoted = String::from(left_quote);

    if (left_quote, right_quote) == ("‘", "’") {
        if let Ok(value) = std::str::from_utf8(value) {
            let right_quote_character = right_quote
                .chars()
                .next()
                .expect("UTF-8 right quote must contain one character");
            for character in value.chars() {
                match character {
                    '\u{7}' => quoted.push_str("\\a"),
                    '\u{8}' => quoted.push_str("\\b"),
                    '\t' => quoted.push_str("\\t"),
                    '\n' => quoted.push_str("\\n"),
                    '\u{b}' => quoted.push_str("\\v"),
                    '\u{c}' => quoted.push_str("\\f"),
                    '\r' => quoted.push_str("\\r"),
                    '\\' => quoted.push_str("\\\\"),
                    _ if character == right_quote_character => {
                        quoted.push('\\');
                        quoted.push(character);
                    }
                    _ => quoted.push(character),
                }
            }
            quoted.push_str(right_quote);
            return quoted;
        }
    }

    for byte in value {
        match byte {
            b'\x07' => quoted.push_str("\\a"),
            b'\x08' => quoted.push_str("\\b"),
            b'\t' => quoted.push_str("\\t"),
            b'\n' => quoted.push_str("\\n"),
            b'\x0b' => quoted.push_str("\\v"),
            b'\x0c' => quoted.push_str("\\f"),
            b'\r' => quoted.push_str("\\r"),
            b'\\' => quoted.push_str("\\\\"),
            b'\'' if right_quote == "'" => quoted.push_str("\\'"),
            byte if byte.is_ascii_graphic() || *byte == b' ' => quoted.push(*byte as char),
            byte => quoted.push_str(&format!("\\{byte:03o}")),
        }
    }
    quoted.push_str(right_quote);
    quoted
}

fn touch_invalid_date_format(value: &[u8]) -> String {
    let (left_quote, right_quote) = locale_quote_marks();
    touch_invalid_date_format_with_marks(value, left_quote, right_quote)
}

fn touch_invalid_date_format_with_marks(
    value: &[u8],
    left_quote: &str,
    right_quote: &str,
) -> String {
    format!(
        "invalid date format {}",
        touch_quote_argmatch_bytes_with_marks(value, left_quote, right_quote)
    )
}

mod touch_format {
    pub(crate) const POSIX_LOCALE: &str = "%a %b %e %H:%M:%S %Y";
    pub(crate) const ISO_8601: &str = "%Y-%m-%d";
    // "%Y%m%d%H%M.%S" 15字符
    pub(crate) const YYYYMMDDHHMM_DOT_SS: &str = "%Y%m%d%H%M.%S";
    // "%Y-%m-%d %H:%M:%S.%SS" 12字符
    pub(crate) const YYYYMMDDHHMMSS: &str = "%Y-%m-%d %H:%M:%S.%f";
    // "%Y-%m-%d %H:%M:%S" 12字符
    pub(crate) const YYYYMMDDHHMMS: &str = "%Y-%m-%d %H:%M:%S";
    // "%Y-%m-%d %H:%M" 12字符
    // 用于tests/touch/no-rights.sh中的示例
    pub(crate) const YYYY_MM_DD_HH_MM: &str = "%Y-%m-%d %H:%M";
    // "%Y%m%d%H%M" 12字符
    pub(crate) const YYYYMMDDHHMM: &str = "%Y%m%d%H%M";
}

/// 将具有TZ偏移量的DateTime转换为FileTime
/// DateTime将转换为Unix时间戳，从中构建FileTime。
fn touch_datetime_to_filetime<T: TimeZone>(dt: &DateTime<T>) -> FileTime {
    FileTime::from_unix_time(dt.timestamp(), dt.timestamp_subsec_nanos())
}

fn touch_filetime_to_datetime(ft: &FileTime) -> Option<DateTime<Local>> {
    Some(DateTime::from_timestamp(ft.unix_seconds(), ft.nanoseconds())?.into())
}

fn touch_parse_full_naive_datetime(input: &str, format: &str) -> Option<NaiveDateTime> {
    let (datetime, remainder) = NaiveDateTime::parse_and_remainder(input, format).ok()?;
    remainder.is_empty().then_some(datetime)
}

fn touch_parse_full_naive_date(input: &str, format: &str) -> Option<NaiveDate> {
    let (date, remainder) = NaiveDate::parse_and_remainder(input, format).ok()?;
    remainder.is_empty().then_some(date)
}

#[cfg(test)]
fn touch_parse_posix_locale_datetime<T: TimeZone>(input: &str, timezone: T) -> Option<FileTime> {
    let parsed = touch_parse_full_naive_datetime(input, touch_format::POSIX_LOCALE)?;
    touch_select_local_datetime(timezone, parsed)
        .map(|datetime| touch_datetime_to_filetime(&datetime))
}

fn touch_parse_system_posix_locale_datetime(input: &str) -> Option<FileTime> {
    let parsed = touch_parse_full_naive_datetime(input, touch_format::POSIX_LOCALE)?;
    ct_parse_datetime::resolve_local_datetime_gnu_compat(parsed)
        .map(|datetime| touch_datetime_to_filetime(&datetime))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TouchTime {
    Now,
    Omit,
    Timestamp(FileTime),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TouchTimes {
    access: TouchTime,
    modification: TouchTime,
}

impl TouchTimes {
    fn now() -> Self {
        Self {
            access: TouchTime::Now,
            modification: TouchTime::Now,
        }
    }

    fn timestamps(access: FileTime, modification: FileTime) -> Self {
        Self {
            access: TouchTime::Timestamp(access),
            modification: TouchTime::Timestamp(modification),
        }
    }

    fn as_timestamps(self) -> Option<(FileTime, FileTime)> {
        match (self.access, self.modification) {
            (TouchTime::Timestamp(access), TouchTime::Timestamp(modification)) => {
                Some((access, modification))
            }
            _ => None,
        }
    }
}

fn touch_option_is_set(matches: &ArgMatches, option: &str) -> bool {
    matches.get_count(option) > 0
}

pub fn touch_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_matches = ct_app().try_get_matches_from(touch_prepare_args(args)?)?;

    if arg_matches.contains_id(touch_flags::sources::TOUCH_TIMESTAMP)
        && (arg_matches.contains_id(touch_flags::sources::TOUCH_DATE)
            || arg_matches.contains_id(touch_flags::sources::TOUCH_REFERENCE))
    {
        return Err(CtSimpleError::new(
            1,
            format!(
                "cannot specify times from more than one source\nTry '{} --help' for more information.",
                ctcore::ct_help_utility_name()
            ),
        ));
    }

    // 1. 将 files 收集为 Vec，以便我们可以移出作为时间戳的元素
    let mut files: Vec<OsString> = arg_matches
        .get_many::<OsString>(TOUCH_ARG_FILES)
        .ok_or_else(|| {
            let err_message = format!(
                "missing file operand\nTry '{} --help' for more information.",
                ctcore::ct_help_utility_name()
            );
            CtSimpleError::new(1, err_message)
        })?
        .cloned()
        .collect();

    // 2. 嗅探并提取过时的 POSIX 语法时间戳
    let mut obs_time = None;
    if !arg_matches.contains_id(touch_flags::TOUCH_SOURCES) && files.len() >= 2 {
        if let Some(s) = files[0].to_str() {
            let posix2_version = ct_posix_version().unwrap_or(MODERN as i32);
            if let Some(timestamp) = touch_obsolescent_timestamp(s, posix2_version) {
                if !posixly_correct() {
                    touch_warn_obsolescent_timestamp(s, timestamp);
                }
                obs_time = Some(timestamp);
                files.remove(0);
            }
        }
    }

    // 3. 将可能存在的 obs_time 传入
    let times = touch_determine_times(&arg_matches, obs_time)?;

    // 4. 注意这里的循环变量改为借用引用 &files
    for filename in &files {
        #[cfg(target_os = "linux")]
        if filename == "-" {
            if let Err(error) = touch_update_times(&arg_matches, Path::new(""), times, filename) {
                ct_show!(error);
            }
            continue;
        }

        let path_buf = if filename == "-" {
            touch_pathbuf_from_stdout()?
        } else {
            PathBuf::from(filename)
        };

        let path = path_buf.as_path();
        let open_result = (!touch_option_is_set(&arg_matches, touch_flags::TOUCH_NO_CREATE)
            && !touch_option_is_set(&arg_matches, touch_flags::TOUCH_NO_DEREF))
        .then(|| touch_open_for_creation(path));
        let open_error = open_result
            .as_ref()
            .and_then(|result| result.as_ref().err());

        let md_result = if touch_option_is_set(&arg_matches, touch_flags::TOUCH_NO_DEREF) {
            path.symlink_metadata()
        } else {
            path.metadata()
        };

        if let Err(e) = md_result {
            if e.kind() != std::io::ErrorKind::NotFound {
                if let Some(open_error) =
                    open_error.filter(|error| !touch_open_error_is_directory(path, error))
                {
                    ct_show!(CtSimpleError::new(
                        1,
                        touch_open_error_message(filename.as_os_str(), open_error),
                    ));
                } else {
                    let err_message = format!("setting times of {}", touch_quote_path(filename));
                    ct_show!(e.map_err_context(|| err_message));
                }
                continue;
            }

            if touch_option_is_set(&arg_matches, touch_flags::TOUCH_NO_CREATE) {
                continue;
            }

            if touch_option_is_set(&arg_matches, touch_flags::TOUCH_NO_DEREF) {
                let err_message = format!(
                    "setting times of {}: No such file or directory",
                    touch_quote_path(filename)
                );
                ct_show!(CtSimpleError::new(1, err_message));
                continue;
            }

            if let Some(Err(error)) = open_result.as_ref() {
                if touch_open_error_is_directory(path, error) {
                    if let Err(error) = touch_update_times(&arg_matches, path, times, filename) {
                        ct_show!(error);
                    }
                } else {
                    ct_show!(CtSimpleError::new(
                        1,
                        touch_open_error_message(path.as_os_str(), error),
                    ));
                }
                continue;
            }

            // 小优化：如果没有指定参考时间，我们就完成了。
            if !arg_matches.contains_id(touch_flags::TOUCH_SOURCES) && obs_time.is_none() {
                continue;
            }
        }

        if let Err(error) = touch_update_times(&arg_matches, path, times, filename) {
            if let Some(open_error) =
                open_error.filter(|error| !touch_open_error_is_directory(path, error))
            {
                ct_show!(CtSimpleError::new(
                    1,
                    touch_open_error_message(filename.as_os_str(), open_error),
                ));
            } else {
                ct_show!(error);
            }
        }
    }
    Ok(())
}

/// Open a target exactly as GNU touch does before updating its timestamps.
///
/// In particular, this must not use `File::create`, which implies `O_TRUNC`
/// and can destroy a path that appears between metadata lookup and open.
fn touch_open_for_creation(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.custom_flags(ctcore::libc::O_NONBLOCK | ctcore::libc::O_NOCTTY);
    }
    options.open(path)
}

fn touch_open_error_is_directory(path: &Path, error: &io::Error) -> bool {
    error.raw_os_error() == Some(ctcore::libc::EISDIR)
        || (error.raw_os_error() == Some(ctcore::libc::EINVAL) && path.is_dir())
}

fn touch_open_error_message(path: &OsStr, error: &io::Error) -> String {
    format!(
        "cannot touch {}: {}",
        touch_quote_path(path),
        strip_errno(error)
    )
}

fn touch_setting_times_error_message(path: &OsStr, error: &io::Error) -> String {
    format!(
        "setting times of {}: {}",
        touch_quote_path(path),
        strip_errno(error)
    )
}

fn touch_quote_path(path: &OsStr) -> String {
    gnu_quote_shell(path, true)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("touch.about");
    let usage_description = t!("touch.usage");
    let args = vec![
        Arg::new(touch_flags::TOUCH_HELP)
            .long(touch_flags::TOUCH_HELP)
            .help(t!("touch.clap.touch_help"))
            .action(ArgAction::Help),
        Arg::new(touch_flags::TOUCH_ACCESS)
            .short('a')
            .help(t!("touch.clap.touch_access"))
            .action(ArgAction::Count),
        Arg::new(touch_flags::sources::TOUCH_TIMESTAMP)
            .short('t')
            .help(t!("touch.clap.touch_timestamp"))
            .value_name("STAMP")
            .value_parser(ValueParser::os_string())
            .allow_hyphen_values(true)
            .action(ArgAction::Append),
        Arg::new(touch_flags::sources::TOUCH_DATE)
            .short('d')
            .long(touch_flags::sources::TOUCH_DATE)
            .allow_hyphen_values(true)
            .help(t!("touch.clap.touch_date"))
            .value_name("STRING")
            .value_parser(ValueParser::os_string())
            .action(ArgAction::Append),
        Arg::new(touch_flags::TOUCH_MODIFICATION)
            .short('m')
            .help(t!("touch.clap.touch_modification"))
            .action(ArgAction::Count),
        Arg::new(touch_flags::TOUCH_NO_CREATE)
            .short('c')
            .long(touch_flags::TOUCH_NO_CREATE)
            .help(t!("touch.clap.touch_no_create"))
            .action(ArgAction::Count),
        Arg::new(touch_flags::TOUCH_NO_DEREF)
            .short('h')
            .long(touch_flags::TOUCH_NO_DEREF)
            .help(
                "affect each symbolic link instead of any referenced file \
                     (only for systems that can change the timestamps of a symlink)",
            )
            .action(ArgAction::Count),
        Arg::new(touch_flags::TOUCH_FORCE)
            .short('f')
            .help("(ignored)")
            .action(ArgAction::Count),
        Arg::new(touch_flags::sources::TOUCH_REFERENCE)
            .short('r')
            .long(touch_flags::sources::TOUCH_REFERENCE)
            .help(t!("touch.clap.touch_reference"))
            .value_name("FILE")
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::AnyPath)
            .allow_hyphen_values(true)
            .action(ArgAction::Append),
        Arg::new(touch_flags::TOUCH_TIME)
            .long(touch_flags::TOUCH_TIME)
            .help(
                "change only the specified time: \"access\", \"atime\", or \
                     \"use\" are equivalent to -a; \"modify\" or \"mtime\" are \
                     equivalent to -m",
            )
            .value_name("WORD")
            .allow_hyphen_values(true)
            .action(ArgAction::Append)
            .value_parser(touch_parse_time_word),
        Arg::new(TOUCH_ARG_FILES)
            .action(ArgAction::Append)
            .num_args(1..)
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .args(args)
        .group(
            ArgGroup::new(touch_flags::TOUCH_SOURCES)
                .args([
                    touch_flags::sources::TOUCH_TIMESTAMP,
                    touch_flags::sources::TOUCH_DATE,
                    touch_flags::sources::TOUCH_REFERENCE,
                ])
                .multiple(true),
        )
        .gnu_getopt()
}

fn touch_date_source_value(value: &OsString) -> CTResult<&str> {
    value
        .to_str()
        .ok_or_else(|| CtSimpleError::new(1, touch_invalid_date_format(value.as_encoded_bytes())))
}

// 确定访问和修改时间
fn touch_determine_times(matches: &ArgMatches, obs_time: Option<FileTime>) -> CTResult<TouchTimes> {
    match (
        matches
            .get_many::<OsString>(touch_flags::sources::TOUCH_REFERENCE)
            .and_then(Iterator::last),
        matches
            .get_many::<OsString>(touch_flags::sources::TOUCH_DATE)
            .and_then(Iterator::last),
    ) {
        (Some(reference), Some(date)) => {
            let date = touch_date_source_value(date)?;
            let (a_time, m_time) = touch_stat(
                Path::new(&reference),
                !touch_option_is_set(matches, touch_flags::TOUCH_NO_DEREF),
            )?;
            let atime = touch_filetime_to_datetime(&a_time).ok_or_else(|| {
                CtSimpleError::new(1, "Could not process the reference access time")
            })?;
            let mtime = touch_filetime_to_datetime(&m_time).ok_or_else(|| {
                CtSimpleError::new(1, "Could not process the reference modification time")
            })?;
            Ok(TouchTimes::timestamps(
                touch_parse_date(atime, date)?,
                touch_parse_date(mtime, date)?,
            ))
        }
        (Some(reference), None) => {
            let (a_time, m_time) = touch_stat(
                Path::new(&reference),
                !touch_option_is_set(matches, touch_flags::TOUCH_NO_DEREF),
            )?;
            Ok(TouchTimes::timestamps(a_time, m_time))
        }
        (None, Some(date)) => {
            let date = touch_date_source_value(date)?;
            let now = Local::now();
            let timestamp = touch_parse_date(now, date)?;
            if touch_date_uses_current_time(date, now, timestamp) {
                return Ok(TouchTimes::now());
            }
            Ok(TouchTimes::timestamps(timestamp, timestamp))
        }
        (None, None) => {
            if let Some(ts) = matches
                .get_many::<OsString>(touch_flags::sources::TOUCH_TIMESTAMP)
                .and_then(Iterator::last)
            {
                let timestamp = parse_timestamp(touch_date_source_value(ts)?)?;
                return Ok(TouchTimes::timestamps(timestamp, timestamp));
            }

            if let Some(t) = obs_time {
                return Ok(TouchTimes::timestamps(t, t));
            }

            Ok(TouchTimes::now())
        }
    }
}

/// Return whether DATE evaluates to its reference timestamp unchanged.
///
/// GNU touch makes this distinction so that expressions such as `0 seconds`
/// use `UTIME_NOW`, which permits a writable non-owned file to be updated.
fn touch_date_uses_current_time(
    date: &str,
    reference_time: DateTime<Local>,
    parsed_time: FileTime,
) -> bool {
    if parsed_time != touch_datetime_to_filetime(&reference_time) {
        return false;
    }

    let alternate_seconds = reference_time.timestamp() ^ 1;
    let Some(alternate_time) = Local
        .timestamp_opt(alternate_seconds, reference_time.timestamp_subsec_nanos())
        .single()
    else {
        return false;
    };

    touch_parse_date(alternate_time, date)
        .is_ok_and(|parsed_time| parsed_time == touch_datetime_to_filetime(&alternate_time))
}

#[cfg(not(target_os = "linux"))]
fn touch_time_to_filetime(
    time: TouchTime,
    existing: Option<(FileTime, FileTime)>,
    is_access: bool,
    now: FileTime,
) -> FileTime {
    match time {
        TouchTime::Now => now,
        TouchTime::Omit => {
            let (access, modification) = existing.expect("omitted time requires existing metadata");
            if is_access { access } else { modification }
        }
        TouchTime::Timestamp(time) => time,
    }
}

#[cfg(not(target_os = "linux"))]
fn touch_times_to_filetimes(
    path: &Path,
    is_follow: bool,
    times: TouchTimes,
) -> CTResult<(FileTime, FileTime)> {
    let existing = if times.access == TouchTime::Omit || times.modification == TouchTime::Omit {
        Some(touch_stat(path, is_follow)?)
    } else {
        None
    };
    let now = touch_datetime_to_filetime(&Local::now());
    Ok((
        touch_time_to_filetime(times.access, existing, true, now),
        touch_time_to_filetime(times.modification, existing, false, now),
    ))
}

#[cfg(target_os = "linux")]
fn touch_timespec(time: TouchTime) -> ctcore::libc::timespec {
    match time {
        TouchTime::Now => ctcore::libc::timespec {
            tv_sec: 0,
            tv_nsec: ctcore::libc::UTIME_NOW as _,
        },
        TouchTime::Omit => ctcore::libc::timespec {
            tv_sec: 0,
            tv_nsec: ctcore::libc::UTIME_OMIT as _,
        },
        TouchTime::Timestamp(time) => ctcore::libc::timespec {
            tv_sec: time.unix_seconds() as ctcore::libc::time_t,
            tv_nsec: time.nanoseconds() as _,
        },
    }
}

#[cfg(target_os = "linux")]
fn touch_set_times_with_utimensat(
    path: &Path,
    times: TouchTimes,
    no_follow: bool,
) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains interior NUL"))?;
    let times = [
        touch_timespec(times.access),
        touch_timespec(times.modification),
    ];
    let flags = if no_follow {
        ctcore::libc::AT_SYMLINK_NOFOLLOW
    } else {
        0
    };

    let result = unsafe {
        ctcore::libc::utimensat(ctcore::libc::AT_FDCWD, path.as_ptr(), times.as_ptr(), flags)
    };

    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn touch_set_times_on_fd(fd: ctcore::libc::c_int, times: TouchTimes) -> io::Result<()> {
    let timespecs = [
        touch_timespec(times.access),
        touch_timespec(times.modification),
    ];
    let timespecs = if times.access == TouchTime::Now && times.modification == TouchTime::Now {
        std::ptr::null()
    } else {
        timespecs.as_ptr()
    };

    let result = unsafe { ctcore::libc::futimens(fd, timespecs) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn touch_set_times(
    arg_matches: &ArgMatches,
    path: &Path,
    times: TouchTimes,
    file_name: &OsString,
) -> CTResult<()> {
    #[cfg(target_os = "linux")]
    if file_name == "-" {
        let result = if ctcore::ct_stdout_was_closed() {
            Err(io::Error::from_raw_os_error(ctcore::libc::EBADF))
        } else {
            touch_set_times_on_fd(ctcore::libc::STDOUT_FILENO, times)
        };
        if touch_option_is_set(arg_matches, touch_flags::TOUCH_NO_CREATE)
            && result
                .as_ref()
                .is_err_and(|error| error.raw_os_error() == Some(ctcore::libc::EBADF))
        {
            return Ok(());
        }
        return result.map_err(|error| {
            CtSimpleError::new(
                1,
                touch_setting_times_error_message(file_name.as_os_str(), &error),
            )
        });
    }

    let no_follow =
        file_name != "-" && touch_option_is_set(arg_matches, touch_flags::TOUCH_NO_DEREF);
    let result = if let Some((a_time, m_time)) = times.as_timestamps() {
        if file_name == "-" {
            filetime::set_file_times(path, a_time, m_time)
        } else if no_follow {
            set_symlink_file_times(path, a_time, m_time)
        } else {
            set_file_times(path, a_time, m_time)
        }
    } else {
        #[cfg(target_os = "linux")]
        {
            touch_set_times_with_utimensat(path, times, no_follow)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let (a_time, m_time) = touch_times_to_filetimes(path, !no_follow, times)?;
            if file_name == "-" {
                filetime::set_file_times(path, a_time, m_time)
            } else if no_follow {
                set_symlink_file_times(path, a_time, m_time)
            } else {
                set_file_times(path, a_time, m_time)
            }
        }
    };

    result.map_err(|error| {
        CtSimpleError::new(
            1,
            touch_setting_times_error_message(file_name.as_os_str(), &error),
        )
    })
}

// 根据用户指定的选项更新文件访问和修改时间
fn touch_update_times(
    arg_matches: &ArgMatches,
    path: &Path,
    mut times: TouchTimes,
    file_name: &OsString,
) -> CTResult<()> {
    // 如果仅更改atime或mtime，则获取另一个的现有值。
    // 请注意，"-a"和"-m"可以一起传递；这不是xor。
    if touch_option_is_set(arg_matches, touch_flags::TOUCH_ACCESS)
        || touch_option_is_set(arg_matches, touch_flags::TOUCH_MODIFICATION)
        || arg_matches.contains_id(touch_flags::TOUCH_TIME)
    {
        let time_words = arg_matches
            .get_many::<String>(touch_flags::TOUCH_TIME)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect::<Vec<_>>();

        if !(touch_option_is_set(arg_matches, touch_flags::TOUCH_ACCESS)
            || time_words.contains(&"access"))
        {
            times.access = TouchTime::Omit;
        }

        if !(touch_option_is_set(arg_matches, touch_flags::TOUCH_MODIFICATION)
            || time_words.contains(&"modify"))
        {
            times.modification = TouchTime::Omit;
        }
    }

    touch_set_times(arg_matches, path, times, file_name)
}

fn touch_parse_date(ref_time: DateTime<Local>, s: &str) -> CTResult<FileTime> {
    // 首先检查Unix时间戳格式 "@%s"，这应该被视为绝对时间
    // "@%s" 是 "自纪元1970-01-01 00:00:00 +0000 (UTC)以来的秒数。 (TZ) (由mktime(tm)计算。)"
    if s.bytes().next() == Some(b'@') {
        if let Ok(ts) = &s[1..].parse::<i64>() {
            return Ok(FileTime::from_unix_time(*ts, 0));
        }
    }

    // Chrono represents a `:60` leap second with a one-billion-nanosecond
    // value. GNU parse-datetime rejects it before any target is created.
    if ct_parse_datetime::contains_leap_second(s) {
        return Err(CtSimpleError::new(
            1,
            touch_invalid_date_format(s.as_bytes()),
        ));
    }

    // "当前语言环境的首选日期和时间表示。"
    // "(在POSIX语言环境中这相当于%a %b %e %H:%M:%S %Y。)"
    // time 0.1.43将其解析为'a b e T Y'
    // 这相当于POSIX语言环境：%a %b %e %H:%M:%S %Y
    // 周二12月3日...
    // ("%c", POSIX_LOCALE_FORMAT),
    //
    if let Some(parsed) = touch_parse_system_posix_locale_datetime(s) {
        return Ok(parsed);
    }

    // 使用GNU coreutils兼容的日期解析器
    // 支持"next friday"、"last monday"等自然语言日期表达式
    match ct_parse_datetime::parse_datetime_to_filetime(s, ref_time) {
        Ok(ft) => return Ok(ft),
        Err(_) => {
            // 如果新解析器失败，尝试其他格式
        }
    }

    // 还支持在GNU测试中找到的其他格式，如
    // 在tests/misc/stat-nanoseconds.sh中
    // 或tests/touch/no-rights.sh中
    for fmt in [
        touch_format::YYYYMMDDHHMMS,
        touch_format::YYYYMMDDHHMMSS,
        touch_format::YYYY_MM_DD_HH_MM,
    ] {
        if let Some(parsed) = touch_parse_full_naive_datetime(s, fmt) {
            let parsed = ct_parse_datetime::resolve_local_datetime_gnu_compat(parsed)
                .ok_or_else(|| CtSimpleError::new(1, touch_invalid_date_format(s.as_bytes())))?;
            return Ok(touch_datetime_to_filetime(&parsed));
        }
    }

    // "相当于%Y-%m-%d (ISO 8601日期格式)。 (C99)"
    // ("%F", ISO_8601_FORMAT),
    if let Some(parsed_date) = touch_parse_full_naive_date(s, touch_format::ISO_8601) {
        let parsed = ct_parse_datetime::resolve_local_datetime_gnu_compat(
            parsed_date.and_time(NaiveTime::MIN),
        )
        .ok_or_else(|| CtSimpleError::new(1, touch_invalid_date_format(s.as_bytes())))?;
        return Ok(touch_datetime_to_filetime(&parsed));
    }

    Err(CtSimpleError::new(
        1,
        touch_invalid_date_format(s.as_bytes()),
    ))
}

// 获取提供路径的元数据
// 如果`follow`为`true`，函数将尝试跟随符号链接
// 如果`follow`为`false`，函数将返回符号链接本身的元数据
fn touch_stat(path: &Path, is_follow: bool) -> CTResult<(FileTime, FileTime)> {
    let md = match is_follow {
        true => fs::metadata(path),
        false => fs::symlink_metadata(path),
    }
    .map_err_context(|| {
        format!(
            "failed to get attributes of {}",
            touch_quote_path(path.as_os_str())
        )
    })?;

    Ok((
        FileTime::from_last_access_time(&md),
        FileTime::from_last_modification_time(&md),
    ))
}

fn touch_select_local_datetime<T: TimeZone>(
    timezone: T,
    local: NaiveDateTime,
) -> Option<DateTime<T>> {
    touch_choose_local_datetime(timezone.from_local_datetime(&local))
}

fn touch_choose_local_datetime<T: TimeZone>(
    local_result: LocalResult<DateTime<T>>,
) -> Option<DateTime<T>> {
    match local_result {
        LocalResult::Single(datetime) => Some(datetime),
        LocalResult::Ambiguous(first, second) => Some(if first.timestamp() <= second.timestamp() {
            first
        } else {
            second
        }),
        LocalResult::None => None,
    }
}

fn parse_timestamp(s: &str) -> CTResult<FileTime> {
    use touch_format::*;

    let current_year = || Local::now().year();
    let two_digit_year_prefix = || {
        let year = s.chars().take(2).collect::<String>();
        match year.parse::<u8>() {
            Ok(69..=99) => "19",
            _ => "20",
        }
    };

    let (format, ts) = match s.chars().count() {
        15 => (YYYYMMDDHHMM_DOT_SS, s.to_owned()),
        12 => (YYYYMMDDHHMM, s.to_owned()),
        13 => (
            YYYYMMDDHHMM_DOT_SS,
            format!("{}{s}", two_digit_year_prefix()),
        ),
        10 => (YYYYMMDDHHMM, format!("{}{s}", two_digit_year_prefix())),
        11 => (YYYYMMDDHHMM_DOT_SS, format!("{}{}", current_year(), s)),
        8 => (YYYYMMDDHHMM, format!("{}{}", current_year(), s)),
        _ => {
            return Err(CtSimpleError::new(
                1,
                touch_invalid_date_format(s.as_bytes()),
            ));
        }
    };

    let local = NaiveDateTime::parse_from_str(&ts, format)
        .map_err(|_| CtSimpleError::new(1, touch_invalid_date_format(s.as_bytes())))?;
    let mut local = match touch_select_local_datetime(Local, local) {
        Some(datetime) => datetime,
        None => {
            return Err(CtSimpleError::new(
                1,
                touch_invalid_date_format(s.as_bytes()),
            ));
        }
    };

    // Chrono将秒数限制在59，但60是有效的。它可能是一个闰秒
    // 或者跳到下一分钟。但这并不重要，因为我们
    // 只关心时间戳。
    // 在gnu/tests/touch/60-seconds中测试
    if local.second() == 59 && ts.ends_with(".60") {
        local += Duration::try_seconds(1).unwrap();
    }

    // 由于夏令时切换，当地时间可以从凌晨1:59跳到
    // 凌晨3:00，在这种情况下，凌晨2:00到凌晨2:59之间的任何时间都是无效的。
    // 如果我们在这个跳跃中，chrono会从跳跃前获取偏移量。如果我们向前跳一小时，
    // 我们会得到新的修正偏移量。向后跳跃将现在正确考虑跳跃。
    let local2 = local + Duration::try_hours(1).unwrap() - Duration::try_hours(1).unwrap();
    if local.hour() != local2.hour() {
        return Err(CtSimpleError::new(
            1,
            touch_invalid_date_format(s.as_bytes()),
        ));
    }

    Ok(touch_datetime_to_filetime(&local))
}

fn parse_obsolescent_timestamp(s: &str) -> CTResult<FileTime> {
    parse_obsolescent_timestamp_with_timezone(s, Local::now().year(), Local)
}

fn parse_obsolescent_timestamp_with_timezone<T: TimeZone>(
    s: &str,
    current_year: i32,
    timezone: T,
) -> CTResult<FileTime> {
    let format = touch_format::YYYYMMDDHHMM;

    let ts = if s.len() == 8 {
        format!("{current_year}{s}")
    } else if s.len() == 10 {
        // 由于外层已经保证了 YY 必然在 69-99 之间，直接 +1900 即可
        let mmddhhmm = &s[0..8];
        let yy = &s[8..10];
        let year = yy.parse::<i32>().unwrap_or(0) + 1900;
        format!("{year:04}{mmddhhmm}")
    } else {
        return Err(CtSimpleError::new(
            1,
            format!("invalid date format {}", s.quote()),
        ));
    };

    let local = NaiveDateTime::parse_from_str(&ts, format)
        .map_err(|_| CtSimpleError::new(1, format!("invalid date ts format {}", ts.quote())))?;

    let local = match touch_select_local_datetime(timezone, local) {
        Some(datetime) => datetime,
        None => {
            return Err(CtSimpleError::new(
                1,
                format!("invalid date ts format {}", ts.quote()),
            ));
        }
    };

    // 夏令时跳变校验
    let local2 = local.clone() + Duration::try_hours(1).unwrap() - Duration::try_hours(1).unwrap();
    if local.hour() != local2.hour() {
        return Err(CtSimpleError::new(
            1,
            format!("invalid date format {}", s.quote()),
        ));
    }

    Ok(touch_datetime_to_filetime(&local))
}

fn touch_obsolescent_timestamp(s: &str, posix2_version: i32) -> Option<FileTime> {
    if posix2_version >= TRADITIONAL as i32 {
        return None;
    }

    let bytes = s.as_bytes();
    let is_timestamp = match bytes.len() {
        8 => bytes.iter().all(u8::is_ascii_digit),
        10 if bytes.iter().all(u8::is_ascii_digit) => {
            let year = std::str::from_utf8(&bytes[8..10])
                .ok()?
                .parse::<u8>()
                .ok()?;
            (69..=99).contains(&year)
        }
        _ => false,
    };

    is_timestamp
        .then(|| parse_obsolescent_timestamp(s).ok())
        .flatten()
}

fn touch_warn_obsolescent_timestamp(input: &str, timestamp: FileTime) {
    if let Some(timestamp) = touch_filetime_to_datetime(&timestamp) {
        ctcore::ct_show_warning!(
            "'touch {input}' is obsolete; use 'touch -t {}'",
            timestamp.format("%Y%m%d%H%M.%S")
        );
    }
}

// TODO: 这可能是放入ct_fsext的好候选项
/// 返回指向标准输出的PathBuf。
///
/// 在Windows上，使用GetFinalPathNameByHandleW尝试从stdout句柄获取路径。
fn touch_pathbuf_from_stdout() -> CTResult<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Ok(PathBuf::from("/dev/stdout"))
    }
    #[cfg(windows)]
    {
        use std::os::windows::prelude::AsRawHandle;
        use windows_sys::Win32::Foundation::{
            ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_PATH_NOT_FOUND, GetLastError,
            HANDLE, MAX_PATH,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_NAME_OPENED, GetFinalPathNameByHandleW,
        };

        let handle = std::io::stdout().lock().as_raw_handle() as HANDLE;
        let mut file_path_buffer: [u16; MAX_PATH as usize] = [0; MAX_PATH as usize];

        // https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlea#examples
        // SAFETY: 我们将句柄转化为能够将*mut c_void转换为HANDLE（i32），以便rustc允许我们调用GetFinalPathNameByHandleW。
        // GetFinalPathNameByHandleW的参考示例代码表明，只要缓冲区大小正确，
        // 可以安全地让lpszfilepath未初始化。我们在编译时知道缓冲区大小（MAX_PATH）。
        // MAX_PATH是一个小数字（260），因此我们可以将其转换为u32。
        let ret = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                file_path_buffer.as_mut_ptr(),
                file_path_buffer.len() as u32,
                FILE_NAME_OPENED,
            )
        };

        let buffer_size = match ret {
            ERROR_PATH_NOT_FOUND | ERROR_NOT_ENOUGH_MEMORY | ERROR_INVALID_PARAMETER => {
                return Err(CtSimpleError::new(
                    1,
                    format!("GetFinalPathNameByHandleW failed with code {ret}"),
                ));
            }
            0 => {
                return Err(CtSimpleError::new(
                    1,
                    format!(
                        "GetFinalPathNameByHandleW failed with code {}",
                        // SAFETY: GetLastError是线程安全的，没有记录的内存不安全。
                        unsafe { GetLastError() }
                    ),
                ));
            }
            e => e as usize,
        };

        // 不包括空终止符
        Ok(String::from_utf16(&file_path_buffer[0..buffer_size])
            .map_err(|e| CtSimpleError::new(1, e.to_string()))?
            .into())
    }
}

#[derive(Default)]
pub struct Touch;
impl Tool for Touch {
    fn name(&self) -> &'static str {
        "touch"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 直接调用原有的 touch_main 函数
        touch_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};

    #[cfg(test)]
    mod determine_times_tests {
        use super::*;
        use chrono::Local;
        use clap::ArgMatches;
        use filetime::{FileTime, set_file_times};
        use tempfile::tempdir;

        fn build_matches(args: &[&str]) -> ArgMatches {
            let mut argv = Vec::with_capacity(args.len() + 1);
            argv.push(ctcore::ct_util_name());
            argv.extend_from_slice(args);
            ct_app().try_get_matches_from(argv).expect("参数解析失败")
        }

        fn timestamp_pair(times: TouchTimes) -> (FileTime, FileTime) {
            times.as_timestamps().expect("expected explicit timestamps")
        }

        #[test]
        fn determine_times_defaults_to_now() {
            let matches = build_matches(&["dummy"]);
            let times = touch_determine_times(&matches, None).unwrap();
            assert_eq!(times.access, TouchTime::Now);
            assert_eq!(times.modification, TouchTime::Now);
        }

        #[test]
        fn determine_times_date_now_uses_current_time_semantics() {
            let matches = build_matches(&["-d", "now", "dummy"]);
            let times = touch_determine_times(&matches, None).unwrap();
            assert_eq!(times.access, TouchTime::Now);
            assert_eq!(times.modification, TouchTime::Now);
        }

        #[test]
        fn determine_times_zero_relative_date_uses_current_time_semantics() {
            let matches = build_matches(&["-d", "0 seconds", "dummy"]);
            let times = touch_determine_times(&matches, None).unwrap();
            assert_eq!(times.access, TouchTime::Now);
            assert_eq!(times.modification, TouchTime::Now);
        }

        #[test]
        fn determine_times_with_timestamp_argument() {
            let matches = build_matches(&["-t", "202406150830", "dummy"]);
            let (atime, mtime) = timestamp_pair(touch_determine_times(&matches, None).unwrap());
            let expected = Local.with_ymd_and_hms(2024, 6, 15, 8, 30, 0).unwrap();
            assert_eq!(atime.unix_seconds(), expected.timestamp());
            assert_eq!(mtime, atime);
        }

        #[test]
        fn determine_times_with_date_argument() {
            let matches = build_matches(&["-d", "2024-06-15 08:30:00", "dummy"]);
            let (atime, mtime) = timestamp_pair(touch_determine_times(&matches, None).unwrap());
            let expected = Local.with_ymd_and_hms(2024, 6, 15, 8, 30, 0).unwrap();
            assert_eq!(atime.unix_seconds(), expected.timestamp());
            assert_eq!(mtime, atime);
        }

        #[test]
        fn determine_times_with_reference_file() {
            let tmp = tempdir().unwrap();
            let file_path = tmp.path().join("reference.txt");
            std::fs::write(&file_path, b"test").unwrap();
            let custom_time = FileTime::from_unix_time(1_600_000_000, 0);
            set_file_times(&file_path, custom_time, custom_time).unwrap();

            let matches = build_matches(&["-r", file_path.to_str().unwrap(), "dummy"]);
            let (atime, mtime) = timestamp_pair(touch_determine_times(&matches, None).unwrap());
            assert_eq!(atime.unix_seconds(), custom_time.unix_seconds());
            assert_eq!(mtime.unix_seconds(), custom_time.unix_seconds());
        }

        #[test]
        fn determine_times_uses_last_repeated_source_value() {
            let tmp = tempdir().unwrap();
            let first_reference = tmp.path().join("first-reference");
            let last_reference = tmp.path().join("last-reference");
            std::fs::write(&first_reference, b"first").unwrap();
            std::fs::write(&last_reference, b"last").unwrap();
            let first_time = FileTime::from_unix_time(946_684_800, 0);
            let last_time = FileTime::from_unix_time(978_307_200, 0);
            set_file_times(&first_reference, first_time, first_time).unwrap();
            set_file_times(&last_reference, last_time, last_time).unwrap();

            let matches = build_matches(&[
                "-r",
                first_reference.to_str().unwrap(),
                "-r",
                last_reference.to_str().unwrap(),
                "dummy",
            ]);
            let (atime, mtime) = timestamp_pair(touch_determine_times(&matches, None).unwrap());
            assert_eq!(atime, last_time);
            assert_eq!(mtime, last_time);

            let matches = build_matches(&["-d", "2000-01-01", "-d", "2001-01-01", "dummy"]);
            let (atime, mtime) = timestamp_pair(touch_determine_times(&matches, None).unwrap());
            let expected = Local
                .with_ymd_and_hms(2001, 1, 1, 0, 0, 0)
                .unwrap()
                .timestamp();
            assert_eq!(atime.unix_seconds(), expected);
            assert_eq!(mtime.unix_seconds(), expected);

            let matches = build_matches(&["-t", "200001010000", "-t", "200101010000", "dummy"]);
            let (atime, mtime) = timestamp_pair(touch_determine_times(&matches, None).unwrap());
            assert_eq!(atime.unix_seconds(), expected);
            assert_eq!(mtime.unix_seconds(), expected);
        }

        #[test]
        fn determine_times_with_reference_and_date_override() {
            let tmp = tempdir().unwrap();
            let file_path = tmp.path().join("reference.txt");
            std::fs::write(&file_path, b"test").unwrap();
            let custom_time = FileTime::from_unix_time(1_600_000_000, 0);
            set_file_times(&file_path, custom_time, custom_time).unwrap();

            let matches = build_matches(&[
                "-r",
                file_path.to_str().unwrap(),
                "-d",
                "2024-06-15 08:30:00",
                "dummy",
            ]);
            let (atime, mtime) = timestamp_pair(touch_determine_times(&matches, None).unwrap());
            let expected = Local
                .with_ymd_and_hms(2024, 6, 15, 8, 30, 0)
                .unwrap()
                .timestamp();
            assert_eq!(atime.unix_seconds(), expected);
            assert_eq!(mtime.unix_seconds(), expected);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn touch_set_times_on_invalid_file_descriptor_reports_ebadf() {
        let error = touch_set_times_on_fd(-1, TouchTimes::now()).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(ctcore::libc::EBADF));
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Touch;

        // Test name method
        assert_eq!(tool.name(), "touch");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("touch"));

        // Test execute method with help flag (should work)
        let args = vec![OsString::from("touch"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    #[cfg(test)]
    mod parse_timestamp_tests {
        use chrono::{Datelike, FixedOffset, Local, LocalResult, NaiveDate, TimeZone};
        use chrono_tz::America::New_York;

        use super::*;

        #[test]
        fn test_parse_timestamp_valid() {
            // 测试格式为 %Y%m%d%H%M.%S
            let timestamp_str = "202406150830.45";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(2024, 6, 15, 8, 30, 45)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试格式为 %Y%m%d%H%M
            let timestamp_str = "202406150830";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(2024, 6, 15, 8, 30, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试格式为 %Y%m%d%H%M.%S 并带有前缀 "20"
            let timestamp_str = "2406150830.45";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(2024, 6, 15, 8, 30, 45)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试格式为 %Y%m%d%H%M 并带有前缀 "20"
            let timestamp_str = "2406150830";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(2024, 6, 15, 8, 30, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            // 两位年份按 POSIX 规则映射到世纪：69..99 属于 19xx，00..68 属于 20xx。
            let timestamp_str = "9912312359";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(1999, 12, 31, 23, 59, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);

            let timestamp_str = "6812312359";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(2068, 12, 31, 23, 59, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);

            // 测试格式为 %Y%m%d%H%M.%S 并带有当前年份
            let current_year = Local::now().year();
            let timestamp_str = "06150830.45";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(current_year, 6, 15, 8, 30, 45)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试格式为 %Y%m%d%H%M 并带有当前年份
            let timestamp_str = "06150830";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(current_year, 6, 15, 8, 30, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试带有闰秒的时间戳
            let timestamp_str = "202406150830.60";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(2024, 6, 15, 8, 31, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);
        }

        #[test]
        fn test_touch_select_local_datetime_uses_earlier_dst_fold() {
            let local = NaiveDate::from_ymd_opt(2024, 11, 3)
                .unwrap()
                .and_hms_opt(1, 30, 0)
                .unwrap();

            let selected = touch_select_local_datetime(New_York, local)
                .expect("DST fold must select GNU's earlier local-time instance");

            assert_eq!(selected.timestamp(), 1_730_611_800);
        }

        #[test]
        fn test_touch_choose_local_datetime_prefers_earlier_fold_regardless_of_order() {
            let later = FixedOffset::west_opt(5 * 60 * 60)
                .unwrap()
                .with_ymd_and_hms(2024, 11, 3, 1, 30, 0)
                .unwrap();
            let earlier = FixedOffset::west_opt(4 * 60 * 60)
                .unwrap()
                .with_ymd_and_hms(2024, 11, 3, 1, 30, 0)
                .unwrap();

            let selected = touch_choose_local_datetime(LocalResult::Ambiguous(later, earlier))
                .expect("DST fold must select an instant");

            assert_eq!(selected.timestamp(), earlier.timestamp());
        }

        #[test]
        fn test_touch_select_local_datetime_rejects_dst_gap() {
            let local = NaiveDate::from_ymd_opt(2024, 3, 10)
                .unwrap()
                .and_hms_opt(2, 30, 0)
                .unwrap();

            assert!(touch_select_local_datetime(New_York, local).is_none());
        }

        #[test]
        fn test_parse_timestamp_invalid() {
            // 测试无效格式的时间戳
            let timestamp_str = "invalid timestamp";
            let result = parse_timestamp(timestamp_str);
            assert!(result.is_err());

            // 测试无效的时间戳长度
            let timestamp_str = "20240615083"; // 少一位
            let result = parse_timestamp(timestamp_str);
            assert!(result.is_err());

            let timestamp_str = "240615083"; // 少一位
            let result = parse_timestamp(timestamp_str);
            assert!(result.is_err());

            let timestamp_str = "0615083"; // 少一位
            let result = parse_timestamp(timestamp_str);
            assert!(result.is_err());

            let timestamp_str = "20240615083012345"; // 多几位
            let result = parse_timestamp(timestamp_str);
            assert!(result.is_err());

            // 测试无效的时间部分
            let timestamp_str = "202406150860"; // 无效的时间
            let result = parse_timestamp(timestamp_str);
            assert_eq!(
                result.unwrap_err().to_string(),
                touch_invalid_date_format(timestamp_str.as_bytes())
            );

            // 测试无效的日期部分
            let timestamp_str = "202413150830"; // 无效的月份
            let result = parse_timestamp(timestamp_str);
            assert!(result.is_err());

            let timestamp_str = "202406320830"; // 无效的日期
            let result = parse_timestamp(timestamp_str);
            assert!(result.is_err());
        }

        #[test]
        fn test_parse_timestamp_boundary() {
            // 测试Unix Epoch (1970-01-01 00:00:00)
            let timestamp_str = "197001010000";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            // 使用时间戳对比，因为FileTime的unix_seconds在不同时区会有差异
            let expected_dt = Local.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap();
            assert_eq!(filetime.unix_seconds(), expected_dt.timestamp());
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试未来的一个日期
            let timestamp_str = "210001010000";
            let filetime = parse_timestamp(timestamp_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(2100, 1, 1, 0, 0, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试带有纳秒部分的时间戳
            let timestamp_str = "202406150830.123456789";
            let filetime = parse_timestamp(timestamp_str);
            assert!(filetime.is_err());
            assert_eq!(
                filetime.unwrap_err().to_string(),
                touch_invalid_date_format(timestamp_str.as_bytes())
            );

            // 测试带有无效的纳秒部分的时间戳
            let timestamp_str = "202406150830.1234567890"; // 超过纳秒位数
            let result = parse_timestamp(timestamp_str);
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod obsolescent_timestamp_tests {
        use chrono_tz::America::New_York;

        use super::*;

        #[test]
        fn obsolete_timestamp_is_only_enabled_for_pre_200112_posix() {
            assert!(
                touch_obsolescent_timestamp("01010000", ctcore::ct_posix::MODERN as i32).is_none()
            );

            let timestamp =
                touch_obsolescent_timestamp("0101000099", ctcore::ct_posix::OBSOLETE as i32)
                    .unwrap();
            assert_eq!(timestamp.unix_seconds(), 915_148_800);

            assert!(
                touch_obsolescent_timestamp("02310000", ctcore::ct_posix::OBSOLETE as i32)
                    .is_none()
            );
        }

        #[test]
        fn obsolescent_timestamp_uses_earlier_dst_fold() {
            let timestamp = parse_obsolescent_timestamp_with_timezone("1031013099", 2026, New_York)
                .expect("GNU legacy timestamp must accept the DST fold");

            assert_eq!(timestamp.unix_seconds(), 941_347_800);
        }
    }

    #[cfg(test)]
    mod parse_date_tests {
        use chrono::{Local, TimeZone, Utc};

        use super::*;

        #[test]
        fn parse_date_rejects_new_york_dst_gap_in_legacy_datetime_fallback() {
            const CHILD_ENV: &str = "CT_TOUCH_PARSE_DATE_NEW_YORK_DST_CHILD";

            if std::env::var_os(CHILD_ENV).is_some() {
                let reference = Local.timestamp_opt(1_720_000_000, 0).unwrap();
                assert_eq!(reference.offset().local_minus_utc(), -4 * 60 * 60);
                assert!(
                    touch_parse_date(reference, "2024-03-10 02:00:00").is_err(),
                    "GNU rejects a local wall time skipped by the DST transition"
                );
                return;
            }

            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("parse_date_rejects_new_york_dst_gap_in_legacy_datetime_fallback")
                .env(CHILD_ENV, "1")
                .env("TZ", "America/New_York")
                .output()
                .expect("run isolated touch DST parser test");

            assert!(
                output.status.success(),
                "isolated touch DST parser test failed:\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        #[test]
        fn parse_date_rejects_out_of_range_numeric_timezone() {
            let reference = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            for input in [
                "2024-02-29 12:00 +24:01",
                "2024-02-29 12:00 +2401",
                "2024-02-29 12:00 +25:00",
            ] {
                assert!(
                    touch_parse_date(reference, input).is_err(),
                    "input {input} must be rejected"
                );
            }
        }

        #[test]
        fn parse_date_expands_two_digit_year_in_month_name_date() {
            let reference = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            let parsed = touch_parse_date(reference, "24 Sep 72 UTC").unwrap();
            let expected = Utc.with_ymd_and_hms(1972, 9, 24, 0, 0, 0).unwrap();

            assert_eq!(parsed.unix_seconds(), expected.timestamp());
            assert_eq!(parsed.nanoseconds(), 0);
        }

        #[test]
        fn parse_date_preserves_single_digit_year_in_month_name_date() {
            let reference = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            let parsed = touch_parse_date(reference, "024 Sep 7 GMT+3").unwrap();
            let expected = Utc.with_ymd_and_hms(7, 9, 23, 21, 0, 0).unwrap();

            assert_eq!(parsed.unix_seconds(), expected.timestamp());
            assert_eq!(parsed.nanoseconds(), 0);
        }

        #[test]
        fn parse_date_treats_leading_zero_month_day_as_reference_year_date() {
            let reference = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            let parsed = touch_parse_date(reference, "Sep 024 UTC").unwrap();
            let expected = Utc.with_ymd_and_hms(2025, 9, 24, 0, 0, 0).unwrap();

            assert_eq!(parsed.unix_seconds(), expected.timestamp());
            assert_eq!(parsed.nanoseconds(), 0);
        }

        #[test]
        fn parse_date_treats_short_number_after_month_day_as_time() {
            let reference = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            let parsed = touch_parse_date(reference, "Sep 24 7 UTC").unwrap();
            let expected = Utc.with_ymd_and_hms(2025, 9, 24, 7, 0, 0).unwrap();

            assert_eq!(parsed.unix_seconds(), expected.timestamp());
            assert_eq!(parsed.nanoseconds(), 0);
            assert!(touch_parse_date(reference, "Sep 24 24 UTC").is_err());
        }

        #[test]
        fn test_parse_date_valid() {
            // 测试POSIX_LOCALE格式的日期
            let ref_time = Local.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            let date_str = "Wed Jun 15 08:30:00 2022";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            let expected_time = Utc.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            assert_eq!(filetime.unix_seconds(), expected_time.timestamp());

            // 测试ISO 8601格式的日期
            let date_str = "2022-06-15";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            // 使用期望的日期时间对象生成时间戳，而不是硬编码值
            let expected_time = Local.with_ymd_and_hms(2022, 6, 15, 0, 0, 0).unwrap();
            assert_eq!(filetime.unix_seconds(), expected_time.timestamp());
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试其他有效格式的日期
            let date_str = "2022-06-15 08:30:00";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            // 使用本地时间计算期望的时间戳（GNU解析器将此格式视为本地时间）
            let expected_time = Local.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            assert_eq!(filetime.unix_seconds(), expected_time.timestamp());
            assert_eq!(filetime.nanoseconds(), 0);

            let date_str = "2022-06-15 08:30:00.123456";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            // GNU解析器可能不支持微秒，回退到原解析器，使用本地时间
            let expected_time = Local.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            assert_eq!(filetime.unix_seconds(), expected_time.timestamp());
            // 微秒转纳秒：123456 微秒 = 123456000 纳秒
            assert_eq!(filetime.nanoseconds(), 123456000);

            let date_str = "2022-06-15 08:30";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            // GNU解析器将此格式视为本地时间
            let expected_time = Local.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            assert_eq!(filetime.unix_seconds(), expected_time.timestamp());
            assert_eq!(filetime.nanoseconds(), 0);

            let date_str = "202206150830";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            // 使用预期的本地时间计算时间戳
            let expected_time = Local.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            assert_eq!(filetime.unix_seconds(), expected_time.timestamp());
            assert_eq!(filetime.nanoseconds(), 0);

            let date_str = "@1623742200";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            assert_eq!(filetime.unix_seconds(), 1623742200);
            assert_eq!(filetime.nanoseconds(), 0);
        }

        #[test]
        fn test_parse_date_double_dash_uses_reference_date_midnight() {
            let reference = Local.with_ymd_and_hms(2024, 2, 29, 12, 34, 56).unwrap();

            let parsed = touch_parse_date(reference, "--").unwrap();
            let expected = Local.with_ymd_and_hms(2024, 2, 29, 0, 0, 0).unwrap();

            assert_eq!(parsed.unix_seconds(), expected.timestamp());
            assert_eq!(parsed.nanoseconds(), 0);
        }

        #[test]
        fn test_parse_date_invalid() {
            let ref_time = Local.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();

            // 测试无效格式的日期
            let date_str = "invalid date string";
            let result = touch_parse_date(ref_time, date_str);
            assert_eq!(
                result.unwrap_err().to_string(),
                touch_invalid_date_format(date_str.as_bytes())
            );

            let date_str = "2022-13-15"; // 无效的月份
            let result = touch_parse_date(ref_time, date_str);
            assert!(result.is_err());

            let date_str = "2022-06-32"; // 无效的日期
            let result = touch_parse_date(ref_time, date_str);
            assert!(result.is_err());

            let date_str = "202206150860"; // 无效的时间
            let result = touch_parse_date(ref_time, date_str);
            assert!(result.is_err());

            let date_str = "@invalidtimestamp"; // 无效的Unix时间戳
            let result = touch_parse_date(ref_time, date_str);
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod stat_tests {
        use std::fs::{File, create_dir};
        use std::io::Write;
        use std::os::unix::fs::symlink;

        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_stat_regular_file() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");

            // 创建一个文件并写入一些数据
            {
                let mut file = File::create(&file_path).unwrap();
                writeln!(file, "Hello, world!").unwrap();
            }

            // 获取文件的元数据
            let (atime, mtime) = touch_stat(&file_path, true).unwrap();

            // 检查访问时间和修改时间是否合理
            assert!(atime.unix_seconds() > 0);
            assert!(mtime.unix_seconds() > 0);
        }

        #[test]
        fn test_stat_symlink() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");
            let symlink_path = dir.path().join("test_symlink");

            // 创建一个文件并写入一些数据
            {
                let mut file = File::create(&file_path).unwrap();
                writeln!(file, "Hello, world!").unwrap();
            }

            // 创建符号链接
            symlink(&file_path, &symlink_path).unwrap();

            // 获取符号链接本身的元数据
            let (atime, mtime) = touch_stat(&symlink_path, false).unwrap();

            // 检查访问时间和修改时间是否合理
            assert!(atime.unix_seconds() > 0);
            assert!(mtime.unix_seconds() > 0);

            // 获取符号链接指向的文件的元数据
            let (atime, mtime) = touch_stat(&symlink_path, true).unwrap();

            // 检查访问时间和修改时间是否合理
            assert!(atime.unix_seconds() > 0);
            assert!(mtime.unix_seconds() > 0);
        }

        #[test]
        fn test_stat_nonexistent_file() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("nonexistent_file.txt");

            // 尝试获取不存在的文件的元数据
            let result = touch_stat(&file_path, true);

            // 检查结果是否为错误
            assert!(result.is_err());
        }

        #[test]
        fn test_stat_following_dangling_symlink_reports_missing_referent() {
            let dir = tempdir().unwrap();
            let symlink_path = dir.path().join("dangling");
            symlink("missing-target", &symlink_path).unwrap();

            assert!(touch_stat(&symlink_path, true).is_err());
        }

        #[test]
        fn test_stat_directory() {
            let dir = tempdir().unwrap();
            let dir_path = dir.path().join("test_dir");

            // 创建一个目录
            create_dir(&dir_path).unwrap();

            // 获取目录的元数据
            let (atime, mtime) = touch_stat(&dir_path, true).unwrap();

            // 检查访问时间和修改时间是否合理
            assert!(atime.unix_seconds() > 0);
            assert!(mtime.unix_seconds() > 0);
        }

        #[test]
        fn test_stat_nested_symlink() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");
            let symlink_path1 = dir.path().join("test_symlink1");
            let symlink_path2 = dir.path().join("test_symlink2");

            // 创建一个文件并写入一些数据
            {
                let mut file = File::create(&file_path).unwrap();
                writeln!(file, "Hello, world!").unwrap();
            }

            // 创建符号链接
            symlink(&file_path, &symlink_path1).unwrap();
            symlink(&symlink_path1, &symlink_path2).unwrap();

            // 获取嵌套符号链接本身的元数据
            let (atime, mtime) = touch_stat(&symlink_path2, false).unwrap();

            // 检查访问时间和修改时间是否合理
            assert!(atime.unix_seconds() > 0);
            assert!(mtime.unix_seconds() > 0);

            // 获取嵌套符号链接指向的文件的元数据
            let (atime, mtime) = touch_stat(&symlink_path2, true).unwrap();

            // 检查访问时间和修改时间是否合理
            assert!(atime.unix_seconds() > 0);
            assert!(mtime.unix_seconds() > 0);
        }
    }

    #[cfg(test)]
    mod filetime_to_datetime_tests {
        use chrono::{FixedOffset, TimeZone, Utc};

        use super::*;

        #[test]
        fn test_filetime_to_datetime() {
            // 测试Unix时间戳转换
            let filetime = FileTime::from_unix_time(1623742200, 0); // 对应于2021-06-15 08:30:00 UTC
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的DateTime是否与预期值匹配
            assert_eq!(dt.year(), 2021);
            assert_eq!(dt.month(), 6);
            assert_eq!(dt.day(), 15);
            // 使用时间戳对比而不是具体小时，以避免时区问题
            assert_eq!(dt.timestamp(), 1623742200);
            assert_eq!(dt.minute(), 30);
            assert_eq!(dt.second(), 0);
            assert_eq!(dt.nanosecond(), 0);

            // 测试带纳秒部分的Unix时间戳转换
            let filetime = FileTime::from_unix_time(1623742200, 123456789);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的DateTime是否与预期值匹配
            assert_eq!(dt.year(), 2021);
            assert_eq!(dt.month(), 6);
            assert_eq!(dt.day(), 15);
            // 使用时间戳对比而不是具体小时，以避免时区问题
            assert_eq!(dt.timestamp(), 1623742200);
            assert_eq!(dt.minute(), 30);
            assert_eq!(dt.second(), 0);
            assert_eq!(dt.nanosecond(), 123456789);

            // 测试Unix Epoch (1970-01-01 00:00:00 UTC)
            let filetime = FileTime::from_unix_time(0, 0);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的DateTime是否与预期值匹配
            assert_eq!(dt.year(), 1970);
            assert_eq!(dt.month(), 1);
            assert_eq!(dt.day(), 1);
            // 使用时间戳对比而不是具体小时，以避免时区问题
            assert_eq!(dt.timestamp(), 0);
            assert_eq!(dt.minute(), 0);
            assert_eq!(dt.second(), 0);
            assert_eq!(dt.nanosecond(), 0);

            // 测试闰年日期 (2000-02-29 21:50:15)
            let filetime = FileTime::from_unix_time(951832215, 0);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的DateTime是否与预期值匹配
            assert_eq!(dt.year(), 2000);
            assert_eq!(dt.month(), 2);
            assert_eq!(dt.day(), 29);
            // 使用时间戳对比而不是具体小时，以避免时区问题
            assert_eq!(dt.timestamp(), 951832215);
            assert_eq!(dt.minute(), 50);
            assert_eq!(dt.second(), 15);
            assert_eq!(dt.nanosecond(), 0);

            // 测试2025年1月1日 7:59:59.999999999
            let filetime = FileTime::from_unix_time(1735689599, 999999999);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 使用时间戳对比而不是具体小时，以避免时区问题
            assert_eq!(dt.timestamp(), 1735689599);
            assert_eq!(dt.nanosecond(), 999999999);
        }

        #[test]
        fn test_timezone_conversion() {
            // 测试不同的时区
            let ny_tz = FixedOffset::west_opt(5 * 3600).unwrap(); // 纽约时区 (UTC-5)
            let dt_ny = ny_tz.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt_ny);
            let dt_converted = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的时间戳是否匹配
            assert_eq!(dt_converted.timestamp(), dt_ny.timestamp());

            // 测试UTC时间
            let dt_utc = Utc.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt_utc);
            let dt_converted = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的时间戳是否匹配
            assert_eq!(dt_converted.timestamp(), dt_utc.timestamp());

            // 测试亚洲东京时区 (UTC+9)
            let tokyo_tz = FixedOffset::east_opt(9 * 3600).unwrap();
            let dt_tokyo = tokyo_tz.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt_tokyo);
            let dt_converted = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的时间戳是否匹配
            assert_eq!(dt_converted.timestamp(), dt_tokyo.timestamp());
        }
    }

    #[cfg(test)]
    mod datetime_to_filetime_tests {
        use chrono::Utc;
        use chrono::{FixedOffset, Local, TimeZone};

        use super::*;

        #[test]
        fn test_datetime_to_filetime() {
            // 测试时间为2024-01-01 12:00:00
            let dt = Local.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt);

            // 使用dt.timestamp()而不是硬编码的时间戳，以适应不同时区
            assert_eq!(filetime.unix_seconds(), dt.timestamp());
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试时间为2024-05-11 23:59:59.123456789
            let dt = Local
                .with_ymd_and_hms(2024, 5, 11, 23, 59, 59)
                .unwrap()
                .with_nanosecond(123456789)
                .unwrap();
            let filetime = touch_datetime_to_filetime(&dt);

            assert_eq!(filetime.unix_seconds(), dt.timestamp());
            assert_eq!(filetime.nanoseconds(), 123456789);

            // 测试时间为1970-01-01 00:00:00 (Unix Epoch)
            let dt = Local.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt);

            assert_eq!(filetime.unix_seconds(), dt.timestamp());
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试时间为2000-02-29 06:30:15 (闰年日期)
            let dt = Local.with_ymd_and_hms(2000, 2, 29, 6, 30, 15).unwrap();
            let filetime = touch_datetime_to_filetime(&dt);

            assert_eq!(filetime.unix_seconds(), dt.timestamp());
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试时间为2024-12-31 23:59:59.999999999
            let dt = Local
                .with_ymd_and_hms(2024, 12, 31, 23, 59, 59)
                .unwrap()
                .with_nanosecond(999999999)
                .unwrap();
            let filetime = touch_datetime_to_filetime(&dt);

            assert_eq!(filetime.unix_seconds(), dt.timestamp());
            assert_eq!(filetime.nanoseconds(), 999999999);
        }

        #[test]
        fn test_filetime_to_datetime() {
            // 测试Unix时间戳1704100800 (2024-01-01 12:00:00)
            let filetime = FileTime::from_unix_time(1704100800, 0);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 使用时间戳对比
            assert_eq!(dt.timestamp(), 1704100800);

            // 测试Unix时间戳1715817599和纳秒数123456789 (2024-05-11 23:59:59.123456789)
            let filetime = FileTime::from_unix_time(1715817599, 123456789);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 使用时间戳对比而不是具体日期，以避免日期变化导致的问题
            assert_eq!(dt.timestamp(), 1715817599);
            assert_eq!(dt.nanosecond(), 123456789);

            // 测试Unix时间戳0 (1970-01-01 00:00:00)
            let filetime = FileTime::from_unix_time(0, 0);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            assert_eq!(dt.year(), 1970);
            assert_eq!(dt.month(), 1);
            assert_eq!(dt.day(), 1);
            // 使用时间戳对比而不是具体小时，以避免时区问题
            assert_eq!(dt.timestamp(), 0);
            assert_eq!(dt.second(), 0);
            assert_eq!(dt.nanosecond(), 0);

            // 测试Unix时间戳951832215 (2000-02-29 21:50:15)
            let filetime = FileTime::from_unix_time(951832215, 0);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            assert_eq!(dt.year(), 2000);
            assert_eq!(dt.month(), 2);
            assert_eq!(dt.day(), 29);
            // 使用时间戳对比而不是具体小时，以避免时区问题
            assert_eq!(dt.timestamp(), 951832215);
            assert_eq!(dt.second(), 15);
            assert_eq!(dt.nanosecond(), 0);

            // 测试Unix时间戳1735689599和纳秒数999999999 (2025-1-1 7:59:59.999999999)
            let filetime = FileTime::from_unix_time(1735689599, 999999999);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 使用时间戳对比而不是具体年份，以避免年份变化导致的问题
            assert_eq!(dt.timestamp(), 1735689599);
            assert_eq!(dt.nanosecond(), 999999999);
        }

        #[test]
        fn test_timezone_conversion() {
            // 测试不同的时区
            let ny_tz = FixedOffset::west_opt(5 * 3600).unwrap(); // 纽约时区 (UTC-5)
            let dt_ny = ny_tz.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt_ny);
            let dt_converted = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的时间戳是否匹配
            assert_eq!(dt_converted.timestamp(), dt_ny.timestamp());

            // 测试UTC时间
            let dt_utc = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt_utc);
            let dt_converted = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的时间戳是否匹配
            assert_eq!(dt_converted.timestamp(), dt_utc.timestamp());

            // 测试亚洲东京时区 (UTC+9)
            let tokyo_tz = FixedOffset::east_opt(9 * 3600).unwrap();
            let dt_tokyo = tokyo_tz.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt_tokyo);
            let dt_converted = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的时间戳是否匹配
            assert_eq!(dt_converted.timestamp(), dt_tokyo.timestamp());
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use ctcore::ct_error::set_ct_exit_code;
        use tempfile::tempdir;

        #[cfg(unix)]
        use std::os::unix::ffi::OsStrExt;

        use super::*;

        #[test]
        fn test_touch_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = touch_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = touch_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_source_conflict_uses_gnu_diagnostic() {
            let args = [
                ctcore::ct_util_name(),
                "-d",
                "2020-01-01",
                "-t",
                "202001010000",
                "target",
            ];
            let error = touch_main(args.iter().map(OsString::from)).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "cannot specify times from more than one source\nTry '{} --help' for more information.",
                    ctcore::ct_help_utility_name()
                )
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_touch_open_error_quotes_non_utf8_path_like_gnu_quoteaf() {
            let error = std::io::Error::from_raw_os_error(ctcore::libc::ENOENT);

            assert_eq!(
                touch_open_error_message(OsStr::from_bytes(b"\xff/missing"), &error),
                "cannot touch ''$'\\377''/missing': No such file or directory"
            );
        }

        #[test]
        fn test_touch_main_continues_after_target_error() {
            let dir = tempdir().unwrap();
            let regular_file = dir.path().join("regular");
            let valid_file = dir.path().join("valid");
            File::create(&regular_file).unwrap();
            let invalid_path = format!("{}/", regular_file.display());
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from(invalid_path),
                valid_file.clone().into_os_string(),
            ];

            let result = touch_main(args.into_iter());
            set_ct_exit_code(0);

            assert!(result.is_ok());
            assert!(valid_file.exists());
        }

        #[test]
        fn test_touch_open_for_creation_preserves_existing_content() {
            let dir = tempdir().unwrap();
            let file = dir.path().join("file");
            std::fs::write(&file, b"preserve me").unwrap();

            let _opened = touch_open_for_creation(&file).unwrap();

            assert_eq!(std::fs::read(file).unwrap(), b"preserve me");
        }

        #[cfg(unix)]
        #[test]
        fn test_touch_open_error_distinguishes_directory_from_symlink_loop() {
            use std::os::unix::fs::symlink;

            let dir = tempdir().unwrap();
            let directory_error = touch_open_for_creation(dir.path()).unwrap_err();
            assert!(touch_open_error_is_directory(dir.path(), &directory_error));

            let loop_path = dir.path().join("loop");
            symlink("loop", &loop_path).unwrap();
            let loop_error = touch_open_for_creation(&loop_path).unwrap_err();
            assert!(!touch_open_error_is_directory(&loop_path, &loop_error));
        }

        #[test]
        fn test_touch_open_error_message_omits_rust_errno_suffix() {
            let error = io::Error::from_raw_os_error(ctcore::libc::ELOOP);

            assert_eq!(
                touch_open_error_message(OsStr::new("loop"), &error),
                "cannot touch 'loop': Too many levels of symbolic links"
            );
        }

        #[test]
        fn test_touch_setting_times_error_message_preserves_eperm() {
            let error = io::Error::from_raw_os_error(ctcore::libc::EPERM);

            assert_eq!(
                touch_setting_times_error_message(OsStr::new("root-owned"), &error),
                "setting times of 'root-owned': Operation not permitted"
            );
        }

        #[test]
        fn test_touch_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()]; // 缺少任何参数
            let error = touch_main(args.iter().map(OsString::from)).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "missing file operand\nTry '{} --help' for more information.",
                    ctcore::ct_help_utility_name()
                )
            );
        }

        #[test]
        fn test_touch_main_access_time_short() {
            let filename = "test_touch_main_access_time_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-a", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_instead_of_the_current_time_short() {
            let filename = "test_touch_main_instead_of_the_current_time_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-t", "12011233", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_parse_the_current_time_long() {
            let filename = "test_touch_main_parse_the_current_time_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--date", "@2147483647", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_parse_the_current_time_short() {
            let filename = "test_touch_main_parse_the_current_time_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-d", "@2147483647", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_change_modification_time_short() {
            let filename = "test_touch_main_change_modification_time_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-m", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_no_create_long() {
            let filename = "test_touch_main_no_create_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--no-create", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_no_create_short() {
            let filename = "test_touch_main_no_create_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-c", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_no_dereference_short() {
            let filename = "test_touch_main_no_dereference_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-h", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_no_dereference_long() {
            let filename = "test_touch_main_no_dereference_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--no-dereference", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_reference_long() {
            let filename = "test_touch_main_reference_long";
            let reference_filename = "reference_file";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let reference_path = dir.path().join(reference_filename);
            let _ = File::create(&reference_path).unwrap();
            let reference_file_name = reference_path.to_str().unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--reference",
                reference_file_name,
                "--no-dereference",
                file_name,
            ];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_reference_short() {
            let filename = "test_touch_main_reference_short";
            let reference_filename = "reference_file";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let reference_path = dir.path().join(reference_filename);
            let _ = File::create(&reference_path).unwrap();
            let reference_file_name = reference_path.to_str().unwrap();

            let args = [
                ctcore::ct_util_name(),
                "-r",
                reference_file_name,
                "--no-dereference",
                file_name,
            ];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_access() {
            let filename = "test_touch_main_time_long_access";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--time", "access", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_atime() {
            let filename = "test_touch_main_time_long_atime";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--time", "atime", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_use() {
            let filename = "test_touch_main_time_long_use";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--time", "use", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_modify() {
            let filename = "test_touch_main_time_long_modify";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--time", "modify", file_name];
            let result = touch_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_mtime() {
            let file_name = "test_touch_main_time_long_mtime";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--time", "mtime", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_force_short() {
            let file_name = "test_touch_main_force_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-f", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;
        #[cfg(unix)]
        use std::os::unix::ffi::OsStringExt;
        use std::sync::Mutex;

        use super::*;

        static POSIXLY_CORRECT_LOCK: Mutex<()> = Mutex::new(());

        // touch 接口: touch [OPTION]... FILE...
        //
        // Arguments:
        //   [files]...
        //
        // Options:
        //       --help              Print help information.
        //   -a                      change only the access time
        //   -t <STAMP>              use [[CC]YY]MMDDhhmm[.ss] instead of the current time
        //   -d, --date <STRING>     parse argument and use it instead of current time
        //   -m                      change only the modification time
        //   -c, --no-create         do not create any files
        //   -h, --no-dereference    affect each symbolic link instead of any referenced file (only for systems that can change the timestamps of a symlink)
        //   -r, --reference <FILE>  use this file's times instead of the current time
        //       --time <WORD>       change only the specified time: "access", "atime", or "use" are equivalent to -a; "modify" or "mtime" are equivalent to -m [possible values: access, atime, use, modify, mtime]
        //   -V, --version           Print version

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
        fn test_touch_prepare_args_uses_gnu_option_diagnostics() {
            let cases = [
                (
                    "--no-c=ignored",
                    b"option '--no-create' doesn't allow an argument".as_slice(),
                ),
                (
                    "--no=ignored",
                    b"option '--no=ignored' is ambiguous; possibilities: '--no-create' '--no-dereference'"
                        .as_slice(),
                ),
                ("--unknown", b"unrecognized option '--unknown'".as_slice()),
                ("-c=ignored", b"invalid option -- '='".as_slice()),
            ];

            for (argument, expected) in cases {
                let error = touch_prepare_args_with_mode(
                    [OsString::from("touch"), OsString::from(argument)].into_iter(),
                    false,
                )
                .expect_err("GNU option error must be detected before clap");
                assert_eq!(error.diagnostic_bytes().as_ref(), expected);
                assert!(error.usage());
            }
        }

        #[test]
        fn test_touch_prepare_args_uses_gnu_missing_option_value_diagnostics() {
            let cases = [
                ("--date", b"option '--date' requires an argument".as_slice()),
                ("--da", b"option '--date' requires an argument".as_slice()),
                (
                    "--reference",
                    b"option '--reference' requires an argument".as_slice(),
                ),
                ("--time", b"option '--time' requires an argument".as_slice()),
                ("-d", b"option requires an argument -- 'd'".as_slice()),
                ("-r", b"option requires an argument -- 'r'".as_slice()),
                ("-t", b"option requires an argument -- 't'".as_slice()),
            ];

            for (argument, expected) in cases {
                let error = touch_prepare_args_with_mode(
                    [OsString::from("touch"), OsString::from(argument)].into_iter(),
                    false,
                )
                .expect_err("GNU missing option value must be detected before clap");
                assert_eq!(error.diagnostic_bytes().as_ref(), expected);
                assert!(error.usage());
            }
        }

        #[test]
        fn test_touch_prepare_args_uses_gnu_time_word_diagnostics() {
            let cases = [
                (vec!["--time=x"], b"x".as_slice(), "invalid"),
                (vec!["--time="], b"".as_slice(), "ambiguous"),
                (vec!["--ti", "x"], b"x".as_slice(), "invalid"),
            ];

            for (arguments, value, kind) in cases {
                let arguments = std::iter::once(OsString::from("touch"))
                    .chain(arguments.into_iter().map(OsString::from));
                let error = touch_prepare_args_with_mode(arguments, false)
                    .expect_err("GNU --time value error must be detected before clap");
                assert_eq!(
                    error.diagnostic_bytes().as_ref(),
                    touch_time_word_diagnostic_message(value, kind).as_bytes()
                );
                assert!(error.usage());
            }
        }

        #[test]
        fn test_touch_time_word_diagnostic_quotes_non_utf8_bytes() {
            assert_eq!(
                touch_time_word_diagnostic_message_with_marks(b"\xff", "invalid", "'", "'"),
                "invalid argument '\\377' for '--time'\nValid arguments are:\n  - 'atime', 'access', 'use'\n  - 'mtime', 'modify'"
            );
        }

        #[test]
        fn test_touch_invalid_date_format_quotes_multibyte_bytes_in_c_locale() {
            assert_eq!(
                touch_invalid_date_format_with_marks(b"202402291234.\xc3\xa9", "'", "'"),
                "invalid date format '202402291234.\\303\\251'"
            );
        }

        #[test]
        fn test_touch_prepare_args_skips_option_values_and_honors_posix_mode() {
            let date_value = [
                OsString::from("touch"),
                OsString::from("-d"),
                OsString::from("--no-create=ignored"),
                OsString::from("target"),
            ];
            assert_eq!(
                touch_prepare_args_with_mode(date_value.clone().into_iter(), false).unwrap(),
                date_value
            );

            let posix_args = [
                OsString::from("touch"),
                OsString::from("target"),
                OsString::from("--no-create=ignored"),
            ];
            assert_eq!(
                touch_prepare_args_with_mode(posix_args.clone().into_iter(), true).unwrap(),
                posix_args
            );
        }

        #[test]
        fn test_touch_prepare_args_preserves_attached_short_option_values() {
            for argument in ["-d2024-02-29", "-t202402290000", "-r--"] {
                let args = [OsString::from("touch"), OsString::from(argument)];
                assert_eq!(
                    touch_prepare_args_with_mode(args.clone().into_iter(), false).unwrap(),
                    args
                );
            }
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_posixly_correct_stops_option_parsing_at_first_operand() {
            let _guard = POSIXLY_CORRECT_LOCK.lock().unwrap();
            let previous = std::env::var_os("POSIXLY_CORRECT");
            unsafe { std::env::set_var("POSIXLY_CORRECT", "1") };
            let result = ct_app().try_get_matches_from([ctcore::ct_util_name(), "first", "-a"]);
            match previous {
                Some(value) => unsafe { std::env::set_var("POSIXLY_CORRECT", value) },
                None => unsafe { std::env::remove_var("POSIXLY_CORRECT") },
            }

            let matches = result.unwrap();
            assert!(!touch_option_is_set(&matches, touch_flags::TOUCH_ACCESS));
            assert_eq!(
                matches
                    .get_many::<OsString>(TOUCH_ARG_FILES)
                    .unwrap()
                    .map(OsString::as_os_str)
                    .collect::<Vec<_>>(),
                [std::ffi::OsStr::new("first"), std::ffi::OsStr::new("-a")]
            );
        }

        #[test]
        fn test_ct_app_accepts_hyphen_prefixed_option_values() {
            let reference = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), "-r", "-missing", "target"])
                .unwrap();
            assert_eq!(
                reference
                    .get_many::<OsString>(touch_flags::sources::TOUCH_REFERENCE)
                    .unwrap()
                    .next_back()
                    .unwrap(),
                &OsString::from("-missing")
            );

            let timestamp = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), "-t", "-stamp", "target"])
                .unwrap();
            assert_eq!(
                timestamp
                    .get_many::<OsString>(touch_flags::sources::TOUCH_TIMESTAMP)
                    .unwrap()
                    .next_back()
                    .unwrap(),
                &OsString::from("-stamp")
            );

            let error = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), "--time", "-bad", "target"])
                .unwrap_err();
            assert!(matches!(
                error.kind(),
                ErrorKind::InvalidValue | ErrorKind::ValueValidation
            ));
        }

        #[cfg(unix)]
        #[test]
        fn test_ct_app_preserves_non_utf8_date_source_values() {
            for (option, source) in [
                ("-d", touch_flags::sources::TOUCH_DATE),
                ("-t", touch_flags::sources::TOUCH_TIMESTAMP),
            ] {
                let matches = ct_app()
                    .try_get_matches_from([
                        OsString::from(ctcore::ct_util_name()),
                        OsString::from(option),
                        OsString::from_vec(vec![0xff]),
                        OsString::from("target"),
                    ])
                    .expect("date source values must preserve non-UTF-8 bytes");
                assert_eq!(
                    matches
                        .get_one::<OsString>(source)
                        .unwrap()
                        .as_encoded_bytes(),
                    b"\xff"
                );
            }
        }

        #[test]
        fn test_ct_app_access_time_short() {
            let file_name = "test_ct_app_access_time_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-a", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_accepts_repeated_boolean_options() {
            for arguments in [
                ["-aa", "file"],
                ["-mm", "file"],
                ["-cc", "file"],
                ["-hh", "file"],
                ["-ff", "file"],
            ] {
                ct_app()
                    .try_get_matches_from([ctcore::ct_util_name(), arguments[0], arguments[1]])
                    .expect("GNU-compatible repeated flag must be accepted");
            }
        }

        #[test]
        fn test_ct_app_instead_of_the_current_time_short() {
            let file_name = "test_ct_app_instead_of_the_current_time_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-t", "12011233", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_parse_the_current_time_long() {
            let file_name = "test_ct_app_parse_the_current_time_long";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--date", "@2147483647", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_parse_the_current_time_short() {
            let file_name = "test_ct_app_parse_the_current_time_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-d", "@2147483647", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_change_modification_time_short() {
            let file_name = "test_ct_app_change_modification_time_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-m", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_create_long() {
            let file_name = "test_ct_app_no_create_long";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--no-create", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_create_short() {
            let file_name = "test_ct_app_no_create_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-c", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_dereference_short() {
            let file_name = "test_ct_app_no_dereference_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-h", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_dereference_long() {
            let file_name = "test_ct_app_no_dereference_long";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--no-dereference", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_reference_long() {
            let file_name = "test_ct_app_reference_long";
            let reference_file_name = "reference_file";
            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--reference",
                reference_file_name,
                "--no-dereference",
                file_name,
            ];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_reference_short() {
            let file_name = "test_ct_app_reference_short";
            let reference_file_name = "reference_file";
            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                reference_file_name,
                "--no-dereference",
                file_name,
            ];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_time_long_access() {
            let file_name = "test_ct_app_time_long_access";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--time", "access", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_time_long_atime() {
            let file_name = "test_ct_app_time_long_atime";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--time", "atime", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_time_long_use() {
            let file_name = "test_ct_app_time_long_use";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--time", "use", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_time_long_modify() {
            let file_name = "test_ct_app_time_long_modify";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--time", "modify", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_time_long_mtime() {
            let file_name = "test_ct_app_time_long_mtime";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--time", "mtime", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_time_accepts_group_prefixes_and_repeats() {
            let matches = ct_app()
                .try_get_matches_from([
                    ctcore::ct_util_name(),
                    "--time",
                    "a",
                    "--time",
                    "m",
                    "file",
                ])
                .unwrap();

            let values = matches
                .get_many::<String>(touch_flags::TOUCH_TIME)
                .unwrap()
                .map(String::as_str)
                .collect::<Vec<_>>();
            assert_eq!(values, ["access", "modify"]);
        }

        #[test]
        fn test_ct_app_force_short() {
            let file_name = "test_ct_app_force_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-f", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }

    #[cfg(windows)]
    #[test]
    fn test_get_pathbuf_from_stdout_fails_if_stdout_is_not_a_file() {
        // 我们可以通过不设置stdout来触发错误（将失败，代码为1）
        assert!(
            super::touch_pathbuf_from_stdout()
                .expect_err("pathbuf_from_stdout should have failed")
                .to_string()
                .contains("GetFinalPathNameByHandleW failed with code 1")
        );
    }

    #[cfg(test)]
    mod date_parsing_tests {
        use super::*;
        use chrono::{Datelike, FixedOffset, Local, TimeZone, Weekday};

        #[test]
        fn test_parse_posix_locale_datetime_uses_local_wall_time() {
            let new_york_standard_time = FixedOffset::west_opt(5 * 60 * 60).unwrap();
            let parsed = touch_parse_posix_locale_datetime(
                "Mon Jan  1 12:00:00 2024",
                new_york_standard_time,
            )
            .unwrap();

            assert_eq!(parsed, FileTime::from_unix_time(1_704_128_400, 0));
        }

        #[test]
        fn test_parse_weekday_next_friday() {
            // 使用固定的参考时间: 2025年7月24日 (星期四)
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            // 测试 "next Friday" 应该返回 2025年7月25日
            let result = touch_parse_date(ref_time, "next Friday");
            assert!(
                result.is_ok(),
                "Failed to parse 'next Friday': {:?}",
                result.err()
            );

            let file_time = result.unwrap();
            let dt = Local
                .timestamp_opt(file_time.unix_seconds(), file_time.nanoseconds())
                .unwrap();

            assert_eq!(dt.weekday(), Weekday::Fri);
            assert_eq!(dt.year(), 2025);
            assert_eq!(
                dt.month(),
                7,
                "Expected July (month 7), got month {}",
                dt.month()
            );
            assert_eq!(dt.day(), 25);
        }

        #[test]
        fn test_parse_weekday_last_monday() {
            // 使用固定的参考时间: 2025年7月24日 (星期四)
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            // 测试 "last Monday" 应该返回 2025年7月21日
            let result = touch_parse_date(ref_time, "last Monday");
            assert!(
                result.is_ok(),
                "Failed to parse 'last Monday': {:?}",
                result.err()
            );

            let file_time = result.unwrap();
            let dt = Local
                .timestamp_opt(file_time.unix_seconds(), file_time.nanoseconds())
                .unwrap();

            assert_eq!(dt.weekday(), Weekday::Mon);
            assert_eq!(dt.month(), 7);
            assert_eq!(dt.day(), 21);
            assert_eq!(dt.year(), 2025);
        }

        #[test]
        fn test_parse_weekday_plain_wednesday() {
            // 使用固定的参考时间: 2025年7月24日 (星期四)
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            // 测试 "Wednesday" 应该返回下个星期三 2025年7月30日
            let result = touch_parse_date(ref_time, "Wednesday");
            assert!(
                result.is_ok(),
                "Failed to parse 'Wednesday': {:?}",
                result.err()
            );

            let file_time = result.unwrap();
            let dt = Local
                .timestamp_opt(file_time.unix_seconds(), file_time.nanoseconds())
                .unwrap();

            assert_eq!(dt.weekday(), Weekday::Wed);
            assert_eq!(dt.month(), 7);
            assert_eq!(dt.day(), 30);
            assert_eq!(dt.year(), 2025);
        }

        #[test]
        fn test_parse_weekday_this_saturday() {
            // 使用固定的参考时间: 2025年7月24日 (星期四)
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            // 测试 "this Saturday" 应该返回本周六 2025年7月26日
            let result = touch_parse_date(ref_time, "this Saturday");
            assert!(
                result.is_ok(),
                "Failed to parse 'this Saturday': {:?}",
                result.err()
            );

            let file_time = result.unwrap();
            let dt = Local
                .timestamp_opt(file_time.unix_seconds(), file_time.nanoseconds())
                .unwrap();

            assert_eq!(dt.weekday(), Weekday::Sat);
            assert_eq!(dt.month(), 7);
            assert_eq!(dt.day(), 26);
            assert_eq!(dt.year(), 2025);
        }

        #[test]
        fn test_parse_weekday_case_insensitive() {
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            // 测试大小写不敏感
            let test_cases = vec!["FRIDAY", "Friday", "friday", "FrIdAy"];

            for case in test_cases {
                let result = touch_parse_date(ref_time, case);
                assert!(
                    result.is_ok(),
                    "Failed to parse '{}': {:?}",
                    case,
                    result.err()
                );

                let file_time = result.unwrap();
                let dt = Local
                    .timestamp_opt(file_time.unix_seconds(), file_time.nanoseconds())
                    .unwrap();
                assert_eq!(
                    dt.weekday(),
                    Weekday::Fri,
                    "Case '{case}' did not parse to Friday"
                );
            }
        }

        #[test]
        fn test_parse_weekday_abbreviations() {
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            // 测试星期几缩写
            let test_cases = vec![
                ("tues", Weekday::Tue),
                ("wednes", Weekday::Wed),
                ("thur", Weekday::Thu),
                ("thurs", Weekday::Thu),
            ];

            for (abbrev, expected_weekday) in test_cases {
                let result = touch_parse_date(ref_time, abbrev);
                assert!(
                    result.is_ok(),
                    "Failed to parse '{}': {:?}",
                    abbrev,
                    result.err()
                );

                let file_time = result.unwrap();
                let dt = Local
                    .timestamp_opt(file_time.unix_seconds(), file_time.nanoseconds())
                    .unwrap();
                assert_eq!(
                    dt.weekday(),
                    expected_weekday,
                    "Abbreviation '{abbrev}' did not parse to {expected_weekday:?}"
                );
            }
        }

        #[test]
        fn test_parse_relative_time() {
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            // 测试相对时间表达式
            let test_cases = vec![("tomorrow", 25), ("yesterday", 23), ("today", 24)];

            for (expr, expected_day) in test_cases {
                let result = touch_parse_date(ref_time, expr);
                assert!(
                    result.is_ok(),
                    "Failed to parse '{}': {:?}",
                    expr,
                    result.err()
                );

                let file_time = result.unwrap();
                let dt = Local
                    .timestamp_opt(file_time.unix_seconds(), file_time.nanoseconds())
                    .unwrap();
                assert_eq!(
                    dt.day(),
                    expected_day,
                    "Expression '{expr}' did not parse to day {expected_day}"
                );
            }
        }

        #[test]
        fn test_parse_existing_formats_still_work() {
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            // 确保现有的解析格式仍然有效
            let test_cases = vec![
                "2025-12-25",  // ISO 8601 格式
                "@1735689600", // Unix timestamp (2025-01-01 00:00:00 UTC)
                "1 week",      // parse_datetime crate 格式
                "next month",  // parse_datetime crate 格式
            ];

            for case in test_cases {
                let result = touch_parse_date(ref_time, case);
                assert!(
                    result.is_ok(),
                    "Failed to parse existing format '{}': {:?}",
                    case,
                    result.err()
                );
            }
        }

        #[test]
        fn test_parse_invalid_input() {
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            // 测试无效输入
            let invalid_cases = vec!["invalid_weekday", "next invalid", "", "random text"];

            for case in invalid_cases {
                let result = touch_parse_date(ref_time, case);
                assert!(
                    result.is_err(),
                    "Expected '{case}' to fail parsing, but it succeeded"
                );
            }
        }

        #[test]
        fn test_parse_date_rejects_leap_second() {
            let ref_time = Local.with_ymd_and_hms(2025, 7, 24, 12, 0, 0).unwrap();

            for input in ["2024-01-01 12:34:60", "Mon Jan  1 12:34:60 2024"] {
                assert!(
                    touch_parse_date(ref_time, input).is_err(),
                    "input {input} should be rejected"
                );
            }
        }
    }
}
