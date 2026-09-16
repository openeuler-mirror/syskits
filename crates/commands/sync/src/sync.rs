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
use clap::{Arg, ArgAction, Command, builder::OsStringValueParser, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");

use ctcore::Tool;
#[cfg(not(target_os = "linux"))]
use ctcore::ct_display::Quotable;
#[cfg(not(target_os = "linux"))]
use ctcore::ct_error::FromIo;
use ctcore::ct_error::{CTResult, CtSimpleError};
use ctcore::ct_posix::GnuGetoptCommandExt;

use std::borrow::Cow;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Formatter};
#[cfg(not(target_os = "linux"))]
use std::fs::File;
use sys_locale::get_locale;

mod platform;

pub mod sync_flags {
    pub const SYNC_FILE_SYSTEM: &str = "file-system";
    pub const SYNC_DATA: &str = "data";
}

const SYNC_ARG_FILES: &str = "files";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncMode {
    Global,
    File,
    Data,
    FileSystem,
}

fn select_sync_mode(has_data: bool, has_file_system: bool, has_files: bool) -> SyncMode {
    if !has_files {
        SyncMode::Global
    } else if has_file_system {
        SyncMode::FileSystem
    } else if has_data {
        SyncMode::Data
    } else {
        SyncMode::File
    }
}

#[cfg(target_os = "linux")]
fn initialize_c_locale() {
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }
}

pub fn sync_main(args: impl ctcore::Args) -> CTResult<()> {
    #[cfg(target_os = "linux")]
    initialize_c_locale();

    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_matches = ct_app().try_get_matches_from(prepare_sync_args(args)?)?;
    let is_has_data = arg_matches.get_flag(sync_flags::SYNC_DATA);
    let is_file_system = arg_matches.get_flag(sync_flags::SYNC_FILE_SYSTEM);
    #[cfg(target_os = "linux")]
    let files: Vec<OsString> = arg_matches
        .get_many::<OsString>(SYNC_ARG_FILES)
        .map(|values| values.cloned().collect())
        .unwrap_or_default();
    #[cfg(not(target_os = "linux"))]
    let files: Vec<String> = arg_matches
        .get_many::<OsString>(SYNC_ARG_FILES)
        .map(|values| {
            values
                .map(|value| value.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();

    // Check for conflicting options - must match coreutils error message
    if is_has_data && is_file_system {
        let err_message = "cannot specify both --data and --file-system";
        return Err(CtSimpleError::new(1, err_message));
    }

    if is_has_data && files.is_empty() {
        let err_message = "--data needs at least one argument";
        return Err(CtSimpleError::new(1, err_message));
    }

    #[cfg(not(target_os = "linux"))]
    for file in &files {
        check_files(file)?;
    }

    match select_sync_mode(is_has_data, is_file_system, !files.is_empty()) {
        SyncMode::Global => {
            sync();
        }
        SyncMode::File => {
            #[cfg(target_os = "linux")]
            platform::sync_files(&files)?;
            #[cfg(not(target_os = "linux"))]
            sync_files(&files)?;
        }
        SyncMode::Data => {
            #[cfg(target_os = "linux")]
            platform::sync_data(&files)?;
        }
        SyncMode::FileSystem => {
            #[cfg(target_os = "linux")]
            platform::sync_file_systems(&files)?;
            #[cfg(not(target_os = "linux"))]
            sync_fs(files);
        }
    }
    Ok(())
}

const SYNC_LONG_OPTIONS: &[&str] = &["data", "file-system", "help", "version"];
// Preserve syskits' -h/-V extensions while applying GNU diagnostics to invalid short options.
const SYNC_SHORT_OPTIONS: &[u8] = b"dfhV";

enum SyncLongOptionMatch {
    None,
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
}

fn prepare_sync_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    prepare_sync_args_with_mode(args, ctcore::ct_posix::posixly_correct())
}

fn prepare_sync_args_with_mode(
    args: impl ctcore::Args,
    posixly_correct: bool,
) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut parse_options = true;

    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if !parse_options {
            break;
        }
        if bytes == b"--" {
            parse_options = false;
            continue;
        }
        if bytes.len() <= 1 || bytes[0] != b'-' {
            if posixly_correct {
                parse_options = false;
            }
            continue;
        }

        let terminal = if bytes.starts_with(b"--") {
            validate_sync_long_option(bytes)?
        } else {
            validate_sync_short_options(bytes)?;
            false
        };
        if terminal {
            break;
        }
    }

    Ok(args)
}

