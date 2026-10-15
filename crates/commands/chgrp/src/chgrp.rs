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
use ctcore::{Tool, libc};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::ct_display::{Quotable, locale_quote};
pub use ctcore::ct_entries;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_perms::{
    CtGidUidOwnerFilter, CtIfFrom, normalize_posixly_correct_args, opt_flags, ownership_change_base,
};

use std::ffi::OsString;

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};

use std::fs;
use std::os::unix::fs::MetadataExt;

/**
 * 根据命令行参数解析目标GID和UID。
 *
 * 此函数用于处理`chgrp`命令的相关参数，根据这些参数确定要更改的组ID（GID）和所有者信息。
 * 如果提供了引用文件参数，则从该文件的元数据中读取GID，并尝试将其转换为组名；
 * 如果没有提供引用文件，但提供了组名参数，则尝试将组名转换为GID。
 */
fn chgrp_parse_gid_and_uid(args_match: &ArgMatches) -> CTResult<CtGidUidOwnerFilter> {
    let filter = CtIfFrom::All;

    // 初始化用于存储原始组名或ID的变量
    let mut chgrp_raw_group: String = String::new();

    // 处理引用文件参数，如果存在的话
    let dest_gid = if let Some(file) = args_match
        .get_many::<String>(opt_flags::REFERENCE)
        .and_then(Iterator::last)
    {
        // 尝试从文件元数据中获取GID，并尝试将其转换为组名
        fs::metadata(file)
            .map(|meta| {
                let gid = meta.gid();
                chgrp_raw_group = ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string());
                Some(gid)
            })
            .map_err_context(|| format!("failed to get attributes of {}", file.quote()))?
    } else {
        // 处理组名参数
        let group_info = args_match
            .get_one::<String>(opt_flags::ARG_GROUP)
            .map(|s| s.as_str())
            .unwrap_or_default();
        chgrp_raw_group = group_info.to_string();
        if group_info.is_empty() {
            None
        } else {
            // 尝试将组名转换为GID，如果失败则尝试解析为数字GID
            match ct_entries::grp2gid(group_info) {
                Ok(g) => Some(g),
                _ => match group_info
                    .trim_start_matches([' ', '\t', '\n', '\r', '\u{000B}', '\u{000C}'])
                    .parse::<u32>()
                {
                    Ok(gid) => Some(gid),
                    Err(_) => {
                        return Err(CtSimpleError::new(
                            1,
                            format!("invalid group: {}", locale_quote(group_info)),
                        ));
                    }
                },
            }
        }
    };
    let dest_group_name = dest_gid.and_then(|_| {
        (!chgrp_raw_group.is_empty()
            && Group::locate(chgrp_raw_group.as_str())
                .is_ok_and(|entry| entry.name == chgrp_raw_group))
        .then(|| chgrp_raw_group.clone())
    });
    // 构造并返回`CtGidUidOwnerFilter`实例
    Ok(CtGidUidOwnerFilter {
        dest_gid,
        dest_uid: None,
        dest_user_name: None,
        dest_group_name,
        raw_owner: chgrp_raw_group,
        filter,
    })
}

#[derive(Default)]
pub struct Chgrp;
impl Tool for Chgrp {
    fn name(&self) -> &'static str {
        "chgrp"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        chgrp_main(args.iter().cloned())
    }
}

const GNU_LONG_OPTIONS: [&str; 12] = [
    "recursive",
    "changes",
    "dereference",
    "no-dereference",
    "no-preserve-root",
    "preserve-root",
    "quiet",
    "silent",
    "reference",
    "verbose",
    "help",
    "version",
];

fn gnu_long_option(option: &str) -> Option<&'static str> {
    if let Some(option) = GNU_LONG_OPTIONS.iter().find(|known| **known == option) {
        return Some(option);
    }

    let mut matches = GNU_LONG_OPTIONS
        .iter()
        .filter(|known| known.starts_with(option));
    let option = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(option)
    }
}

