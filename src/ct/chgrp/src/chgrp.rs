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
use ctcore::Tool;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::ct_display::Quotable;
pub use ctcore::ct_entries;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::ct_perms::{CtGidUidOwnerFilter, CtIfFrom, chown_base, opt_flags};

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
    // 初始化用于存储原始组名或ID的变量
    let mut chgrp_raw_group: String = String::new();

    // 处理引用文件参数，如果存在的话
    let dest_gid = if let Some(file) = args_match.get_one::<String>(opt_flags::REFERENCE) {
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
            // 尝试将组名转换为GID，如果失败则返回错误
            match ct_entries::grp2gid(group_info) {
                Ok(g) => Some(g),
                _ => {
                    return Err(CtSimpleError::new(
                        1,
                        format!("invalid group: {}", group_info.quote()),
                    ));
                }
            }
        }
    };
    // 构造并返回`CtGidUidOwnerFilter`实例
    Ok(CtGidUidOwnerFilter {
        dest_gid,
        dest_uid: None,
        raw_owner: chgrp_raw_group,
        filter: CtIfFrom::All,
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

pub fn chgrp_main(args: impl ctcore::Args) -> CTResult<()> {
    chown_base(
        ct_app(),
        args,
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
        Arg::new(opt_flags::verbosity::CHANGES)
            .short('c')
            .long(opt_flags::verbosity::CHANGES)
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
            .help("output a diagnostic for every file processed")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::dereference::DEREFERENCE)
            .long(opt_flags::dereference::DEREFERENCE)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::dereference::NO_DEREFERENCE)
            .short('h')
            .long(opt_flags::dereference::NO_DEREFERENCE)
            .help(
                "affect symbolic links instead of any referenced file (useful only on systems that can change the ownership of a symlink)",
            )
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::preserve_root::PRESERVE)
            .long(opt_flags::preserve_root::PRESERVE)
            .help("fail to operate recursively on '/'")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::preserve_root::NO_PRESERVE)
            .long(opt_flags::preserve_root::NO_PRESERVE)
            .help("do not treat '/' specially (the default)")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::REFERENCE)
            .long(opt_flags::REFERENCE)
            .value_name("RFILE")
            .value_hint(clap::ValueHint::FilePath)
            .help("use RFILE's group rather than specifying GROUP values"),
        Arg::new(opt_flags::RECURSIVE)
            .short('R')
            .long(opt_flags::RECURSIVE)
            .help("operate on files and directories recursively")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::traverse::TRAVERSE)
            .short(opt_flags::traverse::TRAVERSE.chars().next().unwrap())
            .help("if a command line argument is a symbolic link to a directory, traverse it")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::traverse::NO_TRAVERSE)
            .short(opt_flags::traverse::NO_TRAVERSE.chars().next().unwrap())
            .help("do not traverse any symbolic links (default)")
            .overrides_with_all([opt_flags::traverse::TRAVERSE, opt_flags::traverse::EVERY])
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::traverse::EVERY)
            .short(opt_flags::traverse::EVERY.chars().next().unwrap())
            .help("traverse every symbolic link to a directory encountered")
            .action(ArgAction::SetTrue),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
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
        let chgrp = Chgrp::default();

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
        assert!(help_result.is_err());
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
    fn test_ct_app_execution_version_valid() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "-V"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
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
        let args = vec![ctcore::ct_util_name(), "--version"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 0);
    }

    #[test]
    fn test_help_ctmain() {
        let args = vec![ctcore::ct_util_name(), "--help"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 0);
    }

    #[test]
    fn test_help_invalid_ctmain() {
        let args = vec![ctcore::ct_util_name(), "-H"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_version_valid_ctmain() {
        let args = vec![ctcore::ct_util_name(), "-V"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 0);
    }

    #[test]
    fn test_dereference_true_ctmain() {
        let args = vec![ctcore::ct_util_name(), "--dereference"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_dereference_false_ctmain() {
        let args = vec![ctcore::ct_util_name(), "-h"];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_dereference_whole_false_ctmain() {
        // 测试用例：有效输入 --no-dereference
        let args = vec![ctcore::ct_util_name(), "--no-dereference"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_preserve_root_true_ctmain() {
        // 测试用例：有效输入 --preserve-root
        let args = vec![ctcore::ct_util_name(), "--preserve-root"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_preserve_root_false_ctmain() {
        // 测试用例：有效输入 --no-preserve-root
        let args = vec![ctcore::ct_util_name(), "--no-preserve-root"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_recursive_ctmain() {
        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "-R"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_recursive_whole_ctmain() {
        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "--recursive"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_verbose_ctmain() {
        // 测试用例：有效输入 --verbose
        let args = vec![ctcore::ct_util_name(), "-v"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_verbose_whole_ctmain() {
        // 测试用例：有效输入 --verbose
        let args = vec![ctcore::ct_util_name(), "--verbose"];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_eq!(result, 1);
    }

    #[test]
    fn test_chgrp_invalid_user_id_ctmain() {
        let dir_path = "test_chgrp_invalid_user_id_ctmain";
        let subdir_name = "subdirectory";
        let file_name = "test_chcon_invalid_user_id.txt";

        // 创建二级目录
        let subdir_path = format!("{}/{}", dir_path, subdir_name);
        fs::create_dir_all(&subdir_path).expect("Failed to create directory");

        // 创建文件路径
        let file_path = format!("{}/{}", subdir_path, file_name);

        // 创建文件并写入内容
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{}' created successfully.", file_path);

        let args = vec![ctcore::ct_util_name(), "-R", "invalid_user_id", dir_path];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_ne!(result, 0); // Expect a non-zero exit code for invalid user ID
        // Remove the directory hierarchy
        fs::remove_dir_all(dir_path).expect("Failed to delete directory");
    }
    #[test]
    fn test_chgrp_h_r_ctmain() {
        let dir_path = "test_chgrp_h_r_ctmain";
        let subdir_name = "subdirectory";
        let file_name = "test_chcon_invalid_user_id.txt";

        // 创建二级目录
        let subdir_path = format!("{}/{}", dir_path, subdir_name);
        fs::create_dir_all(&subdir_path).expect("Failed to create directory");

        // 创建文件路径
        let file_path = format!("{}/{}", subdir_path, file_name);

        // 创建文件并写入内容
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{}' created successfully.", file_path);

        let args = vec![
            ctcore::ct_util_name(),
            "-h",
            "-R",
            "invalid_user_id",
            dir_path,
        ];

        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        assert_ne!(result, 0); // Expect a non-zero exit code for invalid user ID
        // Remove the directory hierarchy
        fs::remove_dir_all(dir_path).expect("Failed to delete directory");
    }
}
