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

//! uptime 命令来查看系统的运行时间和平均负载情况
//  |       功能         |  GNU Coreutils  |  Procps |syskits| 状态 |
//  | -------------------|----------------|--------|------|------|
//  | 基本功能(无参数)     | 支持           | 支持   | 支持 | 一致|
//  | -s/--since 选项    | 支持           | 支持   | 支持 | 一致|
//  | --help 选项        | 支持           | 支持   | 支持 | 一致 |
//  | --version 选项     | 支持           | 支持   | 支持 | 一致 |
//  | -p/--pretty 选项   | 不支持         | 支持   | 支持 | (coreutils 不支持) |
//  | FILE 参数          | 支持           | 不支持 | 支持 | GNU 兼容 |
//  | 错误处理            | 支持           | 支持   | 支持 | 一致|
//  |  输出格式            | 支持           | 支持   | 支持 | 一致 |
// 默认行为尽量兼容 procps，同时保留 GNU coreutils 的 [FILE] 语义。

extern crate rust_i18n;
use chrono::{Local, TimeZone};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, ArgAction, Command, crate_version};

use ctcore::Tool;
use ctcore::ct_error::{CTResult, CtSimpleError, set_ct_exit_code, strip_errno};
use std::ffi::OsString;
use sys_locale::get_locale;

use crate::platform::{
    get_loadavg_values, get_uptime_from_boot_time, get_uptime_with_source, process_utmpx,
    uptime_source_kind,
};

mod platform;

const UPTIME_SECS_PER_DAY: i64 = 86400;
const UPTIME_SECS_PER_WEEK: i64 = UPTIME_SECS_PER_DAY * 7;
const UPTIME_SECS_PER_YEAR: i64 = UPTIME_SECS_PER_DAY * 365;
const UPTIME_SECS_PER_DECADE: i64 = UPTIME_SECS_PER_YEAR * 10;
const UPTIME_SECS_PER_HOUR: i64 = 3600;
const UPTIME_SECS_PER_MIN: i64 = 60;

pub mod uptime_flags {
    pub static SINCE: &str = "since";
    pub static PRETTY: &str = "pretty";
}