fn gnu_ambiguous_long_option_message(args: &[OsString]) -> Option<String> {
    let mut reference_takes_next = false;

    for arg in args.iter().skip(1) {
        let Some(arg) = arg.to_str() else {
            continue;
        };
        if reference_takes_next {
            reference_takes_next = false;
            continue;
        }
        if arg == "--" {
            break;
        }
        let Some(long_option) = arg.strip_prefix("--") else {
            continue;
        };
        if long_option.is_empty() {
            break;
        }
        let (option, value) = long_option
            .split_once('=')
            .map_or((long_option, None), |(name, value)| (name, Some(value)));
        let matching_options: Vec<_> = GNU_LONG_OPTIONS
            .iter()
            .filter(|known| known.starts_with(option))
            .copied()
            .collect();

        if matching_options.len() > 1 {
            let possibilities = matching_options
                .iter()
                .map(|option| locale_quote(&format!("--{option}")))
                .collect::<Vec<_>>()
                .join(" ");
            return Some(format!(
                "option {} is ambiguous; possibilities: {possibilities}",
                locale_quote(arg)
            ));
        }

        let Some(option) = matching_options.first() else {
            // Let the shared parser report the first preceding unknown option.
            return None;
        };
        if value.is_some() && *option != opt_flags::REFERENCE {
            // The shared parser produces GNU's no-argument diagnostic first.
            return None;
        }
        if *option == opt_flags::REFERENCE && value.is_none() {
            reference_takes_next = true;
        }
    }

    None
}

fn gnu_terminal_option_requested(args: &[OsString], terminal_option: &str) -> bool {
    let mut reference_takes_next = false;

    for arg in args.iter().skip(1) {
        let Some(arg) = arg.to_str() else {
            continue;
        };
        if reference_takes_next {
            reference_takes_next = false;
            continue;
        }
        if arg == "--" {
            break;
        }
        if let Some(long_option) = arg.strip_prefix("--") {
            if long_option.is_empty() {
                break;
            }
            let (option, value) = long_option
                .split_once('=')
                .map_or((long_option, None), |(name, value)| (name, Some(value)));
            let Some(option) = gnu_long_option(option) else {
                return false;
            };
            if value.is_some() && option != opt_flags::REFERENCE {
                return false;
            }
            if option == opt_flags::REFERENCE && value.is_none() {
                reference_takes_next = true;
            } else if option == opt_flags::HELP || option == "version" {
                return option == terminal_option;
            }
        } else if let Some(short_options) = arg.strip_prefix('-')
            && !short_options
                .chars()
                .all(|option| matches!(option, 'H' | 'L' | 'P' | 'R' | 'c' | 'f' | 'h' | 'v'))
        {
            return false;
        }
    }
    false
}

const SYSKITS_HELP_TEMPLATE: &str = r#"Usage: {program} [OPTION]... GROUP FILE...
  or:  {program} [OPTION]... --reference=RFILE FILE...
Change the group of each FILE to GROUP.
With --reference, change the group of each FILE to that of RFILE.

  -c, --changes          like verbose but report only when a change is made
  -f, --silent, --quiet  suppress most error messages
  -v, --verbose          output a diagnostic for every file processed
      --dereference      affect the referent of each symbolic link (this is
                         the default), rather than the symbolic link itself
  -h, --no-dereference   affect symbolic links instead of any referenced file
                         (useful only on systems that can change the
                         ownership of a symlink)
      --no-preserve-root  do not treat '/' specially (the default)
      --preserve-root    fail to operate recursively on '/'
      --reference=RFILE  use RFILE's group rather than specifying a GROUP.
                         RFILE is always dereferenced if a symbolic link.
  -R, --recursive        operate on files and directories recursively

The following options modify how a hierarchy is traversed when the -R
option is also specified.  If more than one is specified, only the final
one takes effect.

  -H                     if a command line argument is a symbolic link
                         to a directory, traverse it
  -L                     traverse every symbolic link to a directory
                         encountered
  -P                     do not traverse any symbolic links (default)

      --help        display this help and exit
      --version     output version information and exit

Examples:
  {program} staff /u      Change the group of /u to "staff".
  {program} -hR staff /u  Change the group of /u and subfiles to "staff".

syskits online help: <https://www.ctyun.cn/developer>
"#;

fn syskits_help_output(program: &str) -> String {
    SYSKITS_HELP_TEMPLATE.replace("{program}", program)
}

