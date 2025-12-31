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

//! touch 用于更新文件或目录访问和修改时间戳的命令行工具。
//! 如果指定的文件不存在，touch命令会创建一个空文件。
//! 主要用于以下几种情况：
//! 1.创建新文件：当仅需创建一个空文件时，无需编辑内容，直接使用touch命令即可。
//! 2.更新时间戳：可以用来更新文件的访问时间和修改时间（atime和mtime），使之看起来像是最近被访问或修改过。

use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone, Timelike,
};
use clap::builder::ValueParser;
use clap::{crate_version, Arg, ArgAction, ArgGroup, ArgMatches, Command};
use filetime::{set_file_times, set_symlink_file_times, FileTime};

use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show};

const TOUCH_ABOUT: &str = ct_help_about!("touch.md");
const TOUCH_USAGE: &str = ct_help_usage!("touch.md");

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
}

static TOUCH_ARG_FILES: &str = "files";

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
    // "%Y-%m-%d %H:%M +offset"
    // 用于tests/touch/relative.sh中的示例
    pub(crate) const YYYYMMDDHHMM_OFFSET: &str = "%Y-%m-%d %H:%M %z";
}

/// 将具有TZ偏移量的DateTime转换为FileTime
/// DateTime将转换为Unix时间戳，从中构建FileTime。
fn touch_datetime_to_filetime<T: TimeZone>(dt: &DateTime<T>) -> FileTime {
    FileTime::from_unix_time(dt.timestamp(), dt.timestamp_subsec_nanos())
}

fn touch_filetime_to_datetime(ft: &FileTime) -> Option<DateTime<Local>> {
    Some(DateTime::from_timestamp(ft.unix_seconds(), ft.nanoseconds())?.into())
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    touch_main(args)
}

pub fn touch_main(args: impl ctcore::Args) -> CTResult<()> {
    let arg_matches = ct_app().try_get_matches_from(args)?;

    let files = arg_matches
        .get_many::<OsString>(TOUCH_ARG_FILES)
        .ok_or_else(|| {
            let err_message = format!(
                "missing file operand\nTry '{} --help' for more information.",
                ctcore::ct_execute_phrase()
            );
            CtSimpleError::new(1, err_message)
        })?;

    let (a_time, m_time) = touch_determine_times(&arg_matches)?;

    for filename in files {
        // FIXME: 找到避免必须克隆路径的方法
        let path_buf = if filename == "-" {
            touch_pathbuf_from_stdout()?
        } else {
            PathBuf::from(filename)
        };

        let path = path_buf.as_path();

        let md_result = if arg_matches.get_flag(touch_flags::TOUCH_NO_DEREF) {
            path.symlink_metadata()
        } else {
            path.metadata()
        };

        if let Err(e) = md_result {
            if e.kind() != std::io::ErrorKind::NotFound {
                let err_message = format!("setting times of {}", filename.quote());
                return Err(e.map_err_context(|| err_message));
            }

            if arg_matches.get_flag(touch_flags::TOUCH_NO_CREATE) {
                continue;
            }

            if arg_matches.get_flag(touch_flags::TOUCH_NO_DEREF) {
                let err_message = format!(
                    "setting times of {}: No such file or directory",
                    filename.quote()
                );
                ct_show!(CtSimpleError::new(1, err_message));
                continue;
            }

            if let Err(e) = File::create(path) {
                let err_message = format!("cannot touch {}", path.quote());
                ct_show!(e.map_err_context(|| err_message));
                continue;
            };

            // 小优化：如果没有指定参考时间，我们就完成了。
            if !arg_matches.contains_id(touch_flags::TOUCH_SOURCES) {
                continue;
            }
        }

        touch_update_times(&arg_matches, path, a_time, m_time, filename)?;
    }
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = TOUCH_ABOUT;
    let usage_description = ct_format_usage(TOUCH_USAGE);
    let args = vec![
        Arg::new(touch_flags::TOUCH_HELP)
            .long(touch_flags::TOUCH_HELP)
            .help("Print help information.")
            .action(ArgAction::Help),
        Arg::new(touch_flags::TOUCH_ACCESS)
            .short('a')
            .help("change only the access time")
            .action(ArgAction::SetTrue),
        Arg::new(touch_flags::sources::TOUCH_TIMESTAMP)
            .short('t')
            .help("use [[CC]YY]MMDDhhmm[.ss] instead of the current time")
            .value_name("STAMP"),
        Arg::new(touch_flags::sources::TOUCH_DATE)
            .short('d')
            .long(touch_flags::sources::TOUCH_DATE)
            .allow_hyphen_values(true)
            .help("parse argument and use it instead of current time")
            .value_name("STRING")
            .conflicts_with(touch_flags::sources::TOUCH_TIMESTAMP),
        Arg::new(touch_flags::TOUCH_MODIFICATION)
            .short('m')
            .help("change only the modification time")
            .action(ArgAction::SetTrue),
        Arg::new(touch_flags::TOUCH_NO_CREATE)
            .short('c')
            .long(touch_flags::TOUCH_NO_CREATE)
            .help("do not create any files")
            .action(ArgAction::SetTrue),
        Arg::new(touch_flags::TOUCH_NO_DEREF)
            .short('h')
            .long(touch_flags::TOUCH_NO_DEREF)
            .help(
                "affect each symbolic link instead of any referenced file \
                     (only for systems that can change the timestamps of a symlink)",
            )
            .action(ArgAction::SetTrue),
        Arg::new(touch_flags::sources::TOUCH_REFERENCE)
            .short('r')
            .long(touch_flags::sources::TOUCH_REFERENCE)
            .help("use this file's times instead of the current time")
            .value_name("FILE")
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::AnyPath)
            .conflicts_with(touch_flags::sources::TOUCH_TIMESTAMP),
        Arg::new(touch_flags::TOUCH_TIME)
            .long(touch_flags::TOUCH_TIME)
            .help(
                "change only the specified time: \"access\", \"atime\", or \
                     \"use\" are equivalent to -a; \"modify\" or \"mtime\" are \
                     equivalent to -m",
            )
            .value_name("WORD")
            .value_parser(["access", "atime", "use", "modify", "mtime"]),
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
}

