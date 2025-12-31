/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

use chrono::format::StrftimeItems;
use chrono::{DateTime, FixedOffset, Local, Offset, TimeDelta, Utc};
#[cfg(windows)]
use chrono::{Datelike, Timelike};
use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::FromIo;
use ctcore::ct_error::{CTResult, CtSimpleError};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show};
#[cfg(all(unix, not(target_os = "macos"), not(target_os = "redox")))]
use libc::{clock_settime, timespec, CLOCK_REALTIME};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
#[cfg(windows)]
use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::SetSystemTime};

use ctcore::ct_shortcut_value_parser::CtShortcutValueParser;

// Options
const DATE: &str = "date";
const HOURS: &str = "hours";
const MINUTES: &str = "minutes";
const SECONDS: &str = "seconds";
const NS: &str = "ns";

const DATE_ABOUT: &str = ct_help_about!("date.md");
const DATE_USAGE: &str = ct_help_usage!("date.md");

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

#[cfg(not(any(target_os = "macos", target_os = "redox")))]
static DATE_OPT_SET_HELP_STRING: &str = "set time described by STRING";
#[cfg(target_os = "macos")]
static OPT_SET_HELP_STRING: &str = "set time described by STRING (not available on mac yet)";
#[cfg(target_os = "redox")]
static OPT_SET_HELP_STRING: &str = "set time described by STRING (not available on redox yet)";

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
}

enum DateIso8601Format {
    Date,
    Hours,
    Minutes,
    Seconds,
    Ns,
}

impl<'a> From<&'a str> for DateIso8601Format {
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
impl<'a> From<&'a str> for DateRfc3339Format {
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

#[ctcore::main]
#[allow(clippy::cognitive_complexity)]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    date_main(args).map(|_| ())
}

/**
 * 主函数，用于处理命令行参数并设置或显示系统日期和时间。
 *
 * @param args 命令行参数，实现了 `ctcore::Args` 接口。
 * @return `CTResult<()>`，成功时返回 `Ok(())`，错误时返回包含错误信息的 `Err`。
 */
pub fn date_main(args: impl ctcore::Args) -> CTResult<()> {
    // 从命令行参数中解析匹配项
    let args_match = ct_app().try_get_matches_from(args)?;

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
                let date = parse_date(input.clone());
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
                            format!("invalid date {}", relative_time),
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
                let iter = lines.map_while(Result::ok).map(parse_date);
                Box::new(iter)
            }
            DateSource::Now => {
                let iter = std::iter::once(Ok(now));
                Box::new(iter)
            }
        };

        // 根据日期设置生成格式化字符串
        let format_string = make_format_string(&date_set);

        // 格式化并打印所有日期
        for date in dates_iterator {
            match date {
                Ok(date) => {
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
    let date_source = if let Some(date) = args_match.get_one::<String>(DATE_OPT_DATE) {
        let ref_time = Local::now();
        if let Ok(new_time) = parse_datetime::parse_datetime_at_date(ref_time, date.as_str()) {
            let duration = new_time.signed_duration_since(ref_time);
            DateSource::Human(duration)
        } else {
            DateSource::Custom(date.into())
        }
    } else if let Some(file) = args_match.get_one::<String>(DATE_OPT_FILE) {
        DateSource::File(file.into())
    } else {
        DateSource::Now
    };
    date_source
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
    } else {
        DateFormat::Default
    };
    Ok(date_format)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = DATE_ABOUT;
    let usage_description = ct_format_usage(DATE_USAGE);

    let args = date_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn date_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(DATE_OPT_DATE)
            .short('d')
            .long(DATE_OPT_DATE)
            .value_name("STRING")
            .help("display time described by STRING, not 'now'"),
        Arg::new(DATE_OPT_FILE)
            .short('f')
            .long(DATE_OPT_FILE)
            .value_name("DATEFILE")
            .value_hint(clap::ValueHint::FilePath)
            .help("like --date; once for each line of DATEFILE"),
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
            .help("annotate the parsed date, and warn about questionable usage to stderr")
            .action(ArgAction::SetTrue),
        Arg::new(DATE_OPT_REFERENCE)
            .short('r')
            .long(DATE_OPT_REFERENCE)
            .value_name("FILE")
            .value_hint(clap::ValueHint::AnyPath)
            .help("display the last modification time of FILE"),
        Arg::new(DATE_OPT_SET)
            .short('s')
            .long(DATE_OPT_SET)
            .value_name("STRING")
            .help(DATE_OPT_SET_HELP_STRING),
        Arg::new(DATE_OPT_UNIVERSAL)
            .short('u')
            .long(DATE_OPT_UNIVERSAL)
            .alias(DATE_OPT_UNIVERSAL_2)
            .help("print or set Coordinated Universal Time (UTC)")
            .action(ArgAction::SetTrue),
        Arg::new(DATE_OPT_FORMAT),
    ];
    args
}