fn match_sync_long_option(name: &[u8]) -> SyncLongOptionMatch {
    if let Some(option) = SYNC_LONG_OPTIONS
        .iter()
        .find(|option| option.as_bytes() == name)
    {
        return SyncLongOptionMatch::Recognized(option);
    }

    let matches = SYNC_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|option| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => SyncLongOptionMatch::None,
        [option] => SyncLongOptionMatch::Recognized(option),
        _ => SyncLongOptionMatch::Ambiguous(matches),
    }
}

fn validate_sync_long_option(argument: &[u8]) -> CTResult<bool> {
    let long = &argument[2..];
    let separator = long.iter().position(|byte| *byte == b'=');
    let name = &long[..separator.unwrap_or(long.len())];

    match match_sync_long_option(name) {
        SyncLongOptionMatch::None => {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(argument);
            message.push(b'\'');
            Err(SyncUsageError::boxed(message))
        }
        SyncLongOptionMatch::Ambiguous(matches) => {
            let possibilities = matches
                .into_iter()
                .map(|option| format!("'--{option}'"))
                .collect::<Vec<_>>()
                .join(" ");
            let mut message = b"option '".to_vec();
            message.extend_from_slice(argument);
            message.extend_from_slice(b"' is ambiguous; possibilities: ");
            message.extend_from_slice(possibilities.as_bytes());
            Err(SyncUsageError::boxed(message))
        }
        SyncLongOptionMatch::Recognized(canonical) if separator.is_some() => {
            Err(SyncUsageError::boxed(
                format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
            ))
        }
        SyncLongOptionMatch::Recognized("help" | "version") => Ok(true),
        SyncLongOptionMatch::Recognized(_) => Ok(false),
    }
}

fn validate_sync_short_options(argument: &[u8]) -> CTResult<()> {
    if let Some(unknown) = argument[1..]
        .iter()
        .find(|option| !SYNC_SHORT_OPTIONS.contains(option))
    {
        let mut message = b"invalid option -- '".to_vec();
        message.push(*unknown);
        message.push(b'\'');
        return Err(SyncUsageError::boxed(message));
    }
    Ok(())
}

#[derive(Debug)]
struct SyncUsageError {
    message: Vec<u8>,
}

impl SyncUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn ctcore::ct_error::CTError> {
        Box::new(Self { message })
    }
}

impl Display for SyncUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for SyncUsageError {}

impl ctcore::ct_error::CTError for SyncUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage(&self) -> bool {
        true
    }
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
            .help(t!("sync.clap.sync_file_system"))
            .action(ArgAction::SetTrue),
        Arg::new(sync_flags::SYNC_DATA)
            .short('d')
            .long(sync_flags::SYNC_DATA)
            .help(t!("sync.clap.sync_data"))
            .action(ArgAction::SetTrue),
        Arg::new(SYNC_ARG_FILES)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args_override_self(true)
        .args(args)
        .gnu_getopt()
}

fn sync() -> isize {
    unsafe { platform::do_sync() }
}

#[cfg(not(target_os = "linux"))]
fn sync_fs(files: Vec<String>) -> isize {
    unsafe { platform::do_syncfs(files) }
}

