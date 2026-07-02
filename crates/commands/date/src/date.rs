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
use chrono::format::StrftimeItems;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use chrono::{DateTime, Datelike, FixedOffset, Local, Offset, TimeDelta, Timelike, Utc};
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::FromIo;
use ctcore::ct_error::{CTResult, CtSimpleError};
use ctcore::ct_show;
use sys_locale::get_locale;

#[cfg(target_os = "linux")]
use libc::{
    CLOCK_REALTIME, D_T_FMT, LC_ALL, c_char, clock_getres, clock_settime, gmtime_r, localtime_r,
    nl_langinfo, setlocale, strftime, timespec, tm,
};
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
#[cfg(windows)]
use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::SetSystemTime};

use ctcore::Tool;
use ctcore::ct_shortcut_value_parser::CtShortcutValueParser;
#[cfg(target_os = "linux")]
use std::ffi::CStr;
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::mem;

// Options
const DATE: &str = "date";
const HOURS: &str = "hours";
const MINUTES: &str = "minutes";
const SECONDS: &str = "seconds";
const NS: &str = "ns";

const DATE_OPT_DATE: &str = "date";
const DATE_OPT_FORMAT: &str = "format";
const DATE_OPT_FILE: &str = "file";
const DATE_OPT_DEBUG: &str = "debug";
const DATE_OPT_ISO_8601: &str = "iso-8601";
const DATE_OPT_RFC_EMAIL: &str = "rfc-email";
const DATE_OPT_RFC_3339: &str = "rfc-3339";
const DATE_OPT_SET: &str = "set";
const DATE_OPT_REFERENCE: &str = "reference";
const DATE_OPT_UNIVERSAL: &str = "universal";
const DATE_OPT_UNIVERSAL_2: &str = "utc";
const DATE_OPT_RESOLUTION: &str = "resolution";

// 帮助字符串

static DATE_ISO_8601_HELP_STRING: &str = "output date/time in ISO 8601 format.
 FMT='date' for date only (the default),
 'hours', 'minutes', 'seconds', or 'ns'
 for date and time to the indicated precision.
 Example: 2006-08-14T02:34:56-06:00";

static DATE_RFC_5322_HELP_STRING: &str = "output date and time in RFC 5322 format.
 Example: Mon, 14 Aug 2006 02:34:56 -0600";

static DATE_RFC_3339_HELP_STRING: &str = "output date/time in RFC 3339 format.
 FMT='date', 'seconds', or 'ns'
 for date and time to the indicated precision.
 Example: 2006-08-14 02:34:56-06:00";

#[cfg(target_os = "linux")]
static DATE_OPT_SET_HELP_STRING: &str = "set time described by STRING";

/// Settings for this program, parsed from the command line
struct DateSettings {
    utc: bool,
    format: DateFormat,
    date_source: DateSource,
    set_to: Option<DateTime<FixedOffset>>,
}

/// Various ways of displaying the date
enum DateFormat {
    Iso8601(DateIso8601Format),
    Rfc5322,
    Rfc3339(DateRfc3339Format),
    Custom(String),
    Default,
}

/// Various places that dates can come from
enum DateSource {
    Now,
    Custom(String),
    File(PathBuf),
    Human(TimeDelta),
    Resolution,
    Reference(PathBuf),
}

enum DateIso8601Format {
    Date,
    Hours,
    Minutes,
    Seconds,
    Ns,
}

impl From<&str> for DateIso8601Format {
    fn from(s: &str) -> Self {
        match s {
            HOURS => Self::Hours,
            MINUTES => Self::Minutes,
            SECONDS => Self::Seconds,
            NS => Self::Ns,
            DATE => Self::Date,
            // 注意：此情况已通过 clap 的 `possible_values` 进行捕获
            _ => unreachable!(),
        }
    }
}

enum DateRfc3339Format {
    Date,
    Seconds,
    Ns,
}
// 实现是 Rust 中的 From 泛型 trait，它允许你将一个类型转换为另一个类型。在这里，我们定义了如何从字符串引用 &'a str 转换为 DateRfc3339Format。
// impl<'a> 表示这个实现适用于所有生命周期 'a 的字符串引用。
// From<&'a str> for DateRfc3339Format 表示我们正在实现从 &'a str 转换到 DateRfc3339Format 的功能。
// fn from(s: &str) -> Self 是 From trait 中的 from 方法，它接受一个字符串引用 s，并返回 Self，即 DateRfc3339Format 枚举。
impl From<&str> for DateRfc3339Format {
    fn from(s: &str) -> Self {
        match s {
            DATE => Self::Date,
            SECONDS => Self::Seconds,
            NS => Self::Ns,
            // 应该被clap捕获
            _ => panic!("Invalid format: {s}"),
        }
    }
}

#[derive(Default)]
pub struct Date;
impl Tool for Date {
    fn name(&self) -> &'static str {
        "date"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        date_main(args.iter().cloned()).map(|_| ())
    }
}

/**
 * 主函数，用于处理命令行参数并设置或显示系统日期和时间。
 *
 * @param args 命令行参数，实现了 `ctcore::Args` 接口。
 * @return `CTResult<()>`，成功时返回 `Ok(())`，错误时返回包含错误信息的 `Err`。
 */
pub fn date_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    #[cfg(target_os = "linux")]
    unsafe {
        setlocale(LC_ALL, b"\0".as_ptr() as *const c_char);
    }
    // 从命令行参数中解析匹配项
    let args_match = ct_app().try_get_matches_from(args)?;

    // 如果指定了 -u/--utc/--universal，设置 TZ 环境变量为 UTC0
    if args_match.get_flag(DATE_OPT_UNIVERSAL) {
        unsafe {
            std::env::set_var("TZ", "UTC0");
        }
        #[cfg(target_os = "linux")]
        unsafe {
            unsafe extern "C" {
                fn tzset();
            }
            tzset();
        }
    }

    // 根据命令行参数确定日期格式
    let date_format = match get_date_format(&args_match) {
        Ok(value) => value,
        Err(value) => return value,
    };

    let date_source = get_date_source(&args_match);

    let set_to_params = match set_date_params(&args_match) {
        Ok(value) => value,
        Err(value) => return value,
    };

    date_processing(args_match, date_format, date_source, set_to_params)
}

