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

//! sync 命令在 Linux 中用于确保系统内存中的数据被立即写入到硬盘中，防止数据丢失。
/* synced with: sync (GNU coreutils) 8.13 */

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");

use ctcore::Tool;
use ctcore::ct_error::{CTResult, CtSimpleError};

use std::ffi::OsString;
use sys_locale::get_locale;

mod platform;

pub mod sync_flags {
    pub const SYNC_FILE_SYSTEM: &str = "file-system";
    pub const SYNC_DATA: &str = "data";
}

const SYNC_ARG_FILES: &str = "files";

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    sync_main(args)
}

pub fn sync_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_matches = ct_app().try_get_matches_from(args)?;
    let is_has_data = arg_matches.get_flag(sync_flags::SYNC_DATA);
    let is_file_system = arg_matches.get_flag(sync_flags::SYNC_FILE_SYSTEM);
    let files: Vec<String> = arg_matches
        .get_many::<String>(SYNC_ARG_FILES)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    if is_has_data && files.is_empty() {
        let err_message = "--data needs at least one argument";
        return Err(CtSimpleError::new(1, err_message));
    }

    for f in &files {
        check_files(f)?;
    }

    if is_file_system {
        sync_fs(files);
    } else if is_has_data {
        #[cfg(target_os = "linux")]
        platform::fdatasync(files);
    } else {
        sync();
    }
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("sync.about");
    let usage_description = t!("sync.usage");
    let args = vec![
        Arg::new(sync_flags::SYNC_FILE_SYSTEM)
            .short('f')
            .long(sync_flags::SYNC_FILE_SYSTEM)
            .conflicts_with(sync_flags::SYNC_DATA)
            .help(t!("sync.clap.sync_file_system"))
            .action(ArgAction::SetTrue),
        Arg::new(sync_flags::SYNC_DATA)
            .short('d')
            .long(sync_flags::SYNC_DATA)
            .conflicts_with(sync_flags::SYNC_FILE_SYSTEM)
            .help(t!("sync.clap.sync_data"))
            .action(ArgAction::SetTrue),
        Arg::new(SYNC_ARG_FILES)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
}

fn sync() -> isize {
    unsafe { platform::do_sync() }
}

fn sync_fs(files: Vec<String>) -> isize {
    unsafe { platform::do_syncfs(files) }
}

fn check_files(f: &String) -> CTResult<()> {
    platform::check_files(f)
}

#[derive(Default)]
pub struct Sync;
impl Tool for Sync {
    fn name(&self) -> &'static str {
        "sync"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        sync_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Sync::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "sync");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("sync"));

        // 测试 execute 方法
        let args = vec![OsString::from("sync"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::ffi::OsString;
        use std::fs::File;
        use std::io::Write;

        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_ct_main_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = sync_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_help_short() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_system_long() {
            let args = vec![ctcore::ct_util_name(), "--file-system"];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_system_short() {
            let args = vec![ctcore::ct_util_name(), "-f"];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_data_long() {
            let args = vec![ctcore::ct_util_name(), "--data"];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "--data needs at least one argument".to_string()
            )
        }

        #[test]
        fn test_ct_main_file_data_short() {
            let args = vec![ctcore::ct_util_name(), "-d"];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "--data needs at least one argument".to_string()
            )
        }

        #[test]
        fn test_ct_main_with_dir() {
            let dir = tempdir().unwrap();
            let dir_name = dir.path().to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), dir_name];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_system_long_with_file() {
            let filename = "test_ct_main_file_system_long_with_file";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b c\nc d").unwrap();

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--file-system", file_name];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_system_short_with_file() {
            let filename = "test_ct_main_file_system_short_with_file";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b c\nc d").unwrap();

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-f", file_name];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_data_long_with_file() {
            let filename = "test_ct_main_file_data_long_with_file";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b c\nc d").unwrap();

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--data", file_name];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_data_short_with_file() {
            let filename = "test_ct_main_file_data_short_with_file";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b c\nc d").unwrap();

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-d", file_name];
            let result = sync_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // sync 接口: sync [OPTION]... FILE...
        //
        // Arguments:
        //   [files]...
        //
        // Options:
        //   -f, --file-system  sync the file systems that contain the files
        //   -d, --data         sync only file data, no unneeded metadata (Linux only)
        //   -h, --help         Print help
        //   -V, --version      Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
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
            let missing_args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(missing_args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_system_long() {
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name(), "--file-system"];
            let result = command.try_get_matches_from(missing_args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_system_short() {
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name(), "-f"];
            let result = command.try_get_matches_from(missing_args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_data_long() {
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name(), "--data"];
            let result = command.try_get_matches_from(missing_args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_data_short() {
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name(), "-d"];
            let result = command.try_get_matches_from(missing_args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_system_long_with_file() {
            let filename = "test_ct_app_file_system_long_with_file";
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name(), "--file-system", filename];
            let result = command.try_get_matches_from(missing_args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_system_short_with_file() {
            let filename = "test_ct_app_file_system_short_with_file";
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name(), "-f", filename];
            let result = command.try_get_matches_from(missing_args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_data_long_with_file() {
            let filename = "test_ct_app_file_data_long_with_file";
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name(), "--data", filename];
            let result = command.try_get_matches_from(missing_args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_file_data_short_with_file() {
            let filename = "test_ct_app_file_data_short_with_file";
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name(), "-d", filename];
            let result = command.try_get_matches_from(missing_args);

            assert!(result.is_ok());
        }
    }
}
