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

//! sleep 命令用于使当前进程暂停执行一段时间。这个时间可以是以秒为单位的整数或浮点数。

use std::thread;
use std::time::Duration;

use clap::{Arg, ArgAction, Command, crate_version};
use fundu::{DurationParser, ParseError, SaturatingInto};

use ctcore::Tool;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError};
use ctcore::ct_format_usage;
use ctcore::ct_help_about;
use ctcore::ct_help_section;
use ctcore::ct_help_usage;
use ctcore::ct_show_error;
use std::ffi::OsString;

const SLEEP_ABOUT: &str = ct_help_about!("sleep.md");
const SLEEP_USAGE: &str = ct_help_usage!("sleep.md");
const SLEEP_AFTER_HELP: &str = ct_help_section!("after help", "sleep.md");

mod sleep_flags {
    pub const SLEEP_NUMBER: &str = "NUMBER";
}

#[derive(Default)]
pub struct Sleep;
impl Tool for Sleep {
    fn name(&self) -> &'static str {
        "sleep"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        sleep_main(args.iter().cloned())
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    sleep_main(args)
}
pub fn sleep_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let numbers = sleep_parse_numbers(&matches)?;
    let sleep_dur = sleep_handle_second(&numbers)?;

    sleep(sleep_dur)
}

fn sleep_parse_numbers(matches: &clap::ArgMatches) -> CTResult<Vec<&str>> {
    let numbers = matches
        .get_many::<String>(sleep_flags::SLEEP_NUMBER)
        .ok_or_else(|| {
            let err_message = format!(
                "missing operand\nTry '{} --help' for more information.",
                ctcore::ct_execute_phrase()
            );
            CtSimpleError::new(1, err_message)
        })?
        .map(|sec| sec.as_str())
        .collect::<Vec<_>>();

    Ok(numbers)
}

fn sleep_handle_second(args: &[&str]) -> CTResult<Duration> {
    use fundu::TimeUnit::{Day, Hour, Minute, Second};
    let dur_parser = DurationParser::with_time_units(&[Second, Minute, Hour, Day]);

    let mut arg_error = false;

    let sleep_dur = args
        .iter()
        .filter_map(|input| match dur_parser.parse(input.trim()) {
            Ok(duration) => Some(duration),
            Err(parse_error) => {
                arg_error = true;

                let reason = match parse_error {
                    ParseError::Empty => {
                        if input.is_empty() {
                            "Input was empty".to_string()
                        } else {
                            "Found only whitespace in input".to_string()
                        }
                    }
                    ParseError::TimeUnit(pos, description)
                    | ParseError::Syntax(pos, description) => {
                        format!("{} at position {}", description, pos.saturating_add(1))
                    }
                    ParseError::PositiveExponentOverflow | ParseError::NegativeExponentOverflow => {
                        "Exponent was out of bounds".to_string()
                    }
                    ParseError::NegativeNumber => "Number was negative".to_string(),
                    error => error.to_string(),
                };
                ct_show_error!("invalid time interval '{}': {}", input, reason);

                None
            }
        })
        .fold(Duration::ZERO, |acc, n| {
            // acc 是累加器，初始值为 Duration::ZERO（即零时间间隔）。
            // 每次迭代，它将当前的 acc 与新解析出的 duration（n）相加, saturating_add 方法确保不会因溢出而导致负值。
            acc.saturating_add(SaturatingInto::<std::time::Duration>::saturating_into(n))
        });

    if arg_error {
        return Err(CTsageError::new(1, ""));
    };

    Ok(sleep_dur)
}