fn date_processing(
    args_match: ArgMatches,
    date_format: DateFormat,
    date_source: DateSource,
    set_to_params: Option<DateTime<FixedOffset>>,
) -> CTResult<()> {
    // 创建日期设置结构体
    let date_set = DateSettings {
        utc: args_match.get_flag(DATE_OPT_UNIVERSAL),
        format: date_format,
        date_source,
        set_to: set_to_params,
    };

    #[cfg(target_os = "linux")]
    if matches!(date_set.format, DateFormat::Rfc5322) {
        unsafe {
            libc::setlocale(libc::LC_TIME, b"C\0".as_ptr() as *const libc::c_char);
        }
    }

    // 根据日期设置来设置系统日期时间或者显示当前日期时间
    if let Some(date) = date_set.set_to {
        // 如果需要设置时间，首先确保是UTC格式
        let date: DateTime<Utc> = if date_set.utc {
            date.with_timezone(&Utc)
        } else {
            date.into()
        };

        set_system_datetime(date)
    } else {
        // 获取当前时间，根据设置确定是否使用UTC
        let now: DateTime<FixedOffset> = if date_set.utc {
            let now = Utc::now();
            now.with_timezone(&now.offset().fix())
        } else {
            let now = Local::now();
            now.with_timezone(now.offset())
        };

        // 根据日期来源生成日期的迭代器
        // 创建一个动态分发的迭代器Box<dyn Iterator<Item = _>>，用于根据不同的DateSource枚举值生成对应的日期迭代
        let dates_iterator: Box<dyn Iterator<Item = _>> = match date_set.date_source {
            DateSource::Custom(ref input) => {
                let mut date = parse_date(input.clone());
                if let Ok(dt) = date {
                    if date_set.utc {
                        date = Ok(dt.with_timezone(&Utc).into());
                    }
                }
                let iter = std::iter::once(date);
                Box::new(iter)
            }
            DateSource::Human(relative_time) => {
                let current_time = DateTime::<FixedOffset>::from(Local::now());
                match current_time.checked_add_signed(relative_time) {
                    Some(date) => {
                        let iter = std::iter::once(Ok(date));
                        Box::new(iter)
                    }
                    None => {
                        return Err(CtSimpleError::new(
                            1,
                            format!("invalid date {relative_time}"),
                        ));
                    }
                }
            }
            DateSource::File(ref path) => {
                if path.is_dir() {
                    return Err(CtSimpleError::new(
                        2,
                        format!("expected file, got directory {}", path.quote()),
                    ));
                }
                let file = File::open(path)
                    .map_err_context(|| path.as_os_str().to_string_lossy().to_string())?;
                let lines = BufReader::new(file).lines();
                let mut iter: Box<dyn Iterator<Item = _>> =
                    Box::new(lines.map_while(Result::ok).map(parse_date));
                if date_set.utc {
                    iter = Box::new(iter.map(|res| res.map(|dt| dt.with_timezone(&Utc).into())));
                }
                iter
            }
            DateSource::Now => {
                let iter = std::iter::once(Ok(now));
                Box::new(iter)
            }
            DateSource::Resolution => {
                let (sec, nsec) = get_clock_resolution();
                let dt = DateTime::from_timestamp(sec, nsec as u32).unwrap();
                let dt: DateTime<FixedOffset> = if date_set.utc {
                    dt.with_timezone(&Utc).into()
                } else {
                    dt.with_timezone(&Local).into()
                };
                let iter = std::iter::once(Ok(dt));
                Box::new(iter)
            }
            DateSource::Reference(ref path) => {
                let metadata = std::fs::metadata(path)
                    .map_err(|e| CtSimpleError::new(1, format!("{}: {}", path.quote(), e)))?;
                let time = metadata
                    .modified()
                    .map_err(|e| CtSimpleError::new(1, format!("{}: {}", path.quote(), e)))?;
                let dt: DateTime<FixedOffset> = if date_set.utc {
                    let dt: DateTime<Utc> = time.into();
                    dt.with_timezone(&dt.offset().fix())
                } else {
                    let dt: DateTime<Local> = time.into();
                    dt.with_timezone(dt.offset())
                };
                let iter = std::iter::once(Ok(dt));
                Box::new(iter)
            }
        };

        // 根据日期设置生成格式化字符串
        let format_string = make_format_string(&date_set);

        // 格式化并打印所有日期
        for date in dates_iterator {
            match date {
                Ok(date) => {
                    #[cfg(target_os = "linux")]
                    {
                        let s = format_using_strftime(&date, &format_string)?;
                        println!("{}", s);
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        // 临时替换格式字符串中的 `%N` 为 `%f`，以兼容处理
                        let format_string = &format_string.replace("%N", "%f");
                        // 检查格式字符串是否包含无效的格式项
                        if format_string.contains("%#z") {
                            return Err(CtSimpleError::new(
                                1,
                                format!("invalid format {}", format_string.replace("%f", "%N")),
                            ));
                        }
                        // 格式化日期并打印
                        let formatted = date
                            .format_with_items(StrftimeItems::new(format_string))
                            .to_string()
                            .replace("%f", "%N");
                        println!("{formatted}");
                    }
                }
                Err((input, _err)) => ct_show!(CtSimpleError::new(
                    1,
                    format!("invalid date {}", input.quote())
                )),
            }
        }
        Ok(())
    }
}

