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
use std::borrow::Cow;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Formatter};
use std::os::unix::ffi::OsStrExt;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::builder::ValueParser;
use clap::{Arg, ArgMatches, Command, crate_version};
use std::io::Write;
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult};
use ctcore::ct_posix::{GnuGetoptCommandExt, posixly_correct};
use ctcore::ct_utmpx::{self, CtUtmpx};

static USERS_ARG_FILES: &str = "files";
const USERS_LONG_OPTIONS: &[&str] = &["help", "version"];

#[derive(Debug, PartialEq, Eq)]
enum UsersLongOptionMatch {
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
    None,
}

#[derive(Debug)]
struct UsersUsageError {
    message: Vec<u8>,
}

impl UsersUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

impl Display for UsersUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for UsersUsageError {}

impl CTError for UsersUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsersSession {
    pub user: String,
    pub tty_device: String,
    pub host: String,
    user_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsersSemantic {
    pub sessions: Vec<UsersSession>,
    pub classic_text: String,
    pub classic_bytes: Vec<u8>,
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
        let result = users_native_semantic(args.iter().cloned());
        match result {
            Ok(semantic) => {
                if !semantic.sessions.is_empty() {
                    let mut stdout = std::io::stdout().lock();
                    stdout.write_all(&semantic.classic_bytes)?;
                    stdout.write_all(b"\n")?;
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

pub fn users_main(args: impl ctcore::Args) -> CTResult<Vec<u8>> {
    let semantic = users_native_semantic(args)?;
    Ok(semantic.classic_bytes)
}

fn prepare_users_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    prepare_users_args_with_mode(args, posixly_correct())
}

fn prepare_users_args_with_mode(
    args: impl ctcore::Args,
    posix_mode: bool,
) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut parse_options = true;
    let mut operands = Vec::new();

    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if parse_options && bytes == b"--" {
            parse_options = false;
            continue;
        }

        if parse_options && bytes.len() > 1 && bytes[0] == b'-' {
            if bytes.starts_with(b"--") {
                if validate_users_long_option(bytes)? {
                    let mut terminal_args = Vec::with_capacity(2);
                    if let Some(program) = args.first() {
                        terminal_args.push(program.clone());
                    }
                    terminal_args.push(argument.clone());
                    return Ok(terminal_args);
                }
                continue;
            }

            // -h and -V are syskits extensions, not GNU users options.
            if bytes == b"-h" || bytes == b"-V" {
                return Ok(args);
            }

            let mut message = b"invalid option -- '".to_vec();
            message.push(bytes[1]);
            message.push(b'\'');
            return Err(UsersUsageError::boxed(message));
        }

        operands.push(argument);
        if posix_mode {
            parse_options = false;
        }
    }

    if operands.len() > 1 {
        let mut message = b"extra operand ".to_vec();
        message.extend(users_quote_c_operand(operands[1].as_os_str()));
        return Err(UsersUsageError::boxed(message));
    }

    Ok(args)
}

fn validate_users_long_option(argument: &[u8]) -> CTResult<bool> {
    let long = &argument[2..];
    let separator = long.iter().position(|byte| *byte == b'=');
    let name = &long[..separator.unwrap_or(long.len())];

    match match_users_long_option(name) {
        UsersLongOptionMatch::None => {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(argument);
            message.push(b'\'');
            Err(UsersUsageError::boxed(message))
        }
        UsersLongOptionMatch::Ambiguous(candidates) => {
            let mut message = b"option '".to_vec();
            message.extend_from_slice(argument);
            message.extend_from_slice(b"' is ambiguous; possibilities:");
            for candidate in candidates {
                message.extend_from_slice(b" '--");
                message.extend_from_slice(candidate.as_bytes());
                message.push(b'\'');
            }
            Err(UsersUsageError::boxed(message))
        }
        UsersLongOptionMatch::Recognized(canonical) if separator.is_some() => {
            Err(UsersUsageError::boxed(
                format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
            ))
        }
        UsersLongOptionMatch::Recognized("help" | "version") => Ok(true),
        UsersLongOptionMatch::Recognized(_) => unreachable!("users only has standard options"),
    }
}

fn match_users_long_option(name: &[u8]) -> UsersLongOptionMatch {
    if let Some(option) = USERS_LONG_OPTIONS
        .iter()
        .copied()
        .find(|option| option.as_bytes() == name)
    {
        return UsersLongOptionMatch::Recognized(option);
    }

    let candidates = USERS_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|option| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [] => UsersLongOptionMatch::None,
        [candidate] => UsersLongOptionMatch::Recognized(candidate),
        _ => UsersLongOptionMatch::Ambiguous(candidates),
    }
}

fn users_quote_c_operand(operand: &std::ffi::OsStr) -> Vec<u8> {
    users_quote_c_bytes(operand.as_bytes())
}

fn users_quote_c_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut quoted = Vec::with_capacity(bytes.len() + 2);
    quoted.push(b'\'');
    for byte in bytes {
        match *byte {
            b'\x07' => quoted.extend_from_slice(b"\\a"),
            b'\x08' => quoted.extend_from_slice(b"\\b"),
            b'\t' => quoted.extend_from_slice(b"\\t"),
            b'\n' => quoted.extend_from_slice(b"\\n"),
            b'\x0b' => quoted.extend_from_slice(b"\\v"),
            b'\x0c' => quoted.extend_from_slice(b"\\f"),
            b'\r' => quoted.extend_from_slice(b"\\r"),
            b'\\' => quoted.extend_from_slice(b"\\\\"),
            b'\'' => quoted.extend_from_slice(b"\\'"),
            b' '..=b'~' => quoted.push(*byte),
            _ => {
                quoted.push(b'\\');
                quoted.push(b'0' + (byte >> 6));
                quoted.push(b'0' + ((byte >> 3) & 7));
                quoted.push(b'0' + (byte & 7));
            }
        }
    }
    quoted.push(b'\'');
    quoted
}

