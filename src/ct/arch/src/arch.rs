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

use clap::{crate_version, Command};
use ctcore::ct_error::{UResult, USimpleError};
use ctcore::{help_about, help_section};

static ABOUT: &str = help_about!("arch.md");
static SUMMARY: &str = help_section!("after help", "arch.md");

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> UResult<()> {
    ct_app().try_get_matches_from(args)?;

    let uts = PlatformInfo::new().map_err(|_e| USimpleError::new(1, "cannot get system name"))?;

    println!("{}", uts.machine().to_string_lossy().trim());
    Ok(())
}

pub fn ct_app() -> Command {
    Command::new(ctcore::util_name())
        .version(crate_version!())
        .about(ABOUT)
        .after_help(SUMMARY)
        .infer_long_args(true)
}