fn set_date_params(args_match: &ArgMatches) -> Result<Option<DateTime<FixedOffset>>, CTResult<()>> {
    // 解析并验证设置日期的参数
    let set_to_params = match args_match.get_one::<String>(DATE_OPT_SET).map(parse_date) {
        None => None,
        Some(Err((input, _err))) => {
            return Err(Err(CtSimpleError::new(
                1,
                format!("invalid date {}", input.quote()),
            )));
        }
        Some(Ok(date)) => Some(date),
    };
    Ok(set_to_params)
}

fn get_date_source(args_match: &ArgMatches) -> DateSource {
    // 根据命令行参数确定日期来源
    if args_match.get_flag(DATE_OPT_RESOLUTION) {
        DateSource::Resolution
    } else if let Some(date) = args_match.get_one::<String>(DATE_OPT_DATE) {
        DateSource::Custom(date.into())
    } else if let Some(file) = args_match.get_one::<String>(DATE_OPT_FILE) {
        DateSource::File(file.into())
    } else if let Some(file) = args_match.get_one::<String>(DATE_OPT_REFERENCE) {
        DateSource::Reference(file.into())
    } else {
        DateSource::Now
    }
}

fn get_date_format(args_match: &ArgMatches) -> Result<DateFormat, CTResult<()>> {
    // 根据命令行参数确定日期格式
    let date_format = if let Some(form) = args_match.get_one::<String>(DATE_OPT_FORMAT) {
        if !form.starts_with('+') {
            return Err(Err(CtSimpleError::new(
                1,
                format!("invalid date {}", form.quote()),
            )));
        }
        let form = form[1..].to_string();
        DateFormat::Custom(form)
    } else if let Some(fmt) = args_match
        .get_many::<String>(DATE_OPT_ISO_8601)
        .map(|mut iter| iter.next().unwrap_or(&DATE.to_string()).as_str().into())
    {
        DateFormat::Iso8601(fmt)
    } else if args_match.get_flag(DATE_OPT_RFC_EMAIL) {
        DateFormat::Rfc5322
    } else if let Some(fmt) = args_match
        .get_one::<String>(DATE_OPT_RFC_3339)
        .map(|s| s.as_str().into())
    {
        DateFormat::Rfc3339(fmt)
    } else if args_match.get_flag(DATE_OPT_RESOLUTION) {
        DateFormat::Custom("%s.%N".to_string())
    } else {
        DateFormat::Default
    };
    Ok(date_format)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("date.about");
    let usage_description = t!("date.usage");

    let args = date_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

fn date_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new("help")
            .short('h')
            .long("help")
            .help(t!("date.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("date.clap.version"))
            .action(ArgAction::Version),
        Arg::new(DATE_OPT_DATE)
            .short('d')
            .long(DATE_OPT_DATE)
            .value_name("STRING")
            .help(t!("date.clap.date_opt_date")),
        Arg::new(DATE_OPT_FILE)
            .short('f')
            .long(DATE_OPT_FILE)
            .value_name("DATEFILE")
            .value_hint(clap::ValueHint::FilePath)
            .help(t!("date.clap.date_opt_file")),
        Arg::new(DATE_OPT_ISO_8601)
            .short('I')
            .long(DATE_OPT_ISO_8601)
            .value_name("FMT")
            .value_parser(CtShortcutValueParser::new([
                DATE, HOURS, MINUTES, SECONDS, NS,
            ]))
            .num_args(0..=1)
            .default_missing_value(DATE_OPT_DATE)
            .help(DATE_ISO_8601_HELP_STRING),
        Arg::new(DATE_OPT_RFC_EMAIL)
            .short('R')
            .long(DATE_OPT_RFC_EMAIL)
            .help(DATE_RFC_5322_HELP_STRING)
            .action(ArgAction::SetTrue),
        Arg::new(DATE_OPT_RFC_3339)
            .long(DATE_OPT_RFC_3339)
            .value_name("FMT")
            .value_parser(CtShortcutValueParser::new([DATE, SECONDS, NS]))
            .help(DATE_RFC_3339_HELP_STRING),
        Arg::new(DATE_OPT_DEBUG)
            .long(DATE_OPT_DEBUG)
            .help(t!("date.clap.date_opt_debug"))
            .action(ArgAction::SetTrue),
        Arg::new(DATE_OPT_REFERENCE)
            .short('r')
            .long(DATE_OPT_REFERENCE)
            .value_name("FILE")
            .value_hint(clap::ValueHint::AnyPath)
            .help(t!("date.clap.date_opt_reference")),
        Arg::new(DATE_OPT_SET)
            .short('s')
            .long(DATE_OPT_SET)
            .value_name("STRING")
            .help(DATE_OPT_SET_HELP_STRING),
        Arg::new(DATE_OPT_UNIVERSAL)
            .short('u')
            .long(DATE_OPT_UNIVERSAL)
            .alias(DATE_OPT_UNIVERSAL_2)
            .help(t!("date.clap.date_opt_universal"))
            .action(ArgAction::SetTrue),
        Arg::new(DATE_OPT_RESOLUTION)
            .long(DATE_OPT_RESOLUTION)
            .help("output the available resolution of timestamps")
            .action(ArgAction::SetTrue)
            .conflicts_with_all([DATE_OPT_DATE, DATE_OPT_FILE, DATE_OPT_REFERENCE]),
        Arg::new(DATE_OPT_FORMAT),
    ];
    args
}

/// Return the appropriate format string for the given settings.
fn make_format_string(date_settings: &DateSettings) -> String {
    match date_settings.format {
        DateFormat::Iso8601(ref fmt) => match *fmt {
            DateIso8601Format::Date => "%F".to_string(),
            DateIso8601Format::Hours => "%FT%H%:z".to_string(),
            DateIso8601Format::Minutes => "%FT%H:%M%:z".to_string(),
            DateIso8601Format::Seconds => "%FT%T%:z".to_string(),
            _ => "%FT%T,%f%:z".to_string(),
        },
        DateFormat::Rfc5322 => "%a, %d %b %Y %H:%M:%S %z".to_string(),
        DateFormat::Rfc3339(ref fmt) => match *fmt {
            DateRfc3339Format::Date => "%F".to_string(),
            DateRfc3339Format::Seconds => "%F %T%:z".to_string(),
            _ => "%F %T.%f%:z".to_string(),
        },
        DateFormat::Custom(ref fmt) => fmt.clone(),
        DateFormat::Default => get_default_format(),
    }
}

#[cfg(target_os = "linux")]
fn get_default_format() -> String {
    // Try to detect if we are in a Chinese locale to match GNU date's default format for zh_CN.
    // GNU date uses _DATE_FMT which is not easily accessible/stable via libc crate.
    // For zh_CN, _DATE_FMT is usually "%Y年 %m月 %d日 %A %H:%M:%S %Z"
    if let Ok(lang) = std::env::var("LC_TIME")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LANG"))
    {
        if lang.starts_with("zh_CN") {
            return "%Y年 %m月 %d日 %A %H:%M:%S %Z".to_string();
        }
    }

    unsafe {
        let ptr = nl_langinfo(D_T_FMT);
        if !ptr.is_null() {
            let c_str = CStr::from_ptr(ptr);
            if let Ok(s) = c_str.to_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    // Fallback if D_T_FMT is empty or invalid
    "%a %b %e %H:%M:%S %Z %Y".to_string()
}

#[cfg(not(target_os = "linux"))]
fn get_default_format() -> String {
    "%c".to_string()
}

#[cfg(target_os = "linux")]
fn format_using_strftime(dt: &DateTime<FixedOffset>, fmt: &str) -> CTResult<String> {
    // Handle %N by replacing it with nanoseconds
    // Also handle %f which might be used internally in make_format_string
    let nanos = format!("{:09}", dt.nanosecond());
    let quarter = (dt.month0() / 3) + 1;

    let offset_secs = dt.offset().local_minus_utc();
    let abs_secs = offset_secs.abs();
    let hours = abs_secs / 3600;
    let minutes = (abs_secs % 3600) / 60;
    let seconds = abs_secs % 60;
    let sign = if offset_secs < 0 { "-" } else { "+" };

    let tz_colon = format!("{}{:02}:{:02}", sign, hours, minutes);
    let tz_double_colon = format!("{}{:02}:{:02}:{:02}", sign, hours, minutes, seconds);
    let tz_triple_colon = if seconds == 0 && minutes == 0 {
        format!("{}{:02}", sign, hours)
    } else if seconds == 0 {
        format!("{}{:02}:{:02}", sign, hours, minutes)
    } else {
        format!("{}{:02}:{:02}:{:02}", sign, hours, minutes, seconds)
    };

    // Pre-process format string to handle %-WIDTH conversion (remove width)
    // This is to match GNU date behavior where - flag disables padding, effectively ignoring width.
    let mut fmt_adjusted = String::with_capacity(fmt.len());
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        fmt_adjusted.push(c);
        if c == '%' {
            if let Some(&next) = chars.peek() {
                if next == '-' {
                    chars.next(); // consume '-'
                    fmt_adjusted.push('-');
                    // Skip digits following '-'
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                } else if next == '%' {
                    chars.next();
                    fmt_adjusted.push('%');
                }
            }
        }
    }

    let fmt_final = fmt_adjusted
        .replace("%N", &nanos)
        .replace("%f", &nanos)
        .replace("%q", &quarter.to_string())
        .replace("%:::z", &tz_triple_colon)
        .replace("%::z", &tz_double_colon)
        .replace("%:z", &tz_colon);

    let c_fmt =
        CString::new(fmt_final).map_err(|_| CtSimpleError::new(1, "Invalid format string"))?;

    let ts = dt.timestamp();
    let mut tm_val: tm = unsafe { mem::zeroed() };
    let mut use_tm = false;

    // Try localtime if offset matches
    unsafe {
        let mut tmp_tm: tm = mem::zeroed();
        if localtime_r(&ts, &mut tmp_tm) != std::ptr::null_mut() {
            if tmp_tm.tm_gmtoff as i32 == dt.offset().local_minus_utc() {
                tm_val = tmp_tm;
                use_tm = true;
            }
        }
    }

    if !use_tm && dt.offset().local_minus_utc() == 0 {
        unsafe {
            if gmtime_r(&ts, &mut tm_val) != std::ptr::null_mut() {
                use_tm = true;
            }
        }
    }

    if !use_tm {
        tm_val.tm_sec = dt.second() as i32;
        tm_val.tm_min = dt.minute() as i32;
        tm_val.tm_hour = dt.hour() as i32;
        tm_val.tm_mday = dt.day() as i32;
        tm_val.tm_mon = dt.month0() as i32;
        tm_val.tm_year = dt.year() as i32 - 1900;
        tm_val.tm_wday = dt.weekday().num_days_from_sunday() as i32;
        tm_val.tm_yday = dt.ordinal0() as i32;
        tm_val.tm_isdst = -1;
        tm_val.tm_gmtoff = dt.offset().local_minus_utc() as i64;
    }

    let mut buf = vec![0u8; 256];
    loop {
        let res = unsafe {
            strftime(
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
                c_fmt.as_ptr(),
                &tm_val,
            )
        };
        if res > 0 {
            let s = String::from_utf8_lossy(&buf[..res]);
            return Ok(s.to_string());
        }
        if buf.len() > 65536 {
            if c_fmt.as_bytes().is_empty() {
                return Ok(String::new());
            }
            return Err(CtSimpleError::new(1, "strftime failed"));
        }
        buf.resize(buf.len() * 2, 0);
    }
}

fn parse_date<S: AsRef<str> + Clone>(
    s: S,
) -> Result<DateTime<FixedOffset>, (String, chrono::format::ParseError)> {
    // TODO: The GNU date command can parse a wide variety of inputs.

    let input = s.as_ref();
    let ref_time = Local::now().with_nanosecond(0).unwrap();
    if let Ok(dt) = ctcore::ct_parse_datetime::parse_datetime_gnu_compat(input, ref_time) {
        return Ok(dt.into());
    }
    match input.parse() {
        Ok(date) => Ok(date),
        Err(e) => Err((input.into(), e)),
    }
}

#[cfg(target_os = "linux")]
fn get_clock_resolution() -> (i64, i64) {
    let mut ts = timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { clock_getres(CLOCK_REALTIME, &mut ts) } == 0 {
        (ts.tv_sec as i64, ts.tv_nsec as i64)
    } else {
        (0, 1)
    }
}

#[cfg(not(target_os = "linux"))]
fn get_clock_resolution() -> (i64, i64) {
    (0, 1)
}

#[cfg(target_os = "linux")]
/// System call to set date (unix).
/// See here for more:
/// `<https://doc.rust-lang.org/libc/i686-unknown-linux-gnu/libc/fn.clock_settime.html>`
/// `<https://linux.die.net/man/3/clock_settime>`
/// `<https://www.gnu.org/software/libc/manual/html_node/Time-Types.html>`
fn set_system_datetime(date: DateTime<Utc>) -> CTResult<()> {
    let timespec = timespec {
        tv_sec: date.timestamp() as _,
        tv_nsec: date.timestamp_subsec_nanos() as _,
    };

    let result = unsafe { clock_settime(CLOCK_REALTIME, &timespec) };

    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().map_err_context(|| "cannot set date".to_string()))
    }
}