fn trim_user_name(name: &[u8]) -> &[u8] {
    let length = name
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(0, |index| index + 1);
    &name[..length]
}

fn should_keep_user_pid(check_pids: bool, pid: i32) -> bool {
    if !check_pids || pid <= 0 {
        return true;
    }

    let status = unsafe { ctcore::libc::kill(pid, 0) };
    status == 0 || std::io::Error::last_os_error().raw_os_error() != Some(ctcore::libc::ESRCH)
}

fn users_sessions_from_file(path: &Path, check_pids: bool) -> Vec<UsersSession> {
    let mut sessions = CtUtmpx::iter_all_records_from(path)
        .filter(|ut| ut.is_user_process() && should_keep_user_pid(check_pids, ut.pid()))
        .map(|ut| {
            let user_bytes = trim_user_name(ut.user_bytes()).to_vec();
            UsersSession {
                user: String::from_utf8_lossy(&user_bytes).into_owned(),
                tty_device: ut.tty_device(),
                host: ut.host(),
                user_bytes,
            }
        })
        .collect::<Vec<_>>();

    sessions.sort_by(|left, right| {
        left.user_bytes
            .cmp(&right.user_bytes)
            .then_with(|| left.tty_device.cmp(&right.tty_device))
            .then_with(|| left.host.cmp(&right.host))
    });

    sessions
}

fn users_classic_text(sessions: &[UsersSession]) -> Vec<u8> {
    let mut output = Vec::new();
    for (index, session) in sessions.iter().enumerate() {
        if index != 0 {
            output.push(b' ');
        }
        output.extend_from_slice(trim_user_name(&session.user_bytes));
    }
    output
}

pub fn users_native_semantic(args: impl ctcore::Args) -> CTResult<UsersSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app()
        .after_help(users_get_long_usage())
        .try_get_matches_from(prepare_users_args(args)?)?;

    let (filename, check_pids) = parse_users_files(matches);
    let sessions = users_sessions_from_file(&filename, check_pids);
    let classic_bytes = users_classic_text(&sessions);
    let classic_text = String::from_utf8_lossy(&classic_bytes).into_owned();
    Ok(UsersSemantic {
        sessions,
        classic_text,
        classic_bytes,
    })
}

