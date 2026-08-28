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

//! users命令用于显示当前登录系统的所有用户的用户列表
//! 每个显示的用户名对应一个登录会话。如果一个用户有不止一个登录会话，那他的用户名将显示相同的次数。

extern crate rust_i18n;
use rust_i18n::t;
use std::ffi::OsString;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::builder::ValueParser;
use clap::{Arg, ArgMatches, Command, crate_version};
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

use ctcore::Tool;
use ctcore::ct_error::CTResult;
use ctcore::ct_utmpx::{self, CtUtmpx};

static USERS_ARG_FILES: &str = "files";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsersSession {
    pub user: String,
    pub tty_device: String,
    pub host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsersSemantic {
    pub sessions: Vec<UsersSession>,
    pub classic_text: String,
}

fn users_get_long_usage() -> String {
    format!(
        "Output who is currently logged in according to FILE.
If FILE is not specified, use {}.  /var/log/wtmp as FILE is common.",
        ct_utmpx::DEFAULT_FILE
    )
}

#[derive(Default)]
pub struct Users;
impl Tool for Users {
    fn name(&self) -> &'static str {
        "users"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let result = users_main(args.iter().cloned());
        match result {
            Ok(s) => {
                if !s.is_empty() {
                    println!("{s}");
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

pub fn users_main(args: impl ctcore::Args) -> CTResult<String> {
    let semantic = users_native_semantic(args)?;
    Ok(semantic.classic_text)
}

fn users_sessions_from_file(path: &Path) -> Vec<UsersSession> {
    let mut sessions = CtUtmpx::iter_all_records_from(path)
        .filter(CtUtmpx::is_user_process)
        .map(|ut| UsersSession {
            user: ut.user(),
            tty_device: ut.tty_device(),
            host: ut.host(),
        })
        .collect::<Vec<_>>();

    sessions.sort_by(|left, right| {
        left.user
            .cmp(&right.user)
            .then_with(|| left.tty_device.cmp(&right.tty_device))
            .then_with(|| left.host.cmp(&right.host))
    });

    sessions
}

fn users_classic_text(sessions: &[UsersSession]) -> String {
    sessions
        .iter()
        .map(|session| session.user.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn users_native_semantic(args: impl ctcore::Args) -> CTResult<UsersSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app()
        .after_help(users_get_long_usage())
        .try_get_matches_from(args)?;

    let filename = parse_users_files(matches);
    let sessions = users_sessions_from_file(&filename);
    let classic_text = users_classic_text(&sessions);
    Ok(UsersSemantic {
        sessions,
        classic_text,
    })
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
    let application_info = t!("users.about");
    let usage_description = t!("users.usage");
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
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Users;

        // Test name method
        assert_eq!(tool.name(), "users");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("users"));

        // Test execute method with help flag (should work)
        let args: Vec<OsString> = vec![OsString::from("users"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::fs;
        use std::fs::File;
        use std::io::Write;
        use tempfile::TempDir;

        fn copy_str_to_c_char_array<const N: usize>(dst: &mut [libc::c_char; N], src: &str) {
            for (dst, byte) in dst.iter_mut().zip(src.bytes()) {
                *dst = byte as libc::c_char;
            }
        }

        fn write_users_fixture(rows: &[(&str, &str, &str)]) -> (TempDir, String) {
            let dir = TempDir::with_prefix("test_users_").unwrap();
            let file_path = dir.path().join("users.utmp");
            let mut tmp_file = File::create(&file_path).unwrap();

            for (index, (username, terminal, hostname)) in rows.iter().enumerate() {
                let mut record = unsafe { std::mem::zeroed::<libc::utmpx>() };
                record.ut_type = ctcore::ct_utmpx::USER_PROCESS;
                record.ut_pid = i32::try_from(index + 1).unwrap();
                copy_str_to_c_char_array(&mut record.ut_line, terminal);
                copy_str_to_c_char_array(&mut record.ut_user, username);
                copy_str_to_c_char_array(&mut record.ut_host, hostname);
                let id = format!("{index:04}");
                for (dst, byte) in record.ut_id.iter_mut().zip(id.bytes()) {
                    *dst = byte as libc::c_char;
                }

                let record_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        &record as *const libc::utmpx as *const u8,
                        std::mem::size_of::<libc::utmpx>(),
                    )
                };
                tmp_file.write_all(record_bytes).unwrap();
            }

            (dir, file_path.to_string_lossy().into_owned())
        }

        #[test]
        fn test_users_main_argument_parsing_file() {
            let (_dir, file_name) = write_users_fixture(&[
                ("user3", "tty3", "localhost"),
                ("user1", "tty1", "localhost"),
                ("user2", "tty2", "localhost"),
            ]);

            let args = [ctcore::ct_util_name(), file_name.as_str()];
            let result = users_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), "user1 user2 user3");
        }

        #[test]
        fn test_users_native_semantic_argument_parsing_file() {
            let (_dir, file_name) = write_users_fixture(&[
                ("user2", "pts/2", "remote-b"),
                ("user1", "pts/1", "remote-a"),
            ]);

            let args = [ctcore::ct_util_name(), file_name.as_str()];
            let result = users_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(
                result.sessions,
                vec![
                    UsersSession {
                        user: "user1".into(),
                        tty_device: "pts/1".into(),
                        host: "remote-a".into(),
                    },
                    UsersSession {
                        user: "user2".into(),
                        tty_device: "pts/2".into(),
                        host: "remote-b".into(),
                    },
                ]
            );
            assert_eq!(result.classic_text, "user1 user2");
        }

        #[test]
        fn test_users_main_argument_parsing_utmp_file() {
            let source = "/var/run/utmp";
            let source_path = PathBuf::from(source);
            if source_path.exists() {
                let destination = "./users_main_utmp_test";
                std::fs::copy(source, destination).unwrap();

                let args = [ctcore::ct_util_name(), destination];
                let result = users_main(args.iter().map(OsString::from));

                assert!(result.is_ok());

                fs::remove_file(destination).expect("Failed to remove file");
            } else {
                println!("no exist {source}");
            }
        }

        #[test]
        fn test_users_main_argument_parsing_wtmp_file() {
            let source = "/var/log/wtmp";
            let source_path = PathBuf::from(source);
            if source_path.exists() {
                let destination = "./users_main_wtmp_test";

                std::fs::copy(source, destination).unwrap();
                let args = [ctcore::ct_util_name(), destination];
                let result = users_main(args.iter().map(OsString::from));
                assert!(result.is_ok());

                fs::remove_file(destination).expect("Failed to remove file");
            } else {
                println!("no exist {source}");
            }
        }

        #[test]
        fn test_users_main_argument_parsing_no_file() {
            let args = [ctcore::ct_util_name()];
            let result = users_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_users_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = users_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = users_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = users_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_execution_help_short() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = users_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = users_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = users_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_users_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()];
            let result = users_main(args.iter().map(OsString::from));
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
                println!("no exist {source}");
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
                println!("no exist {source}");
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
