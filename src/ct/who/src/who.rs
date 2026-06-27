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

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_error::CTResult;

#[ctcore::main]
use platform::ctmain;
use std::ffi::OsString;
use sys_locale::get_locale;

use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");

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
    pub const WHO_FILE: &str = "FILE"; // if length=1: FILE, if length=2: ARG1 ARG2
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("who.about");
    let usage_description = t!("who.usage");
    let args = vec![
        Arg::new(who_flags::WHO_ALL)
            .long(who_flags::WHO_ALL)
            .short('a')
            .help(t!("who.clap.options.all"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_BOOT)
            .long(who_flags::WHO_BOOT)
            .short('b')
            .help(t!("who.clap.options.boot"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_DEAD)
            .long(who_flags::WHO_DEAD)
            .short('d')
            .help(t!("who.clap.options.dead"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_HEADING)
            .long(who_flags::WHO_HEADING)
            .short('H')
            .help(t!("who.clap.options.heading"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_LOGIN)
            .long(who_flags::WHO_LOGIN)
            .short('l')
            .help(t!("who.clap.options.login"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_LOOKUP)
            .long(who_flags::WHO_LOOKUP)
            .help(t!("who.clap.options.lookup"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_ONLY_HOSTNAME_USER)
            .short('m')
            .help(t!("who.clap.options.only_hostname_user"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_PROCESS)
            .long(who_flags::WHO_PROCESS)
            .short('p')
            .help(t!("who.clap.options.process"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_COUNT)
            .long(who_flags::WHO_COUNT)
            .short('q')
            .help(t!("who.clap.options.count"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_RUNLEVEL)
            .long(who_flags::WHO_RUNLEVEL)
            .short('r')
            .help(t!("who.clap.options.runlevel"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_SHORT)
            .long(who_flags::WHO_SHORT)
            .short('s')
            .help(t!("who.clap.options.short"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_TIME)
            .long(who_flags::WHO_TIME)
            .short('t')
            .help(t!("who.clap.options.time"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_USERS)
            .long(who_flags::WHO_USERS)
            .short('u')
            .help(t!("who.clap.options.users"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_MESG)
            .long(who_flags::WHO_MESG)
            .short('T')
            .visible_short_alias('w')
            .visible_aliases(["message", "writable"])
            .help(t!("who.clap.options.mesg"))
            .action(ArgAction::SetTrue),
        Arg::new(who_flags::WHO_FILE)
            .num_args(1..=2)
            .value_hint(clap::ValueHint::FilePath)
            .help(t!("who.clap.options.file")),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .help(t!("who.clap.help"))
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .help(t!("who.clap.version"))
                .action(ArgAction::Version),
        )
        .args(&args)
}

#[derive(Default)]
pub struct Who;
impl Tool for Who {
    fn name(&self) -> &'static str {
        "who"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // Set locale based on system settings
        let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
        rust_i18n::set_locale(&lang_code);

        platform::who_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use ctcore::Tool;
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn test_tool_implementation() {
        let tool = Who::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "who");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("who"));

        // 测试 execute 方法
        let args = vec![OsString::from("who"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    // who 接口: who [OPTION]... [ FILE | ARG1 ARG2 ]
    //   -a, --all         same as -b -d --login -p -r -t -T -u
    //   -b, --boot        time of last system boot
    //   -d, --dead        print dead processes
    //   -H, --heading     print line of column headings
    //   -l, --login       print system login processes
    //       --lookup      attempt to canonicalize hostnames via DNS
    //   -m                only hostname and user associated with stdin
    //   -p, --process     print active processes spawned by init
    //   -q, --count       all login names and number of users logged on
    //   -r, --runlevel    print current runlevel
    //   -s, --short       print only name, line, and time (default)
    //   -t, --time        print last system clock change
    //   -T, -w, --mesg    add user's message status as +, - or ?
    //   -u, --users       list users logged in
    //       --message     same as -T
    //       --writable    same as -T
    //       --help     display this help and exit
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
        assert!(result.is_ok());
        // assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
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
    fn test_ct_app_long_option_all() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--all"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_boot() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--boot"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_dead() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--dead"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_heading() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--heading"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_login() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--login"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_lookup() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--lookup"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_process() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--process"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_count() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--count"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_runlevel() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--runlevel"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_short() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--short"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_time() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--time"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_users() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--users"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_mesg() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--mesg"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_file() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--file"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_long_option_file2() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--file"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_short_option_a() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-a"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_b() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-b"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_d() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-d"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_uppercase_h() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-H"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_l() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-l"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_m() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-m"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_p() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-p"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_q() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-q"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_r() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-r"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_s() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-s"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_t() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-t"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_u() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-u"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_uppercase_t() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-T"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_w() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-w"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }
}
