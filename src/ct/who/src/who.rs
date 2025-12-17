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

use clap::{crate_version, Arg, ArgAction, Command};

use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
#[ctcore::main]
use platform::ctmain;

mod platform;

mod who_flags {
    pub const WHO_ALL: &str = "all";
    pub const WHO_BOOT: &str = "boot";
    pub const WHO_DEAD: &str = "dead";
    pub const WHO_HEADING: &str = "heading";
    pub const WHO_LOGIN: &str = "login";
    pub const WHO_LOOKUP: &str = "lookup";
    pub const WHO_ONLY_HOSTNAME_USER: &str = "only_hostname_user";
    pub const WHO_PROCESS: &str = "process";
    pub const WHO_COUNT: &str = "count";
    pub const WHO_RUNLEVEL: &str = "runlevel";
    pub const WHO_SHORT: &str = "short";
    pub const WHO_TIME: &str = "time";
    pub const WHO_USERS: &str = "users";
    pub const WHO_MESG: &str = "mesg";
    // aliases: --message, --writable
    pub const WHO_FILE: &str = "FILE"; // if length=1: FILE, if length=2: ARG1 ARG2
}

const WHO_ABOUT: &str = ct_help_about!("who.md");
const WHO_USAGE: &str = ct_help_usage!("who.md");

#[cfg(target_os = "linux")]
static WHO_RUNLEVEL_HELP: &str = "print current runlevel";
#[cfg(not(target_os = "linux"))]
static WHO_RUNLEVEL_HELP: &str = "print current runlevel (This is meaningless on non Linux)";

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = WHO_ABOUT;
    let usage_description = ct_format_usage(WHO_USAGE);
    let args = vec![
        Arg::new(who_flags::WHO_ALL)
            .long(who_flags::WHO_ALL)
            .short('a')
            .help("same as -b -d --login -p -r -t -T -u")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_BOOT)
            .long(who_flags::WHO_BOOT)
            .short('b')
            .help("time of last system boot")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_DEAD)
            .long(who_flags::WHO_DEAD)
            .short('d')
            .help("print dead processes")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_HEADING)
            .long(who_flags::WHO_HEADING)
            .short('H')
            .help("print line of column headings")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_LOGIN)
            .long(who_flags::WHO_LOGIN)
            .short('l')
            .help("print system login processes")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_LOOKUP)
            .long(who_flags::WHO_LOOKUP)
            .help("attempt to canonicalize hostnames via DNS")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_ONLY_HOSTNAME_USER)
            .short('m')
            .help("only hostname and user associated with stdin")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_PROCESS)
            .long(who_flags::WHO_PROCESS)
            .short('p')
            .help("print active processes spawned by init")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_COUNT)
            .long(who_flags::WHO_COUNT)
            .short('q')
            .help("all login names and number of users logged on")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_RUNLEVEL)
            .long(who_flags::WHO_RUNLEVEL)
            .short('r')
            .help(WHO_RUNLEVEL_HELP)
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_SHORT)
            .long(who_flags::WHO_SHORT)
            .short('s')
            .help("print only name, line, and time (default)")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_TIME)
            .long(who_flags::WHO_TIME)
            .short('t')
            .help("print last system clock change")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_USERS)
            .long(who_flags::WHO_USERS)
            .short('u')
            .help("list users logged in")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_MESG)
            .long(who_flags::WHO_MESG)
            .short('T')
            .visible_short_alias('w')
            .visible_aliases(["message", "writable"])
            .help("add user's message status as +, - or ?")
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_FILE)
            .num_args(1..=2)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

