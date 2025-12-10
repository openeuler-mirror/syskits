/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use platform_info::*;

#[warn(unused_imports)]
use clap::{crate_version, Command};
use ctcore::ct_error::{UResult, USimpleError};
use ctcore::{format_usage, help_about, help_section};

static CT_ABOUT: &str = help_about!("arch.md");
static CT_SUMMARY: &str = help_section!("after help", "arch.md");
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> UResult<()> {
    ct_main(args).map(|_| ())
}

pub fn ct_main(args: impl ctcore::Args) -> UResult<String> {
    ct_app().try_get_matches_from(args)?;

    let uts = PlatformInfo::new().map_err(|_e| USimpleError::new(1, "cannot get system name"))?;

    let binding = uts.machine().to_string_lossy();
    let s = binding.trim();
    println!("{}", uts.machine().to_string_lossy().trim());
    Ok(s.to_string())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::util_name();
    let command_version = crate_version!();
    let application_info = CT_ABOUT;
    let usage_description = format_usage(CT_SUMMARY);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .after_help(usage_description)
        .infer_long_args(true)
}