fn sleep(sleep_dur: Duration) -> CTResult<()> {
    thread::sleep(sleep_dur);

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = SLEEP_ABOUT;
    let usage_description = ct_format_usage(SLEEP_USAGE);
    let args = vec![
        Arg::new(sleep_flags::SLEEP_NUMBER)
            .help("pause for NUMBER seconds")
            .value_name(sleep_flags::SLEEP_NUMBER)
            .action(ArgAction::Append),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(SLEEP_AFTER_HELP)
        .infer_long_args(true)
        .args(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Sleep;

        // Test name method
        assert_eq!(tool.name(), "sleep");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("sleep"));

        // Test execute method with help flag (should work)
        let args: Vec<OsString> = vec![OsString::from("sleep"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    #[cfg(test)]
    mod handle_second_tests {
        use super::*;
        use clap::Command;

        #[test]
        fn test_sleep_parse_numbers() {
            let cmd = Command::new("test")
                .arg(Arg::new(sleep_flags::SLEEP_NUMBER).action(ArgAction::Append));

            let matches = cmd.try_get_matches_from(vec!["test", "5", "10"]).unwrap();
            let numbers = sleep_parse_numbers(&matches).unwrap();

            assert_eq!(numbers, vec!["5", "10"]);
        }

        #[test]
        fn test_sleep_handle_second() {
            let args = vec!["5s", "1m", "2h"];
            let duration = sleep_handle_second(&args).unwrap();

            assert_eq!(duration, Duration::from_secs(5 + 60 + 2 * 3600));
        }

        #[test]
        fn test_sleep_handle_second_invalid_input() {
            let args = vec!["5x", "1m"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_single_second() {
            let args = vec!["5s"];
            let duration = sleep_handle_second(&args).unwrap();

            assert_eq!(duration, Duration::from_secs(5));
        }

        #[test]
        fn test_sleep_handle_second_multiple_units() {
            let args = vec!["1d", "2h", "30m", "45s"];
            let duration = sleep_handle_second(&args).unwrap();

            let expected_duration = Duration::from_secs(1 * 86400 + 2 * 3600 + 30 * 60 + 45);
            assert_eq!(duration, expected_duration);
        }

        #[test]
        fn test_sleep_handle_second_empty_input() {
            let args = vec![""];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_whitespace_input() {
            let args = vec!["  "];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_invalid_format() {
            let args = vec!["5x", "2y"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_mixed_valid_invalid() {
            let args = vec!["5s", "invalid", "10m"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_handle_second_large_duration() {
            let args = vec!["1000d", "24h"];
            let duration = sleep_handle_second(&args).unwrap();

            let expected_duration = Duration::from_secs(1000 * 86400 + 24 * 3600);
            assert_eq!(duration, expected_duration);
        }

        #[test]
        fn test_sleep_handle_second_negative_duration() {
            let args = vec!["-5s"];
            let result = sleep_handle_second(&args);

            assert!(result.is_err());
        }
    }
    #[cfg(test)]
    mod sleep_parse_numbers_tests {
        use super::*;
        #[test]
        fn test_sleep_parse_numbers_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches);

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("missing operand"));
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_5() {
            let args = vec![ctcore::ct_util_name(), "5"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches).unwrap();

            assert_eq!(result, ["5"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_0() {
            let args = vec![ctcore::ct_util_name(), "0"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches).unwrap();

            assert_eq!(result, ["0"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_seconds_2() {
            let args = vec![ctcore::ct_util_name(), "2s"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches).unwrap();

            assert_eq!(result, ["2s"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_minutes_2() {
            let args = vec![ctcore::ct_util_name(), "2m"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches).unwrap();

            assert_eq!(result, ["2m"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_hours_2() {
            let args = vec![ctcore::ct_util_name(), "2h"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches).unwrap();

            assert_eq!(result, ["2h"]);
        }
        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_days_2() {
            let args = vec![ctcore::ct_util_name(), "2d"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches).unwrap();

            assert_eq!(result, ["2d"]);
        }

        #[test]
        fn test_sleep_parse_numbers_sleep_suffix_err_2() {
            let args = vec![ctcore::ct_util_name(), "2q"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = sleep_parse_numbers(&matches).unwrap();

            assert_eq!(result, ["2q"]);
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_sleep_main_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = sleep_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_execution_help_short() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_sleep_1() {
            let args = vec![ctcore::ct_util_name(), "1"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_0_3_0_2() {
            let args = vec![ctcore::ct_util_name(), "0.3", "0.2"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_0_3_0_2_0_1() {
            let args = vec![ctcore::ct_util_name(), "0.3", "0.2", "0.1"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_1_qq() {
            let args = vec![ctcore::ct_util_name(), "1", "qq"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sleep_main_sleep_0_1_0_3_s() {
            let args = vec![ctcore::ct_util_name(), "0.1s", "0.3s"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_0() {
            let args = vec![ctcore::ct_util_name(), "0"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_suffix_seconds_1() {
            let args = vec![ctcore::ct_util_name(), "1s"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sleep_main_sleep_suffix_err_1() {
            let args = vec![ctcore::ct_util_name(), "1q"];
            let result = sleep_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // sleep 接口: sleep NUMBER[SUFFIX]...
        //             sleep OPTION
        //
        // Arguments:
        //   [NUMBER]...  pause for NUMBER seconds
        //
        // Options:
        //   -h, --help     Print help
        //   -V, --version  Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
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
        fn test_ct_app_execution_help_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
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
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_5() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "5"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_0() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "0"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_seconds_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2s"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_minutes_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2m"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_hours_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2h"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_sleep_suffix_days_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2d"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sleep_suffix_err_2() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "2q"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
