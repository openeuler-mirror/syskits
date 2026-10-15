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

//! 创建硬链接
//!
//! 调用 link 函数，创建一个名为 <文件2> 的硬链接，指向现有的文件 <文件1>。

extern crate rust_i18n;
use clap::builder::ValueParser;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::{
    Tool,
    ct_display::{Quotable, locale_quote},
    ct_error::{CTResult, CtSimpleError, FromIo},
};
use std::{ffi::OsString, fs::hard_link, io, path::Path};
use sys_locale::get_locale;

const SYSKITS_LINK_LONG_VERSION: &str = concat!(
    "(syskits) ",
    env!("CARGO_PKG_VERSION"),
    "\nCopyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.\n",
    "License Mulan PSL v2: <http://license.coscl.org.cn/MulanPSL2>.\n",
    "This is free software: you are free to change and redistribute it.\n",
    "There is NO WARRANTY, to the extent permitted by law."
);

const SYSKITS_LINK_AFTER_HELP: &str = "syskits online help: <https://www.ctyun.cn/developer>";

mod link_flags {
    pub const FILES: &str = "FILES";
}

/// 存储链接操作的源文件和目标文件路径
struct LinkFlags<'a> {
    source_path: &'a Path,
    target_path: &'a Path,
}

impl<'a> LinkFlags<'a> {
    /// 从命令行参数创建 LinkFlags 实例
    ///
    /// # Arguments
    /// * `matches` - 解析后的命令行参数
    ///
    /// # Returns
    /// * `CTResult<Self>` - 成功则返回 LinkFlags 实例，失败则返回错误
    fn new(matches: &'a clap::ArgMatches, usage_name: &str) -> CTResult<Self> {
        let files: Vec<_> = matches
            .get_many::<OsString>(link_flags::FILES)
            .unwrap_or_default()
            .collect();

        match files.len() {
            0 => return Err(link_usage_error(usage_name, "missing operand".to_string())),
            1 => {
                return Err(link_usage_error(
                    usage_name,
                    format!(
                        "missing operand after {}",
                        locale_quote_file_operand(files[0])
                    ),
                ));
            }
            2 => {}
            _ => {
                return Err(link_usage_error(
                    usage_name,
                    format!("extra operand {}", locale_quote_file_operand(files[2])),
                ));
            }
        }

        Ok(Self {
            source_path: Path::new(files[0]),
            target_path: Path::new(files[1]),
        })
    }
}

/// 主函数：解析参数并执行链接操作
pub fn link_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut args = args.collect::<Vec<_>>();
    let usage_name = args
        .first()
        .map(|arg| arg.to_string_lossy().into_owned())
        .filter(|arg| !arg.is_empty())
        .unwrap_or_else(|| ctcore::ct_util_name().to_owned());
    stop_option_parsing_for_posixly_correct(&mut args);
    validate_standard_options(&args, &usage_name)?;
    let matches = ct_app().try_get_matches_from(args)?;
    let flags = LinkFlags::new(&matches, &usage_name)?;
    link_exec(&flags)
}

fn stop_option_parsing_for_posixly_correct(args: &mut Vec<OsString>) {
    if std::env::var_os("POSIXLY_CORRECT").is_none() {
        return;
    }

    for index in 1..args.len() {
        let argument = args[index].to_string_lossy();
        if argument == "--" {
            return;
        }
        if argument == "-" || !argument.starts_with('-') {
            args.insert(index, OsString::from("--"));
            return;
        }
    }
}

fn link_usage_error(usage_name: &str, message: String) -> Box<dyn ctcore::ct_error::CTError> {
    CtSimpleError::new(
        1,
        format!("{message}\nTry '{usage_name} --help' for more information."),
    )
}

fn validate_standard_options(args: &[OsString], usage_name: &str) -> CTResult<()> {
    for arg in args.iter().skip(1) {
        let arg = arg.to_string_lossy();
        if arg == "--" {
            break;
        }

        if let Some(long) = arg.strip_prefix("--") {
            let (name, value) = long
                .split_once('=')
                .map_or((long, None), |(name, value)| (name, Some(value)));
            let matches = ["help", "version"]
                .into_iter()
                .filter(|option| option.starts_with(name))
                .collect::<Vec<_>>();

            match matches.as_slice() {
                [option] if value.is_some() => {
                    return Err(link_usage_error(
                        usage_name,
                        format!(
                            "option {} doesn't allow an argument",
                            locale_quote(&format!("--{option}"))
                        ),
                    ));
                }
                [_] => return Ok(()),
                [] => {
                    return Err(link_usage_error(
                        usage_name,
                        format!("unrecognized option {}", locale_quote(&arg)),
                    ));
                }
                _ => {
                    return Err(link_usage_error(
                        usage_name,
                        format!(
                            "option {} is ambiguous; possibilities: '--help' '--version'",
                            locale_quote(&arg)
                        ),
                    ));
                }
            }
        }

        if let Some(short) = arg.strip_prefix('-')
            && !short.is_empty()
            && !short.starts_with('-')
        {
            let option = short.chars().next().expect("short option is not empty");
            return Err(link_usage_error(
                usage_name,
                format!("invalid option -- '{option}'"),
            ));
        }
    }

    Ok(())
}

