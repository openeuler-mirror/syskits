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

//! sleep 命令用于使当前进程暂停执行一段时间。这个时间可以是以秒为单位的整数或浮点数。

use std::thread;
use std::time::Duration;

use clap::{crate_version, Arg, ArgAction, Command};
use fundu::{DurationParser, ParseError, SaturatingInto};

use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError};
use ctcore::ct_format_usage;
use ctcore::ct_help_about;
use ctcore::ct_help_section;
use ctcore::ct_help_usage;
use ctcore::ct_show_error;

const SLEEP_ABOUT: &str = ct_help_about!("sleep.md");
const SLEEP_USAGE: &str = ct_help_usage!("sleep.md");
const SLEEP_AFTER_HELP: &str = ct_help_section!("after help", "sleep.md");

mod sleep_flags {
    pub const SLEEP_NUMBER: &str = "NUMBER";
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
    let args = vec![Arg::new(sleep_flags::SLEEP_NUMBER)
        .help("pause for NUMBER seconds")
        .value_name(sleep_flags::SLEEP_NUMBER)
        .action(ArgAction::Append)];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(SLEEP_AFTER_HELP)
        .infer_long_args(true)
        .args(args)
}