/// Return the appropriate ct_format string for the given settings.
fn make_format_string(date_settings: &DateSettings) -> &str {
    // 在 Rust 中，ref 关键字用于在模式匹配中创建一个绑定（binding），这个绑定是对原始匹配值的引用（reference）。在这个上下文中，
    // ref fmt 表示在匹配 DateFormat::Iso8601 枚举值时，不是将整个 fmt 值复制给 fmt 变量，而是创建一个指向 fmt 内部数据的引用。
    // 这意味着 fmt 是 DateIso8601Format 类型的一个引用，而不是它的副本。
    // 接下来的 match *fmt 则是解引用这个引用，以便进一步根据 DateIso8601Format 的具体值来决定应该选择哪个字符串格式。解引用允许我们访问 fmt 引用所指向的枚举变量的实际值，而不是引用本身。
    // 所以，ref 的作用是确保在匹配过程中不会移动或复制枚举的内部值，而是直接操作它的引用，这样可以在后续的代码中避免不必要的拷贝，并且可以安全地修改（如果枚举是可变引用的话）或读取枚举变量的内容。
    //

    (if let DateFormat::Iso8601(ref fmt) = date_settings.format {
        if let DateIso8601Format::Date = *fmt {
            "%F"
        } else if let DateIso8601Format::Hours = *fmt {
            "%FT%H%:z"
        } else if let DateIso8601Format::Minutes = *fmt {
            "%FT%H:%M%:z"
        } else if let DateIso8601Format::Seconds = *fmt {
            "%FT%T%:z"
        } else {
            "%FT%T,%f%:z"
        }
    } else if let DateFormat::Rfc5322 = date_settings.format {
        "%a, %d %h %Y %T %z"
    } else if let DateFormat::Rfc3339(ref fmt) = date_settings.format {
        if let DateRfc3339Format::Date = *fmt {
            "%F"
        } else if let DateRfc3339Format::Seconds = *fmt {
            "%F %T%:z"
        } else {
            "%F %T.%f%:z"
        }
    } else if let DateFormat::Custom(ref fmt) = date_settings.format {
        fmt
    } else {
        "%c"
    }) as _ //占位符，依赖编译器推断出类型转换的目标类型
}

/// Parse a `String` into a `DateTime`.
/// If it fails, return a tuple of the `String` along with its `ParseError`.
/*
函数签名：fn parse_date<S: AsRef<str> + Clone>(s: S) -> Result<DateTime<FixedOffset>, (String, chrono::format::ParseError)>。
这个函数名为parse_date，它接受一个类型参数S，这个S必须实现AsRef<str>和Clone这两个trait。AsRef<str>允许S可以被转换为一个字符串引用，而Clone使得我们可以复制S的值。
函数返回一个Result类型，其中Ok部分是解析成功的DateTime<FixedOffset>对象，Err部分是一个错误元组，包含一个错误信息字符串和一个chrono::format::ParseError。

功能：此函数的目的是将输入的字符串S解析为一个日期时间（DateTime）对象，使用的是chrono库中的FixedOffset时区。FixedOffset代表固定偏移量的时区，例如UTC+8。

实现：函数体内的代码s.as_ref().parse().map_err(|e| (s.as_ref().into(), e))执行以下操作：

s.as_ref().parse()：尝试将S转换为的字符串引用解析为DateTime<FixedOffset>。这会使用chrono库的默认日期时间格式进行解析。
.map_err(|e| (s.as_ref().into(), e))：这是一个错误处理操作，如果解析失败，它会捕获解析错误e，并将输入的字符串引用s.as_ref()转换为String（通过.into()），
然后将这两者打包成一个元组(String, ParseError)，作为Result的Err部分返回。

使用场景：这个函数通常会在需要从用户输入或文件中解析日期时间的场景中使用，例如读取日志文件或处理命令行参数。由于它返回了一个Result，调用者需要处理可能的解析错误。
*/