fn locale_quote_file_operand(filename: &OsString) -> String {
    if let Some(filename) = filename.to_str() {
        return locale_quote(filename);
    }

    #[cfg(unix)]
    return locale_quote_non_utf8_file_operand(filename);

    #[cfg(not(unix))]
    filename.quote().to_string()
}

#[cfg(unix)]
fn locale_quote_non_utf8_file_operand(filename: &OsString) -> String {
    use std::os::unix::ffi::OsStrExt;

    let marker_quote = locale_quote("x");
    let (left_quote, right_quote) = marker_quote
        .split_once('x')
        .expect("locale quote must surround its content");
    let escape_apostrophe = right_quote == "'";
    let utf8_locale = left_quote == "‘";
    let mut escaped = String::new();
    let mut remaining = filename.as_bytes();

    while !remaining.is_empty() {
        match std::str::from_utf8(remaining) {
            Ok(text) => {
                append_locale_quoted_text(&mut escaped, text, escape_apostrophe, utf8_locale);
                break;
            }
            Err(error) => {
                let valid_end = error.valid_up_to();
                if valid_end != 0 {
                    let (valid, rest) = remaining.split_at(valid_end);
                    append_locale_quoted_text(
                        &mut escaped,
                        std::str::from_utf8(valid).expect("valid UTF-8 prefix"),
                        escape_apostrophe,
                        utf8_locale,
                    );
                    remaining = rest;
                }
                let byte = remaining[0];
                use std::fmt::Write;
                write!(escaped, "\\{byte:03o}").expect("write to string");
                remaining = &remaining[1..];
            }
        }
    }

    format!("{left_quote}{escaped}{right_quote}")
}

#[cfg(unix)]
fn append_locale_quoted_text(
    escaped: &mut String,
    text: &str,
    escape_apostrophe: bool,
    utf8_locale: bool,
) {
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\'' if escape_apostrophe => escaped.push_str("\\'"),
            '\u{0007}' => escaped.push_str("\\a"),
            '\u{0008}' => escaped.push_str("\\b"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\u{000B}' => escaped.push_str("\\v"),
            '\u{000C}' => escaped.push_str("\\f"),
            '\r' => escaped.push_str("\\r"),
            character if character.is_control() || (!utf8_locale && !character.is_ascii()) => {
                use std::fmt::Write;
                for byte in character.to_string().as_bytes() {
                    write!(escaped, "\\{byte:03o}").expect("write to string");
                }
            }
            character => escaped.push(character),
        }
    }
}

fn shell_quote_path(path: &Path) -> String {
    #[cfg(unix)]
    if path.as_os_str().to_str().is_none() {
        use std::os::unix::ffi::OsStrExt;

        return shell_quote_non_utf8_file_operand(path.as_os_str().as_bytes());
    }

    path.quote().to_string()
}

#[cfg(unix)]
enum ShellQuoteSegment {
    Plain,
    Escaped,
}

#[cfg(unix)]
fn shell_quote_non_utf8_file_operand(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() + 8);
    let mut segment = None;

    if !matches!(bytes.first(), Some(b'\x20'..=b'\x7e')) {
        result.push_str("''");
    }

    for &byte in bytes {
        match byte {
            b'\x07' => push_shell_escaped_byte(&mut result, &mut segment, "a"),
            b'\x08' => push_shell_escaped_byte(&mut result, &mut segment, "b"),
            b'\x09' => push_shell_escaped_byte(&mut result, &mut segment, "t"),
            b'\x0a' => push_shell_escaped_byte(&mut result, &mut segment, "n"),
            b'\x0b' => push_shell_escaped_byte(&mut result, &mut segment, "v"),
            b'\x0c' => push_shell_escaped_byte(&mut result, &mut segment, "f"),
            b'\x0d' => push_shell_escaped_byte(&mut result, &mut segment, "r"),
            b'\x20'..=b'\x7e' => push_shell_plain_byte(&mut result, &mut segment, byte),
            _ => {
                let escaped = format!("{byte:03o}");
                push_shell_escaped_byte(&mut result, &mut segment, &escaped);
            }
        }
    }

    match segment {
        Some(ShellQuoteSegment::Plain) | Some(ShellQuoteSegment::Escaped) => result.push('\''),
        None => result.push_str("''"),
    }
    result
}