fn parse_users_files(matches: ArgMatches) -> (PathBuf, bool) {
    let files: Vec<&Path> = matches
        .get_many::<OsString>(USERS_ARG_FILES)
        .map(|v| v.map(AsRef::as_ref).collect())
        .unwrap_or_default();

    if files.is_empty() {
        (PathBuf::from(ct_utmpx::DEFAULT_FILE), true)
    } else {
        (files[0].to_path_buf(), false)
    }
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
        .gnu_getopt()
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
        use std::os::unix::ffi::OsStringExt;
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
            assert_eq!(result.unwrap(), b"user1 user2 user3");
        }

        #[test]
        fn users_classic_text_trims_trailing_spaces_from_login_names() {
            let sessions = vec![UsersSession {
                user: "zeta   ".into(),
                tty_device: "pts/1".into(),
                host: "localhost".into(),
                user_bytes: b"zeta   ".to_vec(),
            }];

            assert_eq!(users_classic_text(&sessions), b"zeta");
        }

        #[test]
        fn users_main_preserves_non_utf8_login_name_bytes() {
            let dir = TempDir::with_prefix("test_users_raw_").unwrap();
            let file_path = dir.path().join("users.utmp");
            let mut file = File::create(&file_path).unwrap();
            let mut record = unsafe { std::mem::zeroed::<libc::utmpx>() };
            record.ut_type = ctcore::ct_utmpx::USER_PROCESS;
            record.ut_pid = 1;
            copy_str_to_c_char_array(&mut record.ut_line, "pts/1");
            copy_str_to_c_char_array(&mut record.ut_id, "0001");
            for (dst, byte) in record.ut_user.iter_mut().zip([b'a', 0xff]) {
                *dst = byte as libc::c_char;
            }

            let record_bytes = unsafe {
                std::slice::from_raw_parts(
                    &record as *const libc::utmpx as *const u8,
                    std::mem::size_of::<libc::utmpx>(),
                )
            };
            file.write_all(record_bytes).unwrap();

            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from(file_path),
            ];
            assert_eq!(users_main(args.into_iter()).unwrap(), b"a\xff");
        }

        #[test]
        fn users_reports_unknown_option_with_gnu_diagnostic() {
            let args = [ctcore::ct_util_name(), "--invalid-option"];
            let error = users_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(error.to_string(), "unrecognized option '--invalid-option'");
        }

        #[test]
        fn users_reports_second_file_as_extra_operand() {
            let args = [ctcore::ct_util_name(), "first", "second"];
            let error = users_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(error.to_string(), "extra operand 'second'");
        }

        #[test]
        fn users_posix_mode_stops_option_parsing_after_first_file() {
            let args = [ctcore::ct_util_name(), "first", "--version"];
            let error =
                prepare_users_args_with_mode(args.iter().map(OsString::from), true).unwrap_err();

            assert_eq!(error.to_string(), "extra operand '--version'");
        }

        #[test]
        fn users_preserves_non_utf8_unknown_option_bytes() {
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from_vec(vec![b'-', b'-', 0xff]),
            ];
            let error = users_main(args.into_iter()).unwrap_err();

            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"unrecognized option '--\xff'"
            );
        }

        #[test]
        fn users_default_source_discards_missing_positive_user_pids() {
            assert!(!should_keep_user_pid(true, i32::MAX));
            assert!(should_keep_user_pid(false, i32::MAX));
            assert!(should_keep_user_pid(true, 0));
            assert!(should_keep_user_pid(true, std::process::id() as i32));
        }

        #[test]
        fn users_only_check_pids_for_the_default_source() {
            let default_matches = ct_app()
                .try_get_matches_from([ctcore::ct_util_name()])
                .unwrap();
            let (default_file, default_checks_pids) = parse_users_files(default_matches);
            assert_eq!(default_file, PathBuf::from(ct_utmpx::DEFAULT_FILE));
            assert!(default_checks_pids);

            let explicit_matches = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), "fixture.utmp"])
                .unwrap();
            let (explicit_file, explicit_checks_pids) = parse_users_files(explicit_matches);
            assert_eq!(explicit_file, PathBuf::from("fixture.utmp"));
            assert!(!explicit_checks_pids);
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
                        user_bytes: b"user1".to_vec(),
                    },
                    UsersSession {
                        user: "user2".into(),
                        tty_device: "pts/2".into(),
                        host: "remote-b".into(),
                        user_bytes: b"user2".to_vec(),
                    },
                ]
            );
            assert_eq!(result.classic_bytes, b"user1 user2");
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