const SYSKITS_VERSION_TEMPLATE: &str = r#"{program} (syskits) {version}
Copyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.
License Mulan PSL v2: <http://license.coscl.org.cn/MulanPSL2>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.
"#;

fn syskits_version_output(program: &str) -> String {
    SYSKITS_VERSION_TEMPLATE
        .replace("{program}", program)
        .replace("{version}", crate_version!())
}

pub fn chgrp_main(args: impl ctcore::Args) -> CTResult<()> {
    let args = args.collect();
    let lang_code = sys_locale::get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let command = ct_app();
    let args = normalize_posixly_correct_args(&command, args);
    if gnu_terminal_option_requested(&args, opt_flags::HELP) {
        print!("{}", syskits_help_output(ctcore::ct_util_name()));
        return Ok(());
    }
    if gnu_terminal_option_requested(&args, "version") {
        print!("{}", syskits_version_output(ctcore::ct_util_name()));
        return Ok(());
    }
    if let Some(message) = gnu_ambiguous_long_option_message(&args) {
        return Err(CTsageError::new(libc::EXIT_FAILURE, message));
    }
    ownership_change_base(
        command,
        args.into_iter(),
        opt_flags::ARG_GROUP,
        chgrp_parse_gid_and_uid,
        true,
    )
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("chgrp.about");
    let usage_description = t!("chgrp.usage");

    let args = vec![
        Arg::new(opt_flags::HELP)
            .long(opt_flags::HELP)
            .help("Print help information.")
            .action(ArgAction::Help),
        Arg::new("version")
            .long("version")
            .action(ArgAction::Version),
        Arg::new(opt_flags::verbosity::CHANGES)
            .short('c')
            .long(opt_flags::verbosity::CHANGES)
            .overrides_with(opt_flags::verbosity::VERBOSE)
            .help("like verbose but report only when a change is made")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::verbosity::SILENT)
            .short('f')
            .long(opt_flags::verbosity::SILENT)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::verbosity::QUIET)
            .long(opt_flags::verbosity::QUIET)
            .help("suppress most error messages")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::verbosity::VERBOSE)
            .short('v')
            .long(opt_flags::verbosity::VERBOSE)
            .overrides_with(opt_flags::verbosity::CHANGES)
            .help("output a diagnostic for every file processed")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::dereference::DEREFERENCE)
            .long(opt_flags::dereference::DEREFERENCE)
            .overrides_with(opt_flags::dereference::NO_DEREFERENCE)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::dereference::NO_DEREFERENCE)
            .short('h')
            .long(opt_flags::dereference::NO_DEREFERENCE)
            .overrides_with(opt_flags::dereference::DEREFERENCE)
            .help(
                "affect symbolic links instead of any referenced file (useful only on systems that can change the ownership of a symlink)",
            )
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::preserve_root::PRESERVE)
            .long(opt_flags::preserve_root::PRESERVE)
            .overrides_with(opt_flags::preserve_root::NO_PRESERVE)
            .help("fail to operate recursively on '/'")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::preserve_root::NO_PRESERVE)
            .long(opt_flags::preserve_root::NO_PRESERVE)
            .overrides_with(opt_flags::preserve_root::PRESERVE)
            .help("do not treat '/' specially (the default)")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::REFERENCE)
            .long(opt_flags::REFERENCE)
            .value_name("RFILE")
            .value_hint(clap::ValueHint::FilePath)
            .help("use RFILE's group rather than specifying GROUP values")
            .num_args(1)
            .action(ArgAction::Append),
        Arg::new(opt_flags::RECURSIVE)
            .short('R')
            .long(opt_flags::RECURSIVE)
            .help("operate on files and directories recursively")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::traverse::TRAVERSE)
            .short(opt_flags::traverse::TRAVERSE.chars().next().unwrap())
            .help("if a command line argument is a symbolic link to a directory, traverse it")
            .overrides_with_all([opt_flags::traverse::NO_TRAVERSE, opt_flags::traverse::EVERY])
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::traverse::NO_TRAVERSE)
            .short(opt_flags::traverse::NO_TRAVERSE.chars().next().unwrap())
            .help("do not traverse any symbolic links (default)")
            .overrides_with_all([opt_flags::traverse::TRAVERSE, opt_flags::traverse::EVERY])
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::traverse::EVERY)
            .short(opt_flags::traverse::EVERY.chars().next().unwrap())
            .help("traverse every symbolic link to a directory encountered")
            .overrides_with_all([opt_flags::traverse::TRAVERSE, opt_flags::traverse::NO_TRAVERSE])
            .action(ArgAction::SetTrue),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;
    use std::fs::File;

    use std::io::Write;

    #[test]
    fn test_tool_implementation() {
        let chgrp = Chgrp;

        // Test name method
        assert_eq!(chgrp.name(), "chgrp");

        // Test command method
        assert!(chgrp.command().get_name().contains("chgrp"));

        // Test execute method - should fail with no arguments, which is expected
        let args: Vec<OsString> = vec![OsString::from("chgrp")];
        let result = chgrp.execute(&args);
        assert!(result.is_err());

        // Test execute with --help, which should succeed
        let help_args: Vec<OsString> = vec![OsString::from("chgrp"), OsString::from("--help")];
        let help_result = chgrp.execute(&help_args);
        assert!(help_result.is_ok());
    }

    #[test]
    fn test_ct_app_execution_help() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "--help"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "--version"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_rejects_short_version_option() {
        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), "-V"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_rejects_from_option() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--from=0:0"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_execution_dereference_true() {
        let command = ct_app();

        // 测试用例：有效输入 --dereference
        let args = vec![ctcore::ct_util_name(), "--dereference"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::dereference::DEREFERENCE));
    }

    #[test]
    fn test_ct_app_execution_dereference_false() {
        let command = ct_app();

        // 测试用例：有效输入 --no-dereference
        let args = vec![ctcore::ct_util_name(), "-h"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::dereference::NO_DEREFERENCE));
        assert!(!matches.get_flag(opt_flags::dereference::DEREFERENCE));
    }

    #[test]
    fn test_ct_app_execution_dereference_whole_false() {
        let command = ct_app();

        // 测试用例：有效输入 --no-dereference
        let args = vec![ctcore::ct_util_name(), "--no-dereference"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::dereference::NO_DEREFERENCE));
        assert!(!matches.get_flag(opt_flags::dereference::DEREFERENCE));
    }

    #[test]
    fn test_ct_app_execution_preserve_root_true() {
        let command = ct_app();

        // 测试用例：有效输入 --preserve-root
        let args = vec![ctcore::ct_util_name(), "--preserve-root"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::preserve_root::PRESERVE));
    }

    #[test]
    fn test_ct_app_execution_preserve_root_false() {
        let command = ct_app();

        // 测试用例：有效输入 --no-preserve-root
        let args = vec![ctcore::ct_util_name(), "--no-preserve-root"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::preserve_root::NO_PRESERVE));
        assert!(!matches.get_flag(opt_flags::preserve_root::PRESERVE));
    }

    #[test]
    fn test_last_preserve_root_option_wins() {
        let no_preserve_last = ct_app()
            .try_get_matches_from([
                ctcore::ct_util_name(),
                "--preserve-root",
                "--no-preserve-root",
            ])
            .expect("parse preserve-root options");
        assert!(!no_preserve_last.get_flag(opt_flags::preserve_root::PRESERVE));
        assert!(no_preserve_last.get_flag(opt_flags::preserve_root::NO_PRESERVE));

        let preserve_last = ct_app()
            .try_get_matches_from([
                ctcore::ct_util_name(),
                "--no-preserve-root",
                "--preserve-root",
            ])
            .expect("parse preserve-root options");
        assert!(preserve_last.get_flag(opt_flags::preserve_root::PRESERVE));
        assert!(!preserve_last.get_flag(opt_flags::preserve_root::NO_PRESERVE));
    }

    #[test]
    fn test_changes_and_verbose_options_override_each_other() {
        let verbose_last = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "-c", "-v"])
            .expect("parse verbosity options");
        assert!(!verbose_last.get_flag(opt_flags::verbosity::CHANGES));
        assert!(verbose_last.get_flag(opt_flags::verbosity::VERBOSE));

        let quiet_and_verbose = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "-v", "--quiet"])
            .expect("parse verbosity options");
        assert!(quiet_and_verbose.get_flag(opt_flags::verbosity::VERBOSE));
        assert!(quiet_and_verbose.get_flag(opt_flags::verbosity::QUIET));
    }

    #[test]
    fn test_ct_app_execution_recursive() {
        let command = ct_app();

        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "-R"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::RECURSIVE));
    }

    #[test]
    fn test_ct_app_execution_recursive_whole() {
        let command = ct_app();

        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "--recursive"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::RECURSIVE));
    }

    #[test]
    fn test_version_ctmain() {
        let args = [ctcore::ct_util_name(), "--version"];
        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_ok());
    }

    #[test]
    fn test_help_ctmain() {
        let args = [ctcore::ct_util_name(), "--help"];
        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_ok());
    }

    #[test]
    fn test_help_invalid_ctmain() {
        let args = [ctcore::ct_util_name(), "-H"];
        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_short_version_ctmain_is_invalid() {
        let args = [ctcore::ct_util_name(), "-V"];
        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_dereference_true_ctmain() {
        let args = [ctcore::ct_util_name(), "--dereference"];
        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_dereference_false_ctmain() {
        let args = [ctcore::ct_util_name(), "-h"];
        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_dereference_whole_false_ctmain() {
        // 测试用例：有效输入 --no-dereference
        let args = [ctcore::ct_util_name(), "--no-dereference"];

        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_preserve_root_true_ctmain() {
        // 测试用例：有效输入 --preserve-root
        let args = [ctcore::ct_util_name(), "--preserve-root"];

        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_preserve_root_false_ctmain() {
        // 测试用例：有效输入 --no-preserve-root
        let args = [ctcore::ct_util_name(), "--no-preserve-root"];

        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_recursive_ctmain() {
        // 测试用例：有效输入 --recursive
        let args = [ctcore::ct_util_name(), "-R"];

        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_recursive_whole_ctmain() {
        // 测试用例：有效输入 --recursive
        let args = [ctcore::ct_util_name(), "--recursive"];

        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_verbose_ctmain() {
        // 测试用例：有效输入 --verbose
        let args = [ctcore::ct_util_name(), "-v"];

        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_verbose_whole_ctmain() {
        // 测试用例：有效输入 --verbose
        let args = [ctcore::ct_util_name(), "--verbose"];

        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_chgrp_invalid_user_id_ctmain() {
        let dir_path = "test_chgrp_invalid_user_id_ctmain";
        let subdir_name = "subdirectory";
        let file_name = "test_chcon_invalid_user_id.txt";

        // 创建二级目录
        let subdir_path = format!("{dir_path}/{subdir_name}");
        fs::create_dir_all(&subdir_path).expect("Failed to create directory");

        // 创建文件路径
        let file_path = format!("{subdir_path}/{file_name}");

        // 创建文件并写入内容
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{file_path}' created successfully.");

        let args = [ctcore::ct_util_name(), "-R", "invalid_user_id", dir_path];

        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err()); // Expect a non-zero exit code for invalid user ID
        // Remove the directory hierarchy
        fs::remove_dir_all(dir_path).expect("Failed to delete directory");
    }
    #[test]
    fn test_chgrp_h_r_ctmain() {
        let dir_path = "test_chgrp_h_r_ctmain";
        let subdir_name = "subdirectory";
        let file_name = "test_chcon_invalid_user_id.txt";

        // 创建二级目录
        let subdir_path = format!("{dir_path}/{subdir_name}");
        fs::create_dir_all(&subdir_path).expect("Failed to create directory");

        // 创建文件路径
        let file_path = format!("{subdir_path}/{file_name}");

        // 创建文件并写入内容
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{file_path}' created successfully.");

        let args = [
            ctcore::ct_util_name(),
            "-h",
            "-R",
            "invalid_user_id",
            dir_path,
        ];

        let result = chgrp_main(args.iter().map(OsString::from));
        assert!(result.is_err()); // Expect a non-zero exit code for invalid user ID
        // Remove the directory hierarchy
        fs::remove_dir_all(dir_path).expect("Failed to delete directory");
    }
}