#[cfg(unix)]
fn push_shell_plain_byte(result: &mut String, segment: &mut Option<ShellQuoteSegment>, byte: u8) {
    if matches!(segment, Some(ShellQuoteSegment::Escaped)) {
        result.push('\'');
        *segment = None;
    }
    if segment.is_none() {
        result.push('\'');
        *segment = Some(ShellQuoteSegment::Plain);
    }
    if byte == b'\'' {
        result.push_str("'\\\\''");
    } else {
        result.push(char::from(byte));
    }
}

#[cfg(unix)]
fn push_shell_escaped_byte(
    result: &mut String,
    segment: &mut Option<ShellQuoteSegment>,
    escaped: &str,
) {
    if matches!(segment, Some(ShellQuoteSegment::Plain)) {
        result.push('\'');
        *segment = None;
    }
    if segment.is_none() {
        result.push_str("$'");
        *segment = Some(ShellQuoteSegment::Escaped);
    }
    result.push('\\');
    result.push_str(escaped);
}

/// 执行硬链接创建操作
fn link_exec(flags: &LinkFlags) -> CTResult<()> {
    match hard_link(flags.source_path, flags.target_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(CtSimpleError::new(
            1,
            format!(
                "cannot create link {} to {}: File exists",
                shell_quote_path(flags.target_path),
                shell_quote_path(flags.source_path)
            ),
        )),
        #[cfg(unix)]
        Err(error) if error.raw_os_error() == Some(ctcore::libc::EPERM) => Err(CtSimpleError::new(
            1,
            format!(
                "cannot create link {} to {}: Operation not permitted",
                shell_quote_path(flags.target_path),
                shell_quote_path(flags.source_path)
            ),
        )),
        Err(error) => Err(error.map_err_context(|| {
            format!(
                "cannot create link {} to {}",
                shell_quote_path(flags.target_path),
                shell_quote_path(flags.source_path)
            )
        })),
    }
}

/// 创建命令行应用程序配置
pub fn ct_app() -> Command {
    let arg = Arg::new(link_flags::FILES)
        .hide(true)
        .action(ArgAction::Append)
        .value_hint(clap::ValueHint::AnyPath)
        .value_parser(ValueParser::os_string());

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .long_version(SYSKITS_LINK_LONG_VERSION)
        .about(t!("link.about"))
        .after_help(SYSKITS_LINK_AFTER_HELP)
        .override_usage(t!("link.usage"))
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .long("help")
                .help("display this help and exit")
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .help("output version information and exit")
                .action(ArgAction::Version),
        )
        .arg(arg)
}

#[derive(Default)]
pub struct Link;
impl Tool for Link {
    fn name(&self) -> &'static str {
        "link"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        link_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    use tempfile::Builder;

    #[test]
    fn test_tool_implementation() {
        let tool = Link;

        // 测试 name 方法
        assert_eq!(tool.name(), "link");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("link"));

        // 测试 execute 方法
        let args = vec![OsString::from("link"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err()); // link命令需要参数，所以不带参数应该返回错误
    }