#[derive(Debug, Clone, PartialEq)]
pub struct UptimeSemantic {
    pub view_kind: String,
    pub uptime_source_kind: String,
    pub sample_time_unix: i64,
    pub sample_time_local: String,
    pub boot_time_unix: Option<i64>,
    pub boot_time_local: Option<String>,
    pub uptime_seconds: Option<i64>,
    pub uptime_pretty: Option<String>,
    pub user_count: usize,
    pub load_averages: Vec<f64>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

fn uptime_effective_boot_time(sample_time_unix: i64, uptime_seconds: Option<i64>) -> Option<i64> {
    uptime_seconds.map(|uptime| sample_time_unix - uptime)
}

fn uptime_boot_time_error(read_error: Option<&std::io::Error>) -> String {
    read_error
        .map(|err| format!("couldn't get boot time: {}", strip_errno(err)))
        .unwrap_or_else(|| "couldn't get boot time".to_string())
}

fn uptime_format_timestamp_local(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .unwrap()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn uptime_print_time_at(sample_time_unix: i64) -> String {
    let local_time = Local.timestamp_opt(sample_time_unix, 0).unwrap().time();
    format!(" {}  ", local_time.format("%H:%M:%S"))
}

fn uptime_render_loadavg(load_averages: &[f64]) -> String {
    if load_averages.is_empty() {
        String::new()
    } else {
        let mut result = "load average: ".to_string();
        for (index, value) in load_averages.iter().enumerate() {
            let separator = if index + 1 == load_averages.len() {
                "\n"
            } else {
                ", "
            };
            result.push_str(&format!("{value:.2}{separator}"));
        }
        result
    }
}

fn uptime_print_uptime(up_secs: i64) -> String {
    let up_days = up_secs / UPTIME_SECS_PER_DAY;
    let up_hours = (up_secs - (up_days * UPTIME_SECS_PER_DAY)) / UPTIME_SECS_PER_HOUR;
    let up_mins = (up_secs - (up_days * UPTIME_SECS_PER_DAY) - (up_hours * UPTIME_SECS_PER_HOUR))
        / UPTIME_SECS_PER_MIN;
    match up_days.cmp(&1) {
        std::cmp::Ordering::Equal => format!("up {up_days:1} day, {up_hours:2}:{up_mins:02},  "),
        std::cmp::Ordering::Greater => {
            format!("up {up_days:1} days, {up_hours:2}:{up_mins:02},  ")
        }
        _ => {
            if up_hours == 0 {
                format!("up {up_mins} min,  ")
            } else {
                format!("up {up_hours:2}:{up_mins:02},  ")
            }
        }
    }
}

#[cfg(test)]
fn uptime_print_time() -> String {
    uptime_print_time_at(Local::now().timestamp())
}

fn uptime_print_n_users(n_users: usize) -> String {
    match n_users.cmp(&1) {
        std::cmp::Ordering::Equal => "1 user,  ".to_string(),
        std::cmp::Ordering::Greater => format!("{n_users} users,  "),
        _ => "0 users,  ".to_string(),
    }
}

fn uptime_print_pretty(up_secs: i64) -> String {
    let mut uptime_secs = up_secs.max(0);

    let mut decades = 0;
    let mut years = 0;
    let mut weeks = 0;
    let mut days = 0;
    let mut hours = 0;
    let mut minutes = 0;
    let mut parts = Vec::new();

    if uptime_secs > UPTIME_SECS_PER_DECADE {
        decades = uptime_secs / UPTIME_SECS_PER_DECADE;
        uptime_secs -= decades * UPTIME_SECS_PER_DECADE;
    }

    if uptime_secs > UPTIME_SECS_PER_YEAR {
        years = uptime_secs / UPTIME_SECS_PER_YEAR;
        uptime_secs -= years * UPTIME_SECS_PER_YEAR;
    }

    if uptime_secs > UPTIME_SECS_PER_WEEK {
        weeks = uptime_secs / UPTIME_SECS_PER_WEEK;
        uptime_secs -= weeks * UPTIME_SECS_PER_WEEK;
    }

    if uptime_secs > UPTIME_SECS_PER_DAY {
        days = uptime_secs / UPTIME_SECS_PER_DAY;
        uptime_secs -= days * UPTIME_SECS_PER_DAY;
    }

    if uptime_secs > UPTIME_SECS_PER_HOUR {
        hours = uptime_secs / UPTIME_SECS_PER_HOUR;
        uptime_secs -= hours * UPTIME_SECS_PER_HOUR;
    }

    if uptime_secs > UPTIME_SECS_PER_MIN {
        minutes = uptime_secs / UPTIME_SECS_PER_MIN;
        uptime_secs -= minutes * UPTIME_SECS_PER_MIN;
    }

    let mut push_part = |value: i64, singular: &str, plural: &str| {
        let unit = if value == 1 { singular } else { plural };
        parts.push(format!("{value} {unit}"));
    };

    if decades > 0 {
        push_part(decades, "decade", "decades");
    }
    if years > 0 {
        push_part(years, "year", "years");
    }
    if weeks > 0 {
        push_part(weeks, "week", "weeks");
    }
    if days > 0 {
        push_part(days, "day", "days");
    }
    if hours > 0 {
        push_part(hours, "hour", "hours");
    }
    if minutes > 0 || uptime_secs <= UPTIME_SECS_PER_MIN {
        push_part(minutes, "minute", "minutes");
    }

    if parts.is_empty() {
        "up 0 minutes".to_string()
    } else {
        format!("up {}", parts.join(", "))
    }
}

fn uptime_print_unknown_uptime() -> &'static str {
    "up ???? days ??:??,  "
}

pub fn uptime_main(args: impl ctcore::Args) -> CTResult<()> {
    let semantic = uptime_native_semantic(args)?;
    if !semantic.classic_text.is_empty() {
        print!("{}", semantic.classic_text);
    }
    if !semantic.stderr_text.is_empty() {
        eprint!("{}", semantic.stderr_text);
    }
    if semantic.exit_code != 0 {
        set_ct_exit_code(semantic.exit_code);
    }
    Ok(())
}

pub fn uptime_native_semantic(args: impl ctcore::Args) -> CTResult<UptimeSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;
    let file_path = matches.get_one::<String>("file").map(String::as_str);
    let sample_time_unix = Local::now().timestamp();

    let (boot_time, user_count, read_error) = process_utmpx(file_path);
    let (uptime, uptime_source) = if file_path.is_some() {
        let uptime = get_uptime_from_boot_time(boot_time);
        let source = if uptime >= 0 {
            crate::platform::UptimeSource::BootTime
        } else {
            crate::platform::UptimeSource::Unknown
        };
        (uptime, source)
    } else {
        get_uptime_with_source(boot_time)
    };
    let uptime_seconds = if uptime >= 0 { Some(uptime) } else { None };
    let boot_time_unix = uptime_effective_boot_time(sample_time_unix, uptime_seconds);
    let boot_time_local = boot_time_unix.map(uptime_format_timestamp_local);
    let load_averages = get_loadavg_values();
    let uptime_pretty = uptime_seconds.map(uptime_print_pretty);

