/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use clap::{Command, crate_version};
use ctcore::{ct_error::CTResult, ct_format_usage, ct_help_about, ct_help_usage};
use libc::c_long;

const HOSTID_USAGE: &str = ct_help_usage!("hostid.md");
const HOSTID_ABOUT: &str = ct_help_about!("hostid.md");

// currently rust libc interface doesn't include gethostid
unsafe extern "C" {
    pub unsafe fn gethostid() -> c_long;
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    hostid_main(args)
}

pub fn hostid_main(args: impl ctcore::Args) -> CTResult<()> {
    ct_app().try_get_matches_from(args)?;
    hostid();
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = HOSTID_ABOUT;
    let usage_description = ct_format_usage(HOSTID_USAGE);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
}

fn hostid() {
    /*
     * POSIX says gethostid returns a "32-bit identifier" but is silent
     * whether it's sign-extended.  Turn off any sign-extension.  This
     * is a no-op unless unsigned int is wider than 32 bits.
     */

    let mut result: c_long = unsafe { gethostid() };

    #[allow(overflowing_literals)]
    let mask = 0xffff_ffff;

    result &= mask;
    println!("{result:0>8x}");
}