// 确定访问和修改时间
fn touch_determine_times(matches: &ArgMatches) -> CTResult<(FileTime, FileTime)> {
    match (
        matches.get_one::<OsString>(touch_flags::sources::TOUCH_REFERENCE),
        matches.get_one::<String>(touch_flags::sources::TOUCH_DATE),
    ) {
        (Some(reference), Some(date)) => {
            let (a_time, m_time) = touch_stat(
                Path::new(&reference),
                !matches.get_flag(touch_flags::TOUCH_NO_DEREF),
            )?;
            let atime = touch_filetime_to_datetime(&a_time).ok_or_else(|| {
                CtSimpleError::new(1, "Could not process the reference access time")
            })?;
            let mtime = touch_filetime_to_datetime(&m_time).ok_or_else(|| {
                CtSimpleError::new(1, "Could not process the reference modification time")
            })?;
            Ok((
                touch_parse_date(atime, date)?,
                touch_parse_date(mtime, date)?,
            ))
        }
        (Some(reference), None) => touch_stat(
            Path::new(&reference),
            !matches.get_flag(touch_flags::TOUCH_NO_DEREF),
        ),
        (None, Some(date)) => {
            let timestamp = touch_parse_date(Local::now(), date)?;
            Ok((timestamp, timestamp))
        }
        (None, None) => {
            let timestamp = if let Some(ts) =
                matches.get_one::<String>(touch_flags::sources::TOUCH_TIMESTAMP)
            {
                parse_timestamp(ts)?
            } else {
                touch_datetime_to_filetime(&Local::now())
            };
            Ok((timestamp, timestamp))
        }
    }
}

