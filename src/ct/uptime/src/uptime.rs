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

//! uptime 命令来查看系统的运行时间和平均负载情况

use chrono::{Local, TimeZone, Utc};
use clap::{crate_version, Arg, ArgAction, Command};

use ctcore::ct_error::{CTResult, CtSimpleError};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};

use crate::platform::{get_uptime, print_loadavg, process_utmpx};

mod platform;

const UPTIME_ABOUT: &str = ct_help_about!("uptime.md");
const UPTIME_USAGE: &str = ct_help_usage!("uptime.md");
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
    let application_info = UPTIME_ABOUT;
    let usage_description = ct_format_usage(UPTIME_USAGE);
    let arg = Arg::new(uptime_flags::SINCE)
        .short('s')
        .long(uptime_flags::SINCE)
        .help("system up since")
        .action(ArgAction::SetTrue);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
}

#[cfg(test)]
mod tests {
    use super::*;

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

}