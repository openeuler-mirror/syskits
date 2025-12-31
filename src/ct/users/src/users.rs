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

//! users命令用于显示当前登录系统的所有用户的用户列表
//! 每个显示的用户名对应一个登录会话。如果一个用户有不止一个登录会话，那他的用户名将显示相同的次数。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::builder::ValueParser;
use clap::{crate_version, Arg, ArgMatches, Command};

use ctcore::ct_error::CTResult;
use ctcore::ct_utmpx::{self, CtUtmpx};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};

const USERS_ABOUT: &str = ct_help_about!("users.md");
const USERS_USAGE: &str = ct_help_usage!("users.md");

static USERS_ARG_FILES: &str = "files";

fn users_get_long_usage() -> String {
    format!(
        "Output who is currently logged in according to FILE.
If FILE is not specified, use {}.  /var/log/wtmp as FILE is common.",
        ct_utmpx::DEFAULT_FILE
    )
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    match users_main(args) {
        Ok(users) => {
            if !users.is_empty() {
                println!("{}", users);
            }
            Ok(())
        }
        Err(err) => Err(err),
    }
}

pub fn users_main(args: impl ctcore::Args) -> CTResult<String> {
    let matches = ct_app()
        .after_help(users_get_long_usage())
        .try_get_matches_from(args)?;

    let filename = parse_users_files(matches);

    let mut users_info = CtUtmpx::iter_all_records_from(filename)
        .filter(CtUtmpx::is_user_process)
        .map(|ut| ut.user())
        .collect::<Vec<_>>();

    if !users_info.is_empty() {
        users_info.sort();
        let users = users_info.join(" ");
        Ok(users)
    } else {
        Ok(String::from(""))
    }
}

fn parse_users_files(matches: ArgMatches) -> PathBuf {
    let files: Vec<&Path> = matches
        .get_many::<OsString>(USERS_ARG_FILES)
        .map(|v| v.map(AsRef::as_ref).collect())
        .unwrap_or_default();

    let file_name = if files.is_empty() {
        ct_utmpx::DEFAULT_FILE.as_ref()
    } else {
        files[0]
    };

    file_name.to_path_buf()
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = USERS_ABOUT;
    let usage_description = ct_format_usage(USERS_USAGE);
    let arg = Arg::new(USERS_ARG_FILES)
        .num_args(1)
        .value_hint(clap::ValueHint::FilePath)
        .value_parser(ValueParser::os_string());

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
}