// 根据用户指定的选项更新文件访问和修改时间
fn touch_update_times(
    arg_matches: &ArgMatches,
    path: &Path,
    mut a_time: FileTime,
    mut m_time: FileTime,
    file_name: &OsString,
) -> CTResult<()> {
    // 如果仅更改atime或mtime，则获取另一个的现有值。
    // 请注意，"-a"和"-m"可以一起传递；这不是xor。
    if arg_matches.get_flag(touch_flags::TOUCH_ACCESS)
        || arg_matches.get_flag(touch_flags::TOUCH_MODIFICATION)
        || arg_matches.contains_id(touch_flags::TOUCH_TIME)
    {
        let st = touch_stat(path, !arg_matches.get_flag(touch_flags::TOUCH_NO_DEREF))?;
        let time = arg_matches
            .get_one::<String>(touch_flags::TOUCH_TIME)
            .map(|s| s.as_str())
            .unwrap_or("");

        if !(arg_matches.get_flag(touch_flags::TOUCH_ACCESS)
            || time.contains(&"access".to_owned())
            || time.contains(&"atime".to_owned())
            || time.contains(&"use".to_owned()))
        {
            a_time = st.0;
        }

        if !(arg_matches.get_flag(touch_flags::TOUCH_MODIFICATION)
            || time.contains(&"modify".to_owned())
            || time.contains(&"mtime".to_owned()))
        {
            m_time = st.1;
        }
    }

    // 设置文件或符号链接的访问和修改时间, 提供文件名、访问时间（atime）和修改时间（mtime）作为输入。
    // 如果文件名不是"-"，表示touch -h -的特殊情况，
    // 代码检查是否设置了NO_DEREF标志，这意味着用户想要为符号链接本身设置时间，而不是它指向的文件。
    if file_name == "-" {
        filetime::set_file_times(path, a_time, m_time)
    } else if arg_matches.get_flag(touch_flags::TOUCH_NO_DEREF) {
        set_symlink_file_times(path, a_time, m_time)
    } else {
        set_file_times(path, a_time, m_time)
    }
    .map_err_context(|| format!("setting times of {}", path.quote()))
}

// 获取提供路径的元数据
// 如果`follow`为`true`，函数将尝试跟随符号链接
// 如果`follow`为`false`或符号链接损坏，函数将返回符号链接本身的元数据
fn touch_stat(path: &Path, is_follow: bool) -> CTResult<(FileTime, FileTime)> {
    let md = match is_follow {
        true => fs::metadata(path).or_else(|_| fs::symlink_metadata(path)),
        false => fs::symlink_metadata(path),
    }
    .map_err_context(|| format!("failed to get attributes of {}", path.quote()))?;

    Ok((
        FileTime::from_last_access_time(&md),
        FileTime::from_last_modification_time(&md),
    ))
}

fn touch_parse_date(ref_time: DateTime<Local>, s: &str) -> CTResult<FileTime> {
    // 这实际上不兼容GNU touch，但似乎没有
    // 关于此参数允许的日期格式的简单规范，我不打算
    // 实现GNU parse_datetime。
    // http://git.savannah.gnu.org/gitweb/?p=gnulib.git;a=blob_plain;f=lib/parse-datetime.y

    // TODO: 匹配字符数？

    // "当前语言环境的首选日期和时间表示。"
    // "(在POSIX语言环境中这相当于%a %b %e %H:%M:%S %Y。)"
    // time 0.1.43将其解析为'a b e T Y'
    // 这相当于POSIX语言环境：%a %b %e %H:%M:%S %Y
    // 周二12月3日...
    // ("%c", POSIX_LOCALE_FORMAT),
    //
    if let Ok(parsed) = NaiveDateTime::parse_from_str(s, touch_format::POSIX_LOCALE) {
        return Ok(touch_datetime_to_filetime(&parsed.and_utc()));
    }

    // 还支持在GNU测试中找到的其他格式，如
    // 在tests/misc/stat-nanoseconds.sh中
    // 或tests/touch/no-rights.sh中
    for fmt in [
        touch_format::YYYYMMDDHHMMS,
        touch_format::YYYYMMDDHHMMSS,
        touch_format::YYYY_MM_DD_HH_MM,
        touch_format::YYYYMMDDHHMM_OFFSET,
    ] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(touch_datetime_to_filetime(&parsed.and_utc()));
        }
    }

    // "相当于%Y-%m-%d (ISO 8601日期格式)。 (C99)"
    // ("%F", ISO_8601_FORMAT),
    if let Ok(parsed_date) = NaiveDate::parse_from_str(s, touch_format::ISO_8601) {
        let parsed = Local
            .from_local_datetime(&parsed_date.and_time(NaiveTime::MIN))
            .unwrap();
        return Ok(touch_datetime_to_filetime(&parsed));
    }

    // "@%s" 是 "自纪元1970-01-01 00:00:00 +0000 (UTC)以来的秒数。 (TZ) (由mktime(tm)计算。)"
    if s.bytes().next() == Some(b'@') {
        if let Ok(ts) = &s[1..].parse::<i64>() {
            return Ok(FileTime::from_unix_time(*ts, 0));
        }
    }

    if let Ok(dt) = parse_datetime::parse_datetime_at_date(ref_time, s) {
        return Ok(touch_datetime_to_filetime(&dt));
    }

    Err(CtSimpleError::new(1, format!("Unable to parse date: {s}")))
}