#[cfg(not(target_os = "linux"))]
fn sync_files(files: &[String]) -> CTResult<()> {
    for path in files {
        let file =
            File::open(path).map_err_context(|| format!("error opening {}", path.quote()))?;
        file.sync_all()
            .map_err_context(|| format!("error syncing {}", path.quote()))?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
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
    use std::sync::Mutex;

    #[cfg(unix)]
    static POSIXLY_CORRECT_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_tool_implementation() {
        let tool = Sync;

        // 测试 name 方法
        assert_eq!(tool.name(), "sync");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("sync"));

        // 测试 execute 方法
        let args = vec![OsString::from("sync"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[test]
    fn test_select_sync_mode() {
        assert_eq!(select_sync_mode(false, false, false), SyncMode::Global);
        assert_eq!(select_sync_mode(false, false, true), SyncMode::File);
        assert_eq!(select_sync_mode(true, false, true), SyncMode::Data);
        assert_eq!(select_sync_mode(false, true, true), SyncMode::FileSystem);
    }

    #[cfg(unix)]
    #[test]
    fn test_sync_main_accepts_non_utf8_file_operand() {
        use std::os::unix::ffi::OsStringExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir
            .path()
            .join(OsString::from_vec(b"file-\xff".to_vec()));
        std::fs::File::create(&file).unwrap();

        let result = sync_main([OsString::from("sync"), file.into_os_string()].into_iter());

        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_posixly_correct_stops_option_parsing_after_file_operand() {
        let _guard = POSIXLY_CORRECT_LOCK.lock().unwrap();
        let previous = std::env::var_os("POSIXLY_CORRECT");
        unsafe { std::env::set_var("POSIXLY_CORRECT", "1") };

        let matches = ct_app()
            .try_get_matches_from(["sync", "valid", "-d"])
            .unwrap();
        let files: Vec<_> = matches
            .get_many::<OsString>(SYNC_ARG_FILES)
            .unwrap()
            .map(OsString::as_os_str)
            .collect();

        match previous {
            Some(value) => unsafe { std::env::set_var("POSIXLY_CORRECT", value) },
            None => unsafe { std::env::remove_var("POSIXLY_CORRECT") },
        }

        assert!(!matches.get_flag(sync_flags::SYNC_DATA));
        assert_eq!(files, ["valid", "-d"]);
    }

    #[test]
    fn test_sync_main_reports_attached_data_value_like_gnu() {
        let error = sync_main([OsString::from("sync"), OsString::from("--data=value")].into_iter())
            .unwrap_err();

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"option '--data' doesn't allow an argument"
        );
        assert!(error.usage());
    }

    #[test]
    fn test_prepare_sync_args_reports_gnu_option_errors() {
        let cases = [
            (
                "--file-system=value",
                b"option '--file-system' doesn't allow an argument".as_slice(),
            ),
            ("--unknown", b"unrecognized option '--unknown'".as_slice()),
            ("-dfile", b"invalid option -- 'i'".as_slice()),
        ];

        for (argument, expected) in cases {
            let error = prepare_sync_args_with_mode(
                [OsString::from("sync"), OsString::from(argument)].into_iter(),
                false,
            )
            .unwrap_err();

            assert_eq!(error.diagnostic_bytes().as_ref(), expected);
            assert!(error.usage());
        }
    }

    #[test]
    fn test_prepare_sync_args_stops_validation_after_posix_operand() {
        let args = [
            OsString::from("sync"),
            OsString::from("file"),
            OsString::from("--unknown"),
        ];

        let prepared = prepare_sync_args_with_mode(args.clone().into_iter(), true).unwrap();
        assert_eq!(prepared, args);

        let error = prepare_sync_args_with_mode(args.into_iter(), false).unwrap_err();
        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"unrecognized option '--unknown'"
        );
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
            let args = [ctcore::ct_util_name(), "--version"];
            let result = sync_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = sync_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = sync_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_help_short() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = sync_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = sync_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = sync_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()];
            let result = sync_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_system_long() {
            let args = [ctcore::ct_util_name(), "--file-system"];
            let result = sync_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_system_short() {
            let args = [ctcore::ct_util_name(), "-f"];
            let result = sync_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_data_long() {
            let args = [ctcore::ct_util_name(), "--data"];
            let result = sync_main(args.iter().map(OsString::from));

            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "--data needs at least one argument".to_string()
            )
        }

        #[test]
        fn test_ct_main_file_data_short() {
            let args = [ctcore::ct_util_name(), "-d"];
            let result = sync_main(args.iter().map(OsString::from));

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

            let args = [ctcore::ct_util_name(), dir_name];
            let result = sync_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_file_without_mode() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_ct_main_file_without_mode");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "sync-data").unwrap();

            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), file_name];
            let result = sync_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--file-system", file_name];
            let result = sync_main(args.iter().map(OsString::from));
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
            let args = [ctcore::ct_util_name(), "-f", file_name];
            let result = sync_main(args.iter().map(OsString::from));
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
            let args = [ctcore::ct_util_name(), "--data", file_name];
            let result = sync_main(args.iter().map(OsString::from));
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
            let args = [ctcore::ct_util_name(), "-d", file_name];
            let result = sync_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_repeated_file_data_with_file() {
            let filename = "test_ct_main_repeated_file_data_with_file";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "sync-data").unwrap();

            let file_name = file_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-d", "--data", "-d", file_name];
            let result = sync_main(args.iter().map(OsString::from));

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
        fn test_ct_app_repeated_file_data() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d", "--data", "-d", "file"];

            let matches = command.try_get_matches_from(args).unwrap();

            assert!(matches.get_flag(sync_flags::SYNC_DATA));
            assert!(!matches.get_flag(sync_flags::SYNC_FILE_SYSTEM));
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
