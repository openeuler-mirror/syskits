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
use clap::{Arg, ArgMatches, Command, crate_version};

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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::fs;
        use std::fs::File;
        use std::io::Write;
        use tempfile::TempDir;

        #[repr(C)]
        #[derive(Debug)]
        struct UtmpRecord {
            ut_type: u16,
            ut_pid: i32,
            ut_line: [u8; 32],
            ut_id: [u8; 4],
            ut_user: [u8; 32],
            ut_host: [u8; 256],
            ut_exit: [i32; 2],
            ut_session: i32,
            ut_tv: [i32; 2],
            ut_addr_v6: [i32; 4],
            __unused: [u8; 20], // To match the size of C struct
        }

        impl UtmpRecord {
            fn new(username: &str, terminal: &str, hostname: &str) -> Self {
                let mut ut_line = [0; 32];
                ut_line[..terminal.len()].copy_from_slice(terminal.as_bytes());

                let mut ut_user = [0; 32];
                ut_user[..username.len()].copy_from_slice(username.as_bytes());

                let mut ut_host = [0; 256];
                ut_host[..hostname.len()].copy_from_slice(hostname.as_bytes());

                UtmpRecord {
                    ut_type: 7, // USER_PROCESS
                    ut_pid: 0,
                    ut_line,
                    ut_id: [0; 4],
                    ut_user,
                    ut_host,
                    ut_exit: [0; 2],
                    ut_session: 0,
                    ut_tv: [0; 2],
                    ut_addr_v6: [0; 4],
                    __unused: [0; 20],
                }
            }
        }

        #[test]
        fn test_users_main_argument_parsing_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();

            let users = vec![
                ("user1", "tty1", "localhost"),
                ("user2", "tty2", "localhost"),
                ("user3", "tty3", "localhost"),
            ];
            for (username, terminal, hostname) in users {
                let record = UtmpRecord::new(username, terminal, hostname);
                let record_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        &record as *const _ as *const u8,
                        std::mem::size_of::<UtmpRecord>(),
                    )
                };
                tmp_file.write_all(record_bytes).unwrap();
            }

            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), file_name];
            let result = users_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), "user1");
        }

        #[test]
        fn test_users_main_argument_parsing_utmp_file() {
            let source = "/var/run/utmp";
            let source_path = PathBuf::from(source);
            if source_path.exists() {
                let destination = "./users_main_utmp_test";
                std::fs::copy(source, destination).unwrap();

                let args = vec![ctcore::ct_util_name(), destination];
                let result = users_main(args.iter().map(|s| OsString::from(s)));

                assert!(result.is_ok());

                fs::remove_file(destination).expect("Failed to remove file");
            } else {
                println!("no exist {}", source);
            }
        }

        #[test]
        fn test_users_main_argument_parsing_wtmp_file() {
            let source = "/var/log/wtmp";
            let source_path = PathBuf::from(source);
            if source_path.exists() {
                let destination = "./users_main_wtmp_test";

                std::fs::copy(source, destination).unwrap();
                let args = vec![ctcore::ct_util_name(), destination];
                let result = users_main(args.iter().map(|s| OsString::from(s)));
                assert!(result.is_ok());

                fs::remove_file(destination).expect("Failed to remove file");
            } else {
                println!("no exist {}", source);
            }
        }

        #[test]
        fn test_users_main_argument_parsing_no_file() {
            let args = vec![ctcore::ct_util_name()];
            let result = users_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_users_main_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = users_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = users_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = users_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_execution_help_short() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = users_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = users_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = users_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()];
            let result = users_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }
    }
    #[cfg(test)]
    mod ct_app_tests {
        use std::fs;

        use clap::error::ErrorKind;

        use super::*;

        // users 接口: users [OPTION]... [FILE]
        //  If FILE is not specified, use /var/run/utmp.  /var/log/wtmp as FILE is common.
        //       --help     display this help and exit
        //       --version  output version information and exit

        #[test]
        fn test_ct_app_argument_parsing_utmp_file() {
            let source = "/var/run/utmp";
            let source_path = PathBuf::from(source);
            if source_path.exists() {
                let destination = "./ct_app_utmp_test";
                // 复制文件
                std::fs::copy(source, destination).unwrap();
                let command = ct_app();

                // 测试正确的文件路径参数解析
                let args = vec![ctcore::ct_util_name(), destination];
                let executable = command.try_get_matches_from(args);
                assert!(executable.is_ok());

                // Clean up: remove the file after the test
                fs::remove_file(destination).expect("Failed to remove file");
            } else {
                println!("no exist {}", source);
            }
        }

        #[test]
        fn test_ct_app_argument_parsing_wtmp_file() {
            let source = "/var/log/wtmp";
            let source_path = PathBuf::from(source);
            if source_path.exists() {
                let destination = "./ct_app_wtmp_test";
                // 复制文件
                std::fs::copy(source, destination).unwrap();
                let command = ct_app();

                // 测试正确的文件路径参数解析
                let args = vec![ctcore::ct_util_name(), destination];
                let executable = command.try_get_matches_from(args);
                assert!(executable.is_ok());

                // Clean up: remove the file after the test
                fs::remove_file(destination).expect("Failed to remove file");
            } else {
                println!("no exist {}", source);
            }
        }

        #[test]
        fn test_ct_app_argument_parsing_no_file() {
            let command = ct_app();
            // 测试缺少文件路径参数的情况
            let args = vec![ctcore::ct_util_name()];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
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
    }
}