fn parse_timestamp(s: &str) -> CTResult<FileTime> {
    use touch_format::*;

    let current_year = || Local::now().year();

    let (format, ts) = match s.chars().count() {
        15 => (YYYYMMDDHHMM_DOT_SS, s.to_owned()),
        12 => (YYYYMMDDHHMM, s.to_owned()),
        // 如果我们不添加"20"，我们就没有足够的信息来解析
        13 => (YYYYMMDDHHMM_DOT_SS, format!("20{}", s)),
        10 => (YYYYMMDDHHMM, format!("20{}", s)),
        11 => (YYYYMMDDHHMM_DOT_SS, format!("{}{}", current_year(), s)),
        8 => (YYYYMMDDHHMM, format!("{}{}", current_year(), s)),
        _ => {
            return Err(CtSimpleError::new(
                1,
                format!("invalid date ct_format {}", s.quote()),
            ))
        }
    };

    let local = NaiveDateTime::parse_from_str(&ts, format)
        .map_err(|_| CtSimpleError::new(1, format!("invalid date ts ct_format {}", ts.quote())))?;
    let mut local = match chrono::Local.from_local_datetime(&local) {
        LocalResult::Single(dt) => dt,
        _ => {
            return Err(CtSimpleError::new(
                1,
                format!("invalid date ts ct_format {}", ts.quote()),
            ))
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
            format!("invalid date ct_format {}", s.quote()),
        ));
    }

    Ok(touch_datetime_to_filetime(&local))
}