#[cfg(windows)]
/// System call to set date (Windows).
/// See here for more:
/// https://docs.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-setsystemtime
/// https://docs.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-systemtime
fn set_system_datetime(date: DateTime<Utc>) -> CTResult<()> {
    let system_time = SYSTEMTIME {
        wYear: date.year() as u16,
        wMonth: date.month() as u16,
        // Ignored
        wDayOfWeek: 0,
        wDay: date.day() as u16,
        wHour: date.hour() as u16,
        wMinute: date.minute() as u16,
        wSecond: date.second() as u16,
        // TODO: be careful of leap seconds - valid range is [0, 999] - how to handle?
        wMilliseconds: ((date.nanosecond() / 1_000_000) % 1000) as u16,
    };

    let result = unsafe { SetSystemTime(&system_time) };

    if result == 0 {
        Err(std::io::Error::last_os_error().map_err_context(|| "cannot set date".to_string()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Date;

        // 测试 name 方法
        assert_eq!(tool.name(), "date");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("date"));

        // 测试 execute 方法
        let args = vec![OsString::from("date")];
        assert!(tool.execute(&args).is_ok());
    }

    mod tests_ct_app {
        use crate::ct_app;
        use clap::error::ErrorKind;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;
        use tempfile::Builder;
        // 定义一个宏来生成测试用例
        macro_rules! test_date_format {
            ($name:ident, $format:expr) => {
                #[test]
                fn $name() {
                    let args = vec![
                        OsString::from(ctcore::ct_util_name()), // 假设这是获取程序名的方法
                        OsString::from(format!("+{}", $format)),
                    ];

                    assert!(ct_app().try_get_matches_from(args.into_iter()).is_ok());
                }
            };
        }

        // 使用宏生成测试用例
        // 定义所有格式化参数的测试用例
        test_date_format!(test_date_format_a, "%a"); // 本地化的缩写星期几名称
        test_date_format!(test_date_format_aa, "%A"); // 本地化的完整星期几名称
        test_date_format!(test_date_format_b, "%b"); // 本地化的缩写月份名称
        test_date_format!(test_date_format_bb, "%B"); // 本地化的完整月份名称
        test_date_format!(test_date_format_c, "%c"); // 本地化的日期和时间表示
        test_date_format!(test_date_format_cc, "%C"); // 世纪数（年份的前两位）
        test_date_format!(test_date_format_d, "%d"); // 月份中的日子（01-31）
        test_date_format!(test_date_format_dd, "%D"); // 日期，格式为%m/%d/%y
        test_date_format!(test_date_format_e, "%e"); // 月份中的日子，空格填充
        test_date_format!(test_date_format_ff, "%F"); // 完整日期，格式为%Y-%m-%d
        test_date_format!(test_date_format_g, "%g"); // ISO周号的年份的后两位数字
        test_date_format!(test_date_format_gg, "%G"); // ISO周号的年份
        test_date_format!(test_date_format_h, "%h"); // 与%b相同，本地化的缩写月份名称
        test_date_format!(test_date_format_hh, "%H"); // 小时数（00-23）
        test_date_format!(test_date_format_ii, "%I"); // 小时数（01-12）
        test_date_format!(test_date_format_j, "%j"); // 一年中的天数（001-366）
        test_date_format!(test_date_format_k, "%k"); // 小时数（0-23），空格填充
        test_date_format!(test_date_format_l, "%l"); // 小时数（1-12），空格填充
        test_date_format!(test_date_format_m, "%m"); // 月份（01-12）
        test_date_format!(test_date_format_mm, "%M"); // 分钟数（00-59）
        test_date_format!(test_date_format_n, "%n"); // 换行符
        test_date_format!(test_date_format_nn, "%N"); // 纳秒数（000000000-999999999）
        test_date_format!(test_date_format_p, "%p"); // 本地化的AM或PM
        test_date_format!(test_date_format_pp, "%P"); // 与%p相同，但为小写
        // test_date_format!(test_date_format_q, "%q"); // 季度号（1-4）  //TODO 与系统命令不一致，系统命令支持该参数
        test_date_format!(test_date_format_r, "%r"); // 本地化的12小时制时间
        test_date_format!(test_date_format_rr, "%R"); // 24小时制的时间，格式为%H:%M
        test_date_format!(test_date_format_s, "%s"); // 自1970-01-01 00:00:00 UTC以来的秒数
        test_date_format!(test_date_format_ss, "%S"); // 秒数（00-60）
        test_date_format!(test_date_format_t, "%t"); // 制表符
        test_date_format!(test_date_format_tt, "%T"); // 时间，格式为%H:%M:%S
        test_date_format!(test_date_format_u, "%u"); // 星期几的数字（1-7），1为星期一
        test_date_format!(test_date_format_uu, "%U"); // 一年中的周数，星期日为每周的开始
        test_date_format!(test_date_format_vv, "%V"); // ISO周数，星期一为每周的开始
        test_date_format!(test_date_format_w, "%w"); // 星期几的数字（0-6），0为星期日
        test_date_format!(test_date_format_ww, "%W"); // 一年中的周数，星期一为每周的开始
        test_date_format!(test_date_format_x, "%x"); // 本地化的日期表示
        test_date_format!(test_date_format_xx, "%X"); // 本地化的时间表示
        test_date_format!(test_date_format_y, "%y"); // 年份的后两位数字
        test_date_format!(test_date_format_yy, "%Y"); // 年份
        test_date_format!(test_date_format_z, "%z"); // 数字时区（+hhmm或-hhmm）
        test_date_format!(test_date_format_colon_z, "%:z"); // 数字时区，格式为±hh:mm
        test_date_format!(test_date_format_double_colon_z, "%::z"); // 数字时区，格式为±hh:mm:ss
        test_date_format!(test_date_format_triple_colon_z, "%:::z"); // 数字时区，以':'分隔至必要的精度
        test_date_format!(test_date_format_zz, "%Z"); // 字母时区缩写

        #[test]
        fn test_ct_app_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_v() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_date_yesterday() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "yesterday"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_today() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "today"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_tomorrow() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "tomorrow"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_month() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "month"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_week() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "week"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_friday() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "Friday"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_date() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "2024-05-01"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_string() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "May 1 2024"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_next_yesterday() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "next", "yesterday"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_next_today() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "next", "today"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_next_tomorrow() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "next", "tomorrow"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_next_month() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "next", "month"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_next_week() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "next", "week"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_next_friday() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "next", "Friday"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_next_date() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "next", "2024-05-01"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_date_next_string() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--date", "next", "May 1 2024"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_yesterday() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "yesterday"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_today() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "today"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_tomorrow() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "tomorrow"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_month() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "month"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_week() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "week"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_friday() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "Friday"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_date() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "2024-05-01"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_string() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "May 1 2024"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_next_yesterday() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "next", "yesterday"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_next_today() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "next", "today"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_next_tomorrow() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "next", "tomorrow"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_next_month() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "next", "month"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_next_week() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "next", "week"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_next_friday() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "next", "Friday"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_next_date() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "next", "2024-05-01"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_next_string() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "next", "May 1 2024"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", datefile];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_f() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-f", datefile];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_iso_8601_date() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--iso-8601=date"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_iso_8601_hours() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--iso-8601=hours"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_iso_8601_minutes() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--iso-8601=minutes"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_iso_8601_seconds() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--iso-8601=seconds"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_i_8601_date() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "Idate"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_i_8601_hours() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "Ihours"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_i_8601_minutes() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "Iminutes"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_i_8601_seconds() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "Iseconds"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_rfc_mail() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--rfc-email"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_r_debug() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-R", "--debug"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_debug() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--debug"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_rfc_3339_date() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--rfc-3339=date"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_rfc_3339_hours() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--rfc-3339=hours"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_rfc_3339_minutes() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--rfc-3339=minutes"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_rfc_3339_seconds() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--rfc-3339=seconds"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_rfc_3339_ns() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-R", "--rfc-3339=ns"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_r_rfc_3339_date() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-R", "--rfc-3339=date"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_r_rfc_3339_seconds() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-R", "--rfc-3339=seconds"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_r_rfc_3339_ns() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--rfc-3339=ns"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_r_rfc_mail() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-R", "--rfc-email"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_reference() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-r", datefile];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_reference_whole() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--reference", datefile];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-u"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_universal() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--universal"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_iso_8601_date() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--iso-8601=date",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_iso_8601_hours() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--iso-8601=hours",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_iso_8601_minutes() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--iso-8601=minutes",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_iso_8601_seconds() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--iso-8601=seconds",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_i8601_date() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", datefile, "-Idate"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_i8601_hours() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", datefile, "Ihours"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_i8601_minutes() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", datefile, "Iminutes"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_i8601_seconds() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", datefile, "Iseconds"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_rfc_mail() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", datefile, "--rfc-email"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_r_debug() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", datefile, "-R", "--debug"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_debug() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", datefile, "--debug"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_rfc_3339_date() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--rfc-3339=date",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_rfc_3339_hours() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--rfc-3339=hours",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_file_rfc_3339_minutes() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--rfc-3339=minutes",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_file_rfc_3339_seconds() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--rfc-3339=seconds",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_rfc_3339_ns() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "-R",
                "--rfc-3339=ns",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_r_file_rfc_3339_date() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "-R",
                "--rfc-3339=date",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_r_file_rfc_3339_seconds() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "-R",
                "--rfc-3339=seconds",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_r_file_rfc_3339_ns() {
            let temp_dir = Builder::new().prefix("test_ct_app_file").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_ct_app_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", datefile, "--rfc-3339=ns"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
    }

    mod tests_date_main {
        use crate::date_main;

        use std::ffi::OsString;

        use std::fs;
        use std::fs::File;
        use std::io::Write;
        use tempfile::Builder;

        #[test]
        fn test_date_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_date_yesterday() {
            let args = [ctcore::ct_util_name(), "--date", "yesterday"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_today() {
            let args = [ctcore::ct_util_name(), "--date", "today"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_tomorrow() {
            let args = [ctcore::ct_util_name(), "--date", "tomorrow"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_month() {
            let args = [ctcore::ct_util_name(), "--date", "month"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_week() {
            let args = [ctcore::ct_util_name(), "--date", "week"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_friday() {
            let args = [ctcore::ct_util_name(), "--date", "Friday"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_date() {
            let args = [ctcore::ct_util_name(), "--date", "2024-05-01"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_string() {
            let args = [ctcore::ct_util_name(), "--date", "May 1 2024"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_yesterday() {
            let args = [ctcore::ct_util_name(), "--date", "next yesterday"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_today() {
            let args = [ctcore::ct_util_name(), "--date", "next today"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_tomorrow() {
            let args = [ctcore::ct_util_name(), "--date", "next tomorrow"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_month() {
            let args = [ctcore::ct_util_name(), "--date", "next month"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_week() {
            let args = [ctcore::ct_util_name(), "--date", "next week"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_friday() {
            let args = [ctcore::ct_util_name(), "--date", "next Friday"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_date() {
            let args = [ctcore::ct_util_name(), "--date", "next 2024-05-01"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_string() {
            let args = [ctcore::ct_util_name(), "--date", "next May 1 2024"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_yesterday() {
            let args = [ctcore::ct_util_name(), "-d", "yesterday"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_today() {
            let args = [ctcore::ct_util_name(), "-d", "today"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_tomorrow() {
            let args = [ctcore::ct_util_name(), "-d", "tomorrow"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_month() {
            let args = [ctcore::ct_util_name(), "-d", "month"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_week() {
            let args = [ctcore::ct_util_name(), "-d", "week"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_friday() {
            let args = [ctcore::ct_util_name(), "-d", "Friday"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_date() {
            let args = [ctcore::ct_util_name(), "-d", "2024-05-01"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_string() {
            let args = [ctcore::ct_util_name(), "-d", "May 1 2024"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_yesterday() {
            let args = [ctcore::ct_util_name(), "-d", "next yesterday"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_today() {
            let args = [ctcore::ct_util_name(), "-d", "next today"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_tomorrow() {
            let args = [ctcore::ct_util_name(), "-d", "next tomorrow"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_month() {
            let args = [ctcore::ct_util_name(), "-d", "next month"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_week() {
            let args = [ctcore::ct_util_name(), "-d", "next week"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_friday() {
            let args = [ctcore::ct_util_name(), "-d", "next Friday"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_date() {
            let args = [ctcore::ct_util_name(), "-d", "next", "2024-05-01"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_err());
            println!("{}", result.err().unwrap());
        }

        #[test]
        fn test_date_main_d_next_string() {
            let args = [ctcore::ct_util_name(), "-d", "next May 1 2024"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", datefile];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_f() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-f", datefile];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_iso_8601_date() {
            let args = [ctcore::ct_util_name(), "--iso-8601=date"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_iso_8601_hours() {
            let args = [ctcore::ct_util_name(), "--iso-8601=hours"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_iso_8601_minutes() {
            let args = [ctcore::ct_util_name(), "--iso-8601=minutes"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_iso_8601_seconds() {
            let args = [ctcore::ct_util_name(), "--iso-8601=seconds"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_i_8601_date() {
            let args = [ctcore::ct_util_name(), "-Idate"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_i_8601_hours() {
            let args = [ctcore::ct_util_name(), "-Ihours"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_i_8601_minutes() {
            let args = [ctcore::ct_util_name(), "-Iminutes"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_i_8601_seconds() {
            let args = [ctcore::ct_util_name(), "-Iseconds"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_rfc_mail() {
            let args = [ctcore::ct_util_name(), "--rfc-email"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_r_debug() {
            let args = [ctcore::ct_util_name(), "-R", "--debug"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_debug() {
            let args = [ctcore::ct_util_name(), "--debug"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_rfc_3339_date() {
            let args = [ctcore::ct_util_name(), "--rfc-3339=date"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_rfc_3339_hours() {
            let args = [ctcore::ct_util_name(), "--rfc-3339=hours"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_rfc_3339_minutes() {
            let args = [ctcore::ct_util_name(), "--rfc-3339=minutes"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_rfc_3339_seconds() {
            let args = [ctcore::ct_util_name(), "--rfc-3339=seconds"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_rfc_3339_ns() {
            let args = [ctcore::ct_util_name(), "-R", "--rfc-3339=ns"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_r_rfc_3339_date() {
            let args = [ctcore::ct_util_name(), "-R", "--rfc-3339=date"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_r_rfc_3339_seconds() {
            let args = [ctcore::ct_util_name(), "-R", "--rfc-3339=seconds"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_r_rfc_3339_ns() {
            let args = [ctcore::ct_util_name(), "--rfc-3339=ns"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_r_rfc_mail() {
            let args = [ctcore::ct_util_name(), "-R", "--rfc-email"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_reference() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-r", datefile];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_reference_whole() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--reference", datefile];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_u() {
            let args = [ctcore::ct_util_name(), "-u"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_universal() {
            let args = [ctcore::ct_util_name(), "--universal"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_iso_8601_date() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--iso-8601=date",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_iso_8601_hours() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--iso-8601=hours",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_iso_8601_minutes() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--iso-8601=minutes",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_iso_8601_seconds() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--iso-8601=seconds",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_i8601_date() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", datefile, "-Idate"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_i8601_hours() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", datefile, "-Ihours"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_i8601_minutes() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", datefile, "-Iminutes"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_i8601_seconds() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", datefile, "-Iseconds"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_rfc_mail() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", datefile, "--rfc-email"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_r_debug() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", datefile, "-R", "--debug"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_debug() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", datefile, "--debug"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_rfc_3339_date() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--rfc-3339=date",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_rfc_3339_hours() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--rfc-3339=hours",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_file_rfc_3339_minutes() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--rfc-3339=minutes",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_file_rfc_3339_seconds() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "--rfc-3339=seconds",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_file_rfc_3339_ns() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "-R",
                "--rfc-3339=ns",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_r_file_rfc_3339_date() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "-R",
                "--rfc-3339=date",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_r_file_rfc_3339_seconds() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--file",
                datefile,
                "-R",
                "--rfc-3339=seconds",
            ];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_r_file_rfc_3339_ns() {
            let temp_dir = Builder::new()
                .prefix("test_date_main_file")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_date_main_file.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let datefile = test_file_1.to_str().unwrap();

            let content = "Thu Apr 25 11:25:00 AM CST 2024\n\
                   Thu Apr 26 11:25:00 AM CST 2024\n\
                   Thu Apr 27 11:25:00 AM CST 2024\n\
                   Thu Apr 28 11:25:00 AM CST 2024\n\
                   Thu Apr 29 11:25:00 AM CST 2024\n\
                   Thu Apr 30 11:25:00 AM CST 2024\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", datefile, "--rfc-3339=ns"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_format_a() {
            let args = [ctcore::ct_util_name(), "+%a"];

            let result = date_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    mod tests_date_format {
        use crate::date_main;
        use std::ffi::OsString;

        // 定义一个宏来生成测试用例
        macro_rules! test_date_format {
            ($name:ident, $format:expr) => {
                #[test]
                fn $name() {
                    let args = vec![
                        OsString::from(ctcore::ct_util_name()), // 假设这是获取程序名的方法
                        OsString::from(format!("+{}", $format)),
                    ];

                    assert!(date_main(args.into_iter()).is_ok());
                }
            };
        }

        // 使用宏生成测试用例
        // 定义所有格式化参数的测试用例
        test_date_format!(test_date_format_a, "%a"); // 本地化的缩写星期几名称
        test_date_format!(test_date_format_aa, "%A"); // 本地化的完整星期几名称
        test_date_format!(test_date_format_b, "%b"); // 本地化的缩写月份名称
        test_date_format!(test_date_format_bb, "%B"); // 本地化的完整月份名称
        test_date_format!(test_date_format_c, "%c"); // 本地化的日期和时间表示
        test_date_format!(test_date_format_cc, "%C"); // 世纪数（年份的前两位）
        test_date_format!(test_date_format_d, "%d"); // 月份中的日子（01-31）
        test_date_format!(test_date_format_dd, "%D"); // 日期，格式为%m/%d/%y
        test_date_format!(test_date_format_e, "%e"); // 月份中的日子，空格填充
        test_date_format!(test_date_format_ff, "%F"); // 完整日期，格式为%Y-%m-%d
        test_date_format!(test_date_format_g, "%g"); // ISO周号的年份的后两位数字
        test_date_format!(test_date_format_gg, "%G"); // ISO周号的年份
        test_date_format!(test_date_format_h, "%h"); // 与%b相同，本地化的缩写月份名称
        test_date_format!(test_date_format_hh, "%H"); // 小时数（00-23）
        test_date_format!(test_date_format_ii, "%I"); // 小时数（01-12）
        test_date_format!(test_date_format_j, "%j"); // 一年中的天数（001-366）
        test_date_format!(test_date_format_k, "%k"); // 小时数（0-23），空格填充
        test_date_format!(test_date_format_l, "%l"); // 小时数（1-12），空格填充
        test_date_format!(test_date_format_m, "%m"); // 月份（01-12）
        test_date_format!(test_date_format_mm, "%M"); // 分钟数（00-59）
        test_date_format!(test_date_format_n, "%n"); // 换行符
        test_date_format!(test_date_format_nn, "%N"); // 纳秒数（000000000-999999999）
        test_date_format!(test_date_format_p, "%p"); // 本地化的AM或PM
        test_date_format!(test_date_format_pp, "%P"); // 与%p相同，但为小写
        // test_date_format!(test_date_format_q, "%q"); // 季度号（1-4）  //TODO 与系统命令不一致，系统命令支持该参数
        test_date_format!(test_date_format_r, "%r"); // 本地化的12小时制时间
        test_date_format!(test_date_format_rr, "%R"); // 24小时制的时间，格式为%H:%M
        test_date_format!(test_date_format_s, "%s"); // 自1970-01-01 00:00:00 UTC以来的秒数
        test_date_format!(test_date_format_ss, "%S"); // 秒数（00-60）
        test_date_format!(test_date_format_t, "%t"); // 制表符
        test_date_format!(test_date_format_tt, "%T"); // 时间，格式为%H:%M:%S
        test_date_format!(test_date_format_u, "%u"); // 星期几的数字（1-7），1为星期一
        test_date_format!(test_date_format_uu, "%U"); // 一年中的周数，星期日为每周的开始
        test_date_format!(test_date_format_vv, "%V"); // ISO周数，星期一为每周的开始
        test_date_format!(test_date_format_w, "%w"); // 星期几的数字（0-6），0为星期日
        test_date_format!(test_date_format_ww, "%W"); // 一年中的周数，星期一为每周的开始
        test_date_format!(test_date_format_x, "%x"); // 本地化的日期表示
        test_date_format!(test_date_format_xx, "%X"); // 本地化的时间表示
        test_date_format!(test_date_format_y, "%y"); // 年份的后两位数字
        test_date_format!(test_date_format_yy, "%Y"); // 年份
        test_date_format!(test_date_format_z, "%z"); // 数字时区（+hhmm或-hhmm）
        test_date_format!(test_date_format_colon_z, "%:z"); // 数字时区，格式为±hh:mm
        test_date_format!(test_date_format_double_colon_z, "%::z"); // 数字时区，格式为±hh:mm:ss
        test_date_format!(test_date_format_triple_colon_z, "%:::z"); // 数字时区，以':'分隔至必要的精度
        test_date_format!(test_date_format_zz, "%Z"); // 字母时区缩写
    }
}