    mod ct_app_tests {
        /*
        测试版本和帮助标志
        测试无效参数
        测试缺少必需参数
        测试有效参数

        短版本标志测试 (-V)
        短帮助标志测试 (-h)
        参数过多的测试
        */
        use super::*;
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_missing_required_args() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_valid_args() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "source.txt", "target.txt"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_rejects_short_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_rejects_short_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_too_many_args() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "source.txt",
                "target.txt",
                "extra.txt",
            ];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }

    mod link_exec_tests {
        /*
        测试成功创建硬链接
        测试源文件不存在的情况
        测试目标文件已存在的情况

        目录作为源文件的测试
        源文件和目标文件相同的测试
        目标文件在不存在目录中的测试
        验证硬链接创建成功的测试
         */
        use super::*;

        fn setup_test_files() -> (tempfile::TempDir, PathBuf, PathBuf) {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("source.txt");
            let target_file = temp_dir.path().join("target.txt");
            File::create(&source_file).unwrap();
            (temp_dir, source_file, target_file)
        }

        #[test]
        fn test_link_exec_success() {
            let (_temp_dir, source_file, target_file) = setup_test_files();
            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_exec_nonexistent_source() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("nonexistent.txt");
            let target_file = temp_dir.path().join("target.txt");

            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_existing_target() {
            let (_temp_dir, source_file, target_file) = setup_test_files();
            File::create(&target_file).unwrap();

            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_directory_as_source() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_dir = temp_dir.path().join("source_dir");
            let target_file = temp_dir.path().join("target.txt");
            std::fs::create_dir(&source_dir).unwrap();

            let flags = LinkFlags {
                source_path: source_dir.as_path(),
                target_path: target_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_same_file() {
            let (_temp_dir, source_file, _) = setup_test_files();
            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: source_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_target_in_nonexistent_directory() {
            let (_temp_dir, source_file, _) = setup_test_files();
            let target_file = Path::new("/nonexistent/directory/target.txt");

            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file,
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_verify_hard_link() {
            let (_temp_dir, source_file, target_file) = setup_test_files();
            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file.as_path(),
            };
            link_exec(&flags).unwrap();

            // 验证是否创建了硬链接
            let source_metadata = std::fs::metadata(&source_file).unwrap();
            let target_metadata = std::fs::metadata(&target_file).unwrap();
            assert_eq!(source_metadata.nlink(), 2);
            assert_eq!(target_metadata.nlink(), 2);
        }
    }

    mod link_main_tests {
        /*
        测试成功执行
        测试参数数量错误
        测试源文件不存在
        测试帮助标志
        空参数测试
        带空格的路径测试
        相对路径测试
        Unicode 路径测试
        长路径名测试
        */
        use super::*;

        fn setup_test_files() -> (tempfile::TempDir, String, String) {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("source.txt");
            let target_file = temp_dir.path().join("target.txt");
            File::create(&source_file).unwrap();
            (
                temp_dir,
                source_file.to_str().unwrap().to_string(),
                target_file.to_str().unwrap().to_string(),
            )
        }

        #[test]
        fn test_link_main_success() {
            let (_temp_dir, source, target) = setup_test_files();
            let args = [ctcore::ct_util_name(), &source, &target];
            let result = link_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_main_wrong_number_of_arguments() {
            let args = [ctcore::ct_util_name(), "single_file.txt"];
            let result = link_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_link_main_operand_diagnostics() {
            let cases = [
                (vec![ctcore::ct_util_name()], "missing operand".to_string()),
                (
                    vec![ctcore::ct_util_name(), "file1"],
                    format!("missing operand after {}", locale_quote("file1")),
                ),
                (
                    vec![ctcore::ct_util_name(), "file1", "file2", "file3"],
                    format!("extra operand {}", locale_quote("file3")),
                ),
            ];

            for (args, message) in cases {
                let error = link_main(args.iter().map(OsString::from))
                    .expect_err("invalid operand count must fail");
                assert_eq!(
                    error.to_string(),
                    format!(
                        "{message}\nTry '{} --help' for more information.",
                        ctcore::ct_util_name()
                    )
                );
            }
        }

        #[test]
        fn test_link_main_rejects_short_options() {
            for option in ["-h", "-V", "-abc"] {
                let args = [ctcore::ct_util_name(), option];
                let error = link_main(args.iter().map(OsString::from))
                    .expect_err("short options must fail");
                let option = option.trim_start_matches('-').chars().next().unwrap();
                assert_eq!(
                    error.to_string(),
                    format!(
                        "invalid option -- '{option}'\nTry '{} --help' for more information.",
                        ctcore::ct_util_name()
                    )
                );
            }
        }

        #[test]
        fn test_link_main_nonexistent_source() {
            let args = [ctcore::ct_util_name(), "nonexistent.txt", "target.txt"];
            let result = link_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_link_main_with_help_flag() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = link_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_link_main_empty_arguments() {
            let args = [ctcore::ct_util_name()];
            let result = link_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_link_main_with_spaces_in_paths() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("source file.txt");
            let target_file = temp_dir.path().join("target file.txt");
            File::create(&source_file).unwrap();

            let args = [
                ctcore::ct_util_name(),
                source_file.to_str().unwrap(),
                target_file.to_str().unwrap(),
            ];
            let result = link_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_main_with_relative_paths() {
            let (_temp_dir, source, target) = setup_test_files();
            let current_dir = std::env::current_dir().unwrap();
            let relative_source = Path::new(&source)
                .strip_prefix(&current_dir)
                .unwrap_or(Path::new(&source));
            let relative_target = Path::new(&target)
                .strip_prefix(&current_dir)
                .unwrap_or(Path::new(&target));

            let args = [
                ctcore::ct_util_name(),
                relative_source.to_str().unwrap(),
                relative_target.to_str().unwrap(),
            ];
            let result = link_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_main_with_unicode_paths() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("源文件.txt");
            let target_file = temp_dir.path().join("目标文件.txt");
            File::create(&source_file).unwrap();

            let args = [
                ctcore::ct_util_name(),
                source_file.to_str().unwrap(),
                target_file.to_str().unwrap(),
            ];
            let result = link_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_main_with_long_paths() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let long_name = "a".repeat(100);
            let source_file = temp_dir.path().join(format!("{long_name}.txt"));
            let target_file = temp_dir.path().join(format!("{long_name}_target.txt"));
            File::create(&source_file).unwrap();

            let args = [
                ctcore::ct_util_name(),
                source_file.to_str().unwrap(),
                target_file.to_str().unwrap(),
            ];
            let result = link_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
    }
}
