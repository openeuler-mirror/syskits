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

extern crate rust_i18n;
use chrono::{Local, TimeZone, Utc};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use clap::{Arg, ArgAction, Command, crate_version};

use ctcore::Tool;
use ctcore::ct_error::{CTResult, CtSimpleError};
use std::ffi::OsString;
use sys_locale::get_locale;

use crate::platform::{get_uptime, print_loadavg, process_utmpx};

mod platform;

const UPTIME_SECS_PER_DAY: i64 = 86400;
const UPTIME_SECS_PER_HOUR: i64 = 3600;
const UPTIME_SECS_PER_MIN: i64 = 60;

pub mod uptime_flags {
    pub static SINCE: &str = "since";
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
        _ => format!("up {up_hours:2}:{up_mins:02},  "),
    }
}

fn uptime_print_time() -> String {
    let local_time = Local::now().time();

    format!(" {} ", local_time.format("%H:%M:%S"))
}

fn uptime_print_n_users(n_users: usize) -> String {
    match n_users.cmp(&1) {
        std::cmp::Ordering::Equal => "1 user,  ".to_string(),
        std::cmp::Ordering::Greater => format!("{n_users} users,  "),
        _ => "".to_string(),
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    uptime_main(args)
}

pub fn uptime_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;

    let (boot_time, user_count) = process_utmpx();
    let uptime = get_uptime(boot_time);
    if uptime < 0 {
        Err(CtSimpleError::new(1, "could not retrieve system uptime"))
    } else {
        if matches.get_flag(uptime_flags::SINCE) {
            let initial_date = Local
                .timestamp_opt(Utc::now().timestamp() - uptime, 0)
                .unwrap();
            println!("{}", initial_date.format("%Y-%m-%d %H:%M:%S"));
            return Ok(());
        }

        let time_result = uptime_print_time();
        let up_secs = uptime;
        let uptime_result = uptime_print_uptime(up_secs);
        let users_result = uptime_print_n_users(user_count);
        let loadavg_result = print_loadavg();

        print!(
            "{}{}{}{}",
            time_result, uptime_result, users_result, loadavg_result
        );

        Ok(())
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("uptime.about");
    let usage_description = t!("uptime.usage");
    let arg = Arg::new(uptime_flags::SINCE)
        .short('s')
        .long(uptime_flags::SINCE)
        .help(t!("uptime.clap.since"))
        .action(ArgAction::SetTrue);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
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
        let tool = Uptime::default();

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
            assert_eq!("up  0:01,  ", result);
        }

        #[test]
        fn test_uptime_print_uptime_seconds() {
            let up_secs = 10; // 10 seconds
            let result = uptime_print_uptime(up_secs);
            assert_eq!("up  0:00,  ", result);
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
            assert_eq!("up  0:00,  ", result);
        }

        // Test with negative uptime (edge case, though unrealistic)
        #[test]
        fn test_uptime_print_uptime_negative() {
            let up_secs = -10;
            let result = uptime_print_uptime(up_secs);
            // Depending on how you want to handle negative values, the expected output may vary.
            // Assuming it's treated as zero or has a specific error message.
            // Here we assume it's treated as zero for simplicity.
            assert_eq!("up  0:00,  ", result);
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
            assert_eq!(uptime_print_n_users(0), "");
            assert_eq!(uptime_print_n_users(1), "1 user,  ");
            assert_eq!(uptime_print_n_users(2), "2 users,  ");
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_ct_app_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = uptime_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = uptime_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = uptime_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = uptime_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = uptime_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
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
    }
}