    if uptime < 0 && file_path.is_none() {
        Err(CtSimpleError::new(1, "could not retrieve system uptime"))
    } else {
        // -s 选项优先
        if matches.get_flag(uptime_flags::SINCE) {
            if uptime < 0 {
                return Err(CtSimpleError::new(
                    1,
                    uptime_boot_time_error(read_error.as_ref()),
                ));
            }
            let classic_text = format!(
                "{}\n",
                uptime_format_timestamp_local(boot_time_unix.expect("boot time"))
            );
            return Ok(UptimeSemantic {
                view_kind: "since".into(),
                uptime_source_kind: uptime_source_kind(uptime_source).into(),
                sample_time_unix,
                sample_time_local: uptime_format_timestamp_local(sample_time_unix),
                boot_time_unix,
                boot_time_local,
                uptime_seconds,
                uptime_pretty,
                user_count,
                load_averages,
                classic_text,
                stderr_text: String::new(),
                exit_code: 0,
            });
        }

        // -p 选项
        if matches.get_flag(uptime_flags::PRETTY) {
            if uptime < 0 {
                return Err(CtSimpleError::new(
                    1,
                    uptime_boot_time_error(read_error.as_ref()),
                ));
            }
            let classic_text = format!(
                "{}\n",
                uptime_pretty
                    .clone()
                    .unwrap_or_else(|| "up 0 minutes".to_string())
            );
            return Ok(UptimeSemantic {
                view_kind: "pretty".into(),
                uptime_source_kind: uptime_source_kind(uptime_source).into(),
                sample_time_unix,
                sample_time_local: uptime_format_timestamp_local(sample_time_unix),
                boot_time_unix,
                boot_time_local,
                uptime_seconds,
                uptime_pretty,
                user_count,
                load_averages,
                classic_text,
                stderr_text: String::new(),
                exit_code: 0,
            });
        }

        let mut stderr_text = String::new();
        let mut exit_code = 0;
        if file_path.is_some() && uptime < 0 {
            if let Some(err) = read_error {
                stderr_text = format!("uptime: couldn't get boot time: {}\n", strip_errno(&err));
            } else {
                stderr_text = "uptime: couldn't get boot time\n".to_string();
            }
            exit_code = 1;
        }

        // 默认格式
        let time_result = uptime_print_time_at(sample_time_unix);
        let uptime_result = if let Some(uptime_seconds) = uptime_seconds {
            uptime_print_uptime(uptime_seconds)
        } else {
            uptime_print_unknown_uptime().to_string()
        };
        let users_result = uptime_print_n_users(user_count);
        let loadavg_result = uptime_render_loadavg(&load_averages);
        let classic_text = format!("{time_result}{uptime_result}{users_result}{loadavg_result}");

        Ok(UptimeSemantic {
            view_kind: "default".into(),
            uptime_source_kind: uptime_source_kind(uptime_source).into(),
            sample_time_unix,
            sample_time_local: uptime_format_timestamp_local(sample_time_unix),
            boot_time_unix,
            boot_time_local,
            uptime_seconds,
            uptime_pretty,
            user_count,
            load_averages,
            classic_text,
            stderr_text,
            exit_code,
        })
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("uptime.about");
    let usage_description = t!("uptime.usage");

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(
            Arg::new(uptime_flags::SINCE)
                .short('s')
                .long(uptime_flags::SINCE)
                .help(t!("uptime.clap.since", default = "system up since"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(uptime_flags::PRETTY)
                .short('p')
                .long(uptime_flags::PRETTY)
                .help("show uptime in pretty format")
                .action(ArgAction::SetTrue),
        )
        .arg(Arg::new("file").value_name("FILE"))
}

#[derive(Default)]
pub struct Uptime;
impl Tool for Uptime {
    fn name(&self) -> &'static str {
        "uptime"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        uptime_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Uptime;

        // 测试 name 方法
        assert_eq!(tool.name(), "uptime");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("uptime"));

        // 测试 execute 方法
        let args = vec![OsString::from("uptime")];
        assert!(tool.execute(&args).is_ok());
    }

    #[cfg(test)]
    mod uptime_print_uptime_tests {
        use super::*;

        #[test]
        fn test_uptime_print_uptime_days() {
            let up_secs = 86400; // 1 day
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up 1 day,  0:00,  ", result);
        }

        #[test]
        fn test_uptime_print_uptime_hours() {
            let up_secs = 3600; // 1 hour
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up  1:00,  ", result);
        }

        #[test]
        fn test_uptime_print_uptime_minutes() {
            let up_secs = 60; // 1 minute
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up 1 min,  ", result);
        }

        #[test]
        fn test_uptime_print_uptime_seconds() {
            let up_secs = 10; // 10 seconds
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up 0 min,  ", result);
        }

        #[test]
        fn test_uptime_print_uptime_days_hours_minutes() {
            let up_secs = 90060; // 1 day, 1 hour, 1 minute
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up 1 day,  1:01,  ", result);
        }

        #[test]
        fn test_uptime_print_uptime_days_hours() {
            let up_secs = 54000;
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up 15:00,  ", result);
        }

        #[test]
        fn test_uptime_print_uptime_days_minutes() {
            let up_secs = 43200;
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up 12:00,  ", result);
        }

        // Test with multiple days
        #[test]
        fn test_uptime_print_uptime_multiple_days() {
            let up_secs = 2 * 86401; // 2 days
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up 2 days,  0:00,  ", result);
        }

        // Test with exactly one hour, no days or minutes
        #[test]
        fn test_uptime_print_uptime_exactly_one_hour() {
            let up_secs = 3601; // 1 hour
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up  1:00,  ", result);
        }

        // Test with zero uptime
        #[test]
        fn test_uptime_print_uptime_zero() {
            let up_secs = 0;
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up 0 min,  ", result);
        }

        // Test with negative uptime (edge case, though unrealistic)
        #[test]
        fn test_uptime_print_uptime_negative() {
            let up_secs = -10;
            let result = uptime_print_uptime(up_secs);
            // Depending on how you want to handle negative values, the expected output may vary.
            // Assuming it's treated as zero or has a specific error message.
            // Here we assume it's treated as zero for simplicity.
            assert_eq!("up 0 min,  ", result);
        }
    }

    #[cfg(test)]
    mod uptime_print_time_tests {
        use super::*;

        #[test]
        fn test_uptime_print_time() {
            let formatted_time = uptime_print_time();

            assert!(formatted_time.contains(":"));
        }
    }

    #[cfg(test)]
    mod uptime_print_n_users_tests {
        use super::*;

        #[test]
        fn test_uptime_print_n_users() {
            assert_eq!(uptime_print_n_users(0), "0 users,  ");
            assert_eq!(uptime_print_n_users(1), "1 user,  ");
            assert_eq!(uptime_print_n_users(2), "2 users,  ");
        }
    }

    #[cfg(test)]
    mod uptime_print_pretty_tests {
        use super::*;

        #[test]
        fn test_uptime_print_pretty() {
            assert_eq!(uptime_print_pretty(0), "up 0 minutes");
            assert_eq!(uptime_print_pretty(60), "up 0 minutes");
            assert_eq!(uptime_print_pretty(61), "up 1 minute");
            assert_eq!(uptime_print_pretty(120), "up 2 minutes");
            assert_eq!(uptime_print_pretty(3600), "up 60 minutes");
            assert_eq!(uptime_print_pretty(3660), "up 1 hour, 0 minutes");
            assert_eq!(uptime_print_pretty(7200), "up 2 hours, 0 minutes");
            assert_eq!(uptime_print_pretty(7320), "up 2 hours, 2 minutes");
            assert_eq!(uptime_print_pretty(86400), "up 24 hours, 0 minutes");
            assert_eq!(uptime_print_pretty(90000), "up 1 day, 60 minutes");
            assert_eq!(uptime_print_pretty(90061), "up 1 day, 1 hour, 1 minute");
            assert_eq!(uptime_print_pretty(180122), "up 2 days, 2 hours, 2 minutes");
            assert_eq!(
                uptime_print_pretty(28_987_200),
                "up 47 weeks, 6 days, 12 hours, 0 minutes"
            );
            assert_eq!(
                uptime_print_pretty(289_872_000),
                "up 9 years, 10 weeks, 0 minutes"
            );
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code};
        use std::ffi::OsString;

        #[test]
        fn test_ct_app_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = uptime_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = uptime_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = uptime_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = uptime_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = uptime_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_rejects_extra_operand() {
            let args = [ctcore::ct_util_name(), "first-file", "second-file"];
            let result = uptime_main(args.iter().map(OsString::from));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }

        #[test]
        fn test_ct_app_missing_file_operand_sets_exit_code() {
            set_ct_exit_code(0);
            let args = [ctcore::ct_util_name(), "/nonexistent/utmp-file"];
            let result = uptime_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
            assert_eq!(get_ct_exit_code(), 1);
            set_ct_exit_code(0);
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // uptime 接口: uptime [OPTION]...
        //
        // Options:
        //   -s, --since    system up since
        //   -h, --help     Print help
        //   -V, --version  Print version

        #[test]
        fn test_ct_app_execution_parsing_s() {
            let command = ct_app();

            // 测试正确解析 `-s` 选项
            let args = vec![ctcore::ct_util_name(), "-s"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_argument_parsing_since() {
            let command = ct_app();

            // 测试正确解析 `--since` 选项
            let args = vec![ctcore::ct_util_name(), "--since"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

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
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
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
        fn test_ct_app_accepts_single_file_operand() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "unexpected-file"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<String>("file")
                    .map(String::as_str),
                Some("unexpected-file")
            );
        }

        #[test]
        fn test_ct_app_rejects_extra_operand() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "first-file", "second-file"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }
    }
}