fn parse_date<S: AsRef<str> + Clone>(
    s: S,
) -> Result<DateTime<FixedOffset>, (String, chrono::format::ParseError)> {
    // TODO: The GNU date command can parse a wide variety of inputs.

    let input = s.as_ref();
    match input.parse() {
        Ok(date) => Ok(date),
        Err(e) => Err((input.into(), e)),
    }
}

#[cfg(not(any(unix, windows)))]
fn set_system_datetime(_date: DateTime<Utc>) -> CTResult<()> {
    unimplemented!("setting date not implemented (unsupported target)");
}

#[cfg(target_os = "macos")]
fn set_system_datetime(_date: DateTime<Utc>) -> CTResult<()> {
    Err(CtSimpleError::new(
        1,
        "setting the date is not supported by macOS".to_string(),
    ))
}

#[cfg(target_os = "redox")]
fn set_system_datetime(_date: DateTime<Utc>) -> CTResult<()> {
    Err(CtSimpleError::new(
        1,
        "setting the date is not supported by Redox".to_string(),
    ))
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "redox")))]
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
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_date_main_date_yesterday() {
            let args = vec![ctcore::ct_util_name(), "--date", "yesterday"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_today() {
            let args = vec![ctcore::ct_util_name(), "--date", "today"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_tomorrow() {
            let args = vec![ctcore::ct_util_name(), "--date", "tomorrow"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_month() {
            let args = vec![ctcore::ct_util_name(), "--date", "month"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_week() {
            let args = vec![ctcore::ct_util_name(), "--date", "week"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_friday() {
            let args = vec![ctcore::ct_util_name(), "--date", "Friday"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_date() {
            let args = vec![ctcore::ct_util_name(), "--date", "2024-05-01"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_string() {
            let args = vec![ctcore::ct_util_name(), "--date", "May 1 2024"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_yesterday() {
            let args = vec![ctcore::ct_util_name(), "--date", "next yesterday"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_today() {
            let args = vec![ctcore::ct_util_name(), "--date", "next today"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_tomorrow() {
            let args = vec![ctcore::ct_util_name(), "--date", "next tomorrow"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_month() {
            let args = vec![ctcore::ct_util_name(), "--date", "next month"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_week() {
            let args = vec![ctcore::ct_util_name(), "--date", "next week"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_friday() {
            let args = vec![ctcore::ct_util_name(), "--date", "next Friday"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_date() {
            let args = vec![ctcore::ct_util_name(), "--date", "next 2024-05-01"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_date_next_string() {
            let args = vec![ctcore::ct_util_name(), "--date", "next May 1 2024"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_yesterday() {
            let args = vec![ctcore::ct_util_name(), "-d", "yesterday"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_today() {
            let args = vec![ctcore::ct_util_name(), "-d", "today"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_tomorrow() {
            let args = vec![ctcore::ct_util_name(), "-d", "tomorrow"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_month() {
            let args = vec![ctcore::ct_util_name(), "-d", "month"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_week() {
            let args = vec![ctcore::ct_util_name(), "-d", "week"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_friday() {
            let args = vec![ctcore::ct_util_name(), "-d", "Friday"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_date() {
            let args = vec![ctcore::ct_util_name(), "-d", "2024-05-01"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_string() {
            let args = vec![ctcore::ct_util_name(), "-d", "May 1 2024"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_yesterday() {
            let args = vec![ctcore::ct_util_name(), "-d", "next yesterday"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_today() {
            let args = vec![ctcore::ct_util_name(), "-d", "next today"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_date_main_d_next_tomorrow() {
            let args = vec![ctcore::ct_util_name(), "-d", "next tomorrow"];

            let result = date_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }
}