// TODO: 这可能是放入ct_fsext的好候选项
/// 返回指向标准输出的PathBuf。
///
/// 在Windows上，使用GetFinalPathNameByHandleW尝试从stdout句柄获取路径。
fn touch_pathbuf_from_stdout() -> CTResult<PathBuf> {
    #[cfg(all(unix, not(target_os = "android")))]
    {
        Ok(PathBuf::from("/dev/stdout"))
    }
    #[cfg(target_os = "android")]
    {
        Ok(PathBuf::from("/proc/self/fd/1"))
    }
    #[cfg(windows)]
    {
        use std::os::windows::prelude::AsRawHandle;
        use windows_sys::Win32::Foundation::{
            GetLastError, ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_PATH_NOT_FOUND,
            HANDLE, MAX_PATH,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            GetFinalPathNameByHandleW, FILE_NAME_OPENED,
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
                ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod parse_timestamp_tests {
        use chrono::{Datelike, Local, TimeZone};

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
            assert!(result.is_err());

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
            assert_eq!(filetime.unix_seconds(), -28800);
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
                "invalid date ct_format '202406150830.123456789'".to_string()
            );

            // 测试带有无效的纳秒部分的时间戳
            let timestamp_str = "202406150830.1234567890"; // 超过纳秒位数
            let result = parse_timestamp(timestamp_str);
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod parse_date_tests {
        use chrono::{Local, TimeZone};

        use super::*;

        #[test]
        fn test_parse_date_valid() {
            // 测试POSIX_LOCALE格式的日期
            let ref_time = Local.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();
            let date_str = "Tue Jun 15 08:30:00 2022";
            let filetime = touch_parse_date(ref_time, date_str);
            assert!(filetime.is_err());
            assert_eq!(
                filetime.unwrap_err().to_string(),
                "Unable to parse date: Tue Jun 15 08:30:00 2022".to_string()
            );

            // 测试ISO 8601格式的日期
            let date_str = "2022-06-15";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            // 加时区
            let expected_time = Local
                .with_ymd_and_hms(2022, 6, 15, 0, 0, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试其他有效格式的日期
            let date_str = "2022-06-15 08:30:00";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            // 加时区
            let expected_time = Local
                .with_ymd_and_hms(2022, 6, 15, 16, 30, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            let date_str = "2022-06-15 08:30:00.123456";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            // 加时区
            let expected_time = Local
                .with_ymd_and_hms(2022, 6, 15, 16, 30, 0)
                .unwrap()
                .with_nanosecond(123456000)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 123456);

            let date_str = "2022-06-15 08:30";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            // 加时区
            let expected_time = Local
                .with_ymd_and_hms(2022, 6, 15, 16, 30, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            let date_str = "202206150830";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            let expected_time = Local
                .with_ymd_and_hms(2022, 6, 15, 8, 30, 0)
                .unwrap()
                .timestamp();
            assert_eq!(filetime.unix_seconds(), expected_time);
            assert_eq!(filetime.nanoseconds(), 0);

            let date_str = "@1623742200";
            let filetime = touch_parse_date(ref_time, date_str).unwrap();
            assert_eq!(filetime.unix_seconds(), 1623742200);
            assert_eq!(filetime.nanoseconds(), 0);
        }

        #[test]
        fn test_parse_date_invalid() {
            let ref_time = Local.with_ymd_and_hms(2022, 6, 15, 8, 30, 0).unwrap();

            // 测试无效格式的日期
            let date_str = "invalid date string";
            let result = touch_parse_date(ref_time, date_str);
            assert!(result.is_err());

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
        use std::fs::{create_dir, File};
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
            assert_eq!(dt.hour(), 15);
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
            assert_eq!(dt.hour(), 15);
            assert_eq!(dt.minute(), 30);
            assert_eq!(dt.second(), 0);
            assert_eq!(dt.nanosecond(), 123456789);

            // 测试Unix Epoch (1970-01-01 08:00:00)
            let filetime = FileTime::from_unix_time(0, 0);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的DateTime是否与预期值匹配
            assert_eq!(dt.year(), 1970);
            assert_eq!(dt.month(), 1);
            assert_eq!(dt.day(), 1);
            assert_eq!(dt.hour(), 8);
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
            assert_eq!(dt.hour(), 21);
            assert_eq!(dt.minute(), 50);
            assert_eq!(dt.second(), 15);
            assert_eq!(dt.nanosecond(), 0);

            // 测试2025年1月1日 7:59:59.999999999
            let filetime = FileTime::from_unix_time(1735689599, 999999999);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的DateTime是否与预期值匹配
            assert_eq!(dt.year(), 2025);
            assert_eq!(dt.month(), 1);
            assert_eq!(dt.day(), 1);
            assert_eq!(dt.hour(), 7);
            assert_eq!(dt.minute(), 59);
            assert_eq!(dt.second(), 59);
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

            assert_eq!(filetime.unix_seconds(), 1704081600);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试时间为2024-05-11 23:59:59.123456789
            let dt = Local
                .with_ymd_and_hms(2024, 5, 11, 23, 59, 59)
                .unwrap()
                .with_nanosecond(123456789)
                .unwrap();
            let filetime = touch_datetime_to_filetime(&dt);

            assert_eq!(filetime.unix_seconds(), 1715443199);
            assert_eq!(filetime.nanoseconds(), 123456789);

            // 测试时间为1970-01-01 00:00:00 (Unix Epoch)
            let dt = Local.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt);

            assert_eq!(filetime.unix_seconds(), -28800);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试时间为2000-02-29 06:30:15 (闰年日期)
            let dt = Local.with_ymd_and_hms(2000, 2, 29, 6, 30, 15).unwrap();
            let filetime = touch_datetime_to_filetime(&dt);

            assert_eq!(filetime.unix_seconds(), 951777015);
            assert_eq!(filetime.nanoseconds(), 0);

            // 测试时间为2024-12-31 23:59:59.999999999
            let dt = Local
                .with_ymd_and_hms(2024, 12, 31, 23, 59, 59)
                .unwrap()
                .with_nanosecond(999999999)
                .unwrap();
            let filetime = touch_datetime_to_filetime(&dt);

            assert_eq!(filetime.unix_seconds(), 1735660799);
            assert_eq!(filetime.nanoseconds(), 999999999);
        }

        #[test]
        fn test_filetime_to_datetime() {
            // 测试Unix时间戳1704100800 (2024-01-01 12:00:00)
            let filetime = FileTime::from_unix_time(1704100800, 0);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            assert_eq!(dt.year(), 2024);
            assert_eq!(dt.month(), 1);
            assert_eq!(dt.day(), 1);
            assert_eq!(dt.hour(), 17);
            assert_eq!(dt.minute(), 20);
            assert_eq!(dt.second(), 0);
            assert_eq!(dt.nanosecond(), 0);

            // 测试Unix时间戳1715817599和纳秒数123456789 (2024-05-11 23:59:59.123456789)
            let filetime = FileTime::from_unix_time(1715817599, 123456789);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            assert_eq!(dt.year(), 2024);
            assert_eq!(dt.month(), 5);
            assert_eq!(dt.day(), 16);
            assert_eq!(dt.hour(), 7);
            assert_eq!(dt.minute(), 59);
            assert_eq!(dt.second(), 59);
            assert_eq!(dt.nanosecond(), 123456789);

            // 测试Unix时间戳0 (1970-01-01 08:00:00)
            let filetime = FileTime::from_unix_time(0, 0);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            assert_eq!(dt.year(), 1970);
            assert_eq!(dt.month(), 1);
            assert_eq!(dt.day(), 1);
            assert_eq!(dt.hour(), 8);
            assert_eq!(dt.minute(), 0);
            assert_eq!(dt.second(), 0);
            assert_eq!(dt.nanosecond(), 0);

            // 测试Unix时间戳951832215 (2000-02-29 21:50:15)
            let filetime = FileTime::from_unix_time(951832215, 0);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            assert_eq!(dt.year(), 2000);
            assert_eq!(dt.month(), 2);
            assert_eq!(dt.day(), 29);
            assert_eq!(dt.hour(), 21);
            assert_eq!(dt.minute(), 50);
            assert_eq!(dt.second(), 15);
            assert_eq!(dt.nanosecond(), 0);

            // 测试Unix时间戳1735689599和纳秒数999999999 (2025-1-1 7:59:59.999999999)
            let filetime = FileTime::from_unix_time(1735689599, 999999999);
            let dt = touch_filetime_to_datetime(&filetime).unwrap();

            assert_eq!(dt.year(), 2025);
            assert_eq!(dt.month(), 1);
            assert_eq!(dt.day(), 1);
            assert_eq!(dt.hour(), 7);
            assert_eq!(dt.minute(), 59);
            assert_eq!(dt.second(), 59);
            assert_eq!(dt.nanosecond(), 999999999);
        }

        #[test]
        fn test_timezone_conversion() {
            // 测试不同的时区
            let ny_tz = FixedOffset::west_opt(5 * 3600).unwrap(); // 纽约时区 (UTC-5)
            let dt_ny = ny_tz.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt_ny);
            let dt_converted = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的时间
            assert_eq!(dt_converted.timestamp(), dt_ny.timestamp());

            // 测试UTC时间
            let dt_utc = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt_utc);
            let dt_converted = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的时间
            assert_eq!(dt_converted.timestamp(), dt_utc.timestamp());

            // 测试亚洲东京时区 (UTC+9)
            let tokyo_tz = FixedOffset::east_opt(9 * 3600).unwrap();
            let dt_tokyo = tokyo_tz.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
            let filetime = touch_datetime_to_filetime(&dt_tokyo);
            let dt_converted = touch_filetime_to_datetime(&filetime).unwrap();

            // 验证转换后的时间
            assert_eq!(dt_converted.timestamp(), dt_tokyo.timestamp());
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_touch_main_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = touch_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_touch_main_access_time_short() {
            let filename = "test_touch_main_access_time_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-a", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_instead_of_the_current_time_short() {
            let filename = "test_touch_main_instead_of_the_current_time_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-t", "12011233", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_parse_the_current_time_long() {
            let filename = "test_touch_main_parse_the_current_time_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--date", "@2147483647", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_parse_the_current_time_short() {
            let filename = "test_touch_main_parse_the_current_time_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-d", "@2147483647", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_change_modification_time_short() {
            let filename = "test_touch_main_change_modification_time_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-m", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_no_create_long() {
            let filename = "test_touch_main_no_create_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--no-create", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_no_create_short() {
            let filename = "test_touch_main_no_create_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-c", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_no_dereference_short() {
            let filename = "test_touch_main_no_dereference_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-h", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_no_dereference_long() {
            let filename = "test_touch_main_no_dereference_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--no-dereference", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![
                ctcore::ct_util_name(),
                "--reference",
                reference_file_name,
                "--no-dereference",
                file_name,
            ];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                reference_file_name,
                "--no-dereference",
                file_name,
            ];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_access() {
            let filename = "test_touch_main_time_long_access";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--time", "access", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_atime() {
            let filename = "test_touch_main_time_long_atime";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--time", "atime", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_use() {
            let filename = "test_touch_main_time_long_use";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--time", "use", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_modify() {
            let filename = "test_touch_main_time_long_modify";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--time", "modify", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_touch_main_time_long_mtime() {
            let filename = "test_touch_main_time_long_mtime";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--time", "mtime", file_name];
            let result = touch_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }
    }
}