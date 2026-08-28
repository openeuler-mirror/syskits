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

//! readlink命令是Linux中用于读取符号链接（symlink）并显示其指向的文件或目录的命令。

extern crate rust_i18n;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_fs::{MissingHandling, ResolveMode, canonicalize};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_show_error;
use std::ffi::OsString;
use std::fs;
use std::io::{Write, stdout};
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

mod readlink_flags {
    pub const READLINK_CANONICALIZE: &str = "canonicalize";
    pub const READLINK_CANONICALIZE_MISSING: &str = "canonicalize-missing";
    pub const READLINK_CANONICALIZE_EXISTING: &str = "canonicalize-existing";
    pub const READLINK_NO_NEWLINE: &str = "no-newline";
    pub const READLINK_QUIET: &str = "quiet";
    pub const READLINK_SILENT: &str = "silent";
    pub const READLINK_VERBOSE: &str = "verbose";
    pub const READLINK_ZERO: &str = "zero";

    pub const READLINK_ARG_FILES: &str = "files";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadlinkMode {
    Readlink,
    Canonicalize,
    CanonicalizeExisting,
    CanonicalizeMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadlinkSemanticRow {
    pub input: String,
    pub resolved_path: String,
    pub mode: ReadlinkMode,
    pub no_newline: bool,
    pub zero: bool,
    pub quiet: bool,
    pub verbose: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadlinkSemantic {
    pub rows: Vec<ReadlinkSemanticRow>,
    pub classic_text: String,
}

struct ReadlinkOptions {
    files: Vec<String>,
    mode: ReadlinkMode,
    resolve_mode: ResolveMode,
    missing_handling: MissingHandling,
    quiet: bool,
    verbose: bool,
    line_ending: Option<CtLineEnding>,
    no_newline: bool,
    zero: bool,
}

impl ReadlinkOptions {
    fn from_matches(arg_matches: &ArgMatches) -> CTResult<Self> {
        let mut is_no_trailing_delimiter =
            arg_matches.get_flag(readlink_flags::READLINK_NO_NEWLINE);
        let is_use_zero = arg_matches.get_flag(readlink_flags::READLINK_ZERO);
        let is_silent = arg_matches.get_flag(readlink_flags::READLINK_SILENT)
            || arg_matches.get_flag(readlink_flags::READLINK_QUIET);

        let mut is_verbose = arg_matches.get_flag(readlink_flags::READLINK_VERBOSE);

        let mode = if arg_matches.get_flag(readlink_flags::READLINK_CANONICALIZE_EXISTING) {
            ReadlinkMode::CanonicalizeExisting
        } else if arg_matches.get_flag(readlink_flags::READLINK_CANONICALIZE_MISSING) {
            ReadlinkMode::CanonicalizeMissing
        } else if arg_matches.get_flag(readlink_flags::READLINK_CANONICALIZE) {
            ReadlinkMode::Canonicalize
        } else {
            ReadlinkMode::Readlink
        };

        let resolve_mode = match mode {
            ReadlinkMode::Readlink => ResolveMode::None,
            _ => ResolveMode::Logical,
        };

        if std::env::var_os("POSIXLY_CORRECT").is_some() && resolve_mode == ResolveMode::None {
            is_verbose = true;
        }
        if is_silent {
            is_verbose = false;
        }

        let missing_handling = match mode {
            ReadlinkMode::CanonicalizeExisting => MissingHandling::Existing,
            ReadlinkMode::CanonicalizeMissing => MissingHandling::Missing,
            _ => MissingHandling::Normal,
        };

        let files: Vec<String> = arg_matches
            .get_many::<String>(readlink_flags::READLINK_ARG_FILES)
            .map(|value| value.map(ToString::to_string).collect())
            .unwrap_or_default();
        if files.is_empty() {
            return Err(CTsageError::new(1, "missing operand"));
        }

        if is_no_trailing_delimiter && files.len() > 1 && !is_silent {
            ct_show_error!("ignoring --no-newline with multiple arguments");
            is_no_trailing_delimiter = false;
        }

        let line_ending = match is_no_trailing_delimiter {
            true => None,
            false => Some(CtLineEnding::from_zero_flag(is_use_zero)),
        };

        Ok(Self {
            files,
            mode,
            resolve_mode,
            missing_handling,
            quiet: is_silent,
            verbose: is_verbose,
            line_ending,
            no_newline: is_no_trailing_delimiter,
            zero: is_use_zero,
        })
    }
}

#[derive(Default)]
pub struct Readlink;
impl Tool for Readlink {
    fn name(&self) -> &'static str {
        "readlink"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        readlink_main(args.iter().cloned())
    }
}

pub fn readlink_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_matches = ct_app().try_get_matches_from(args)?;
    let options = ReadlinkOptions::from_matches(&arg_matches)?;

    for input in &options.files {
        let path_buf = PathBuf::from(input);
        let path_result = match options.resolve_mode {
            ResolveMode::None => fs::read_link(&path_buf),
            _ => canonicalize(&path_buf, options.missing_handling, options.resolve_mode),
        };

        match path_result {
            Ok(path) => {
                readlink_show(&path, options.line_ending).map_err_context(String::new)?;
            }
            Err(err) => {
                if options.verbose {
                    return Err(CtSimpleError::new(
                        1,
                        err.map_err_context(move || input.maybe_quote().to_string())
                            .to_string(),
                    ));
                }
                return Err(1.into());
            }
        }
    }
    Ok(())
}

pub fn readlink_native_semantic(args: impl ctcore::Args) -> CTResult<ReadlinkSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_matches = ct_app().try_get_matches_from(args)?;
    let options = ReadlinkOptions::from_matches(&arg_matches)?;

    let mut rows = Vec::with_capacity(options.files.len());
    let mut classic_text = String::new();

    for input in &options.files {
        let path_buf = PathBuf::from(input);
        let path_result = match options.resolve_mode {
            ResolveMode::None => fs::read_link(&path_buf),
            _ => canonicalize(&path_buf, options.missing_handling, options.resolve_mode),
        };

        match path_result {
            Ok(path) => {
                let resolved_path = path.to_string_lossy().to_string();
                classic_text.push_str(&resolved_path);
                if let Some(line_ending) = options.line_ending {
                    classic_text.push_str(&line_ending.to_string());
                }
                rows.push(ReadlinkSemanticRow {
                    input: input.clone(),
                    resolved_path,
                    mode: options.mode,
                    no_newline: options.no_newline,
                    zero: options.zero,
                    quiet: options.quiet,
                    verbose: options.verbose,
                });
            }
            Err(err) => {
                if options.verbose {
                    return Err(CtSimpleError::new(
                        1,
                        err.map_err_context(move || input.maybe_quote().to_string())
                            .to_string(),
                    ));
                }
                return Err(1.into());
            }
        }
    }

    Ok(ReadlinkSemantic { rows, classic_text })
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("readlink.about");
    let usage_description = t!("readlink.usage");
    let args = vec![
        Arg::new(readlink_flags::READLINK_CANONICALIZE)
            .short('f')
            .long(readlink_flags::READLINK_CANONICALIZE)
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively; all but the last component must exist",
            )
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_CANONICALIZE_EXISTING)
            .short('e')
            .long("canonicalize-existing")
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively, all components must exist",
            )
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_CANONICALIZE_MISSING)
            .short('m')
            .long(readlink_flags::READLINK_CANONICALIZE_MISSING)
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively, without requirements on components existence",
            )
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_NO_NEWLINE)
            .short('n')
            .long(readlink_flags::READLINK_NO_NEWLINE)
            .help(t!("readlink.clap.readlink_no_newline"))
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_QUIET)
            .short('q')
            .long(readlink_flags::READLINK_QUIET)
            .help(t!("readlink.clap.readlink_quiet"))
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_SILENT)
            .short('s')
            .long(readlink_flags::READLINK_SILENT)
            .help(t!("readlink.clap.readlink_silent"))
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_VERBOSE)
            .short('v')
            .long(readlink_flags::READLINK_VERBOSE)
            .help(t!("readlink.clap.readlink_verbose"))
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_ZERO)
            .short('z')
            .long(readlink_flags::READLINK_ZERO)
            .help(t!("readlink.clap.readlink_zero"))
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_ARG_FILES)
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

fn readlink_show(path: &Path, line_ending: Option<CtLineEnding>) -> std::io::Result<()> {
    let path = path.to_str().unwrap();
    print!("{path}");

    if let Some(line_ending) = line_ending {
        print!("{line_ending}");
    }
    stdout().flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Readlink;

        // 测试 name 方法
        assert_eq!(tool.name(), "readlink");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("readlink"));

        // 测试 execute 方法
        let args = vec![OsString::from("readlink"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod show_tests {
        use super::*;

        fn test_show_output(path: &str, line_ending: Option<CtLineEnding>, expected_output: &str) {
            let path = Path::new(path);
            let mut output = Vec::new();
            show_with_writer(path, line_ending, &mut output).unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), expected_output);
        }

        #[test]
        fn test_show_with_newline() {
            test_show_output("test/path", Some(CtLineEnding::Newline), "test/path\n");
        }

        #[test]
        fn test_show_with_null() {
            test_show_output("test/path", Some(CtLineEnding::Nul), "test/path\0");
        }

        #[test]
        fn test_show_without_line_ending() {
            test_show_output("test/path", None, "test/path");
        }

        #[test]
        fn test_show_empty_path() {
            test_show_output("", Some(CtLineEnding::Newline), "\n");
        }

        #[test]
        fn test_show_path_with_spaces() {
            test_show_output(
                "test path with spaces",
                Some(CtLineEnding::Newline),
                "test path with spaces\n",
            );
        }

        #[test]
        fn test_show_path_with_unicode() {
            test_show_output("测试/路径", Some(CtLineEnding::Newline), "测试/路径\n");
        }

        #[test]
        fn test_show_very_long_path() {
            let long_path = "a".repeat(1000);
            let expected_output = format!("{long_path}\n");
            test_show_output(&long_path, Some(CtLineEnding::Newline), &expected_output);
        }

        #[test]
        fn test_show_multiple_calls() {
            let path = "repeated_call";
            let mut output = Vec::new();
            let line_ending = Some(CtLineEnding::Newline);
            for _ in 0..3 {
                show_with_writer(Path::new(path), line_ending, &mut output).unwrap();
            }
            let expected_output = format!("{path}\n{path}\n{path}\n");
            assert_eq!(String::from_utf8(output).unwrap(), expected_output);
        }

        fn show_with_writer(
            path: &Path,
            line_ending: Option<CtLineEnding>,
            writer: &mut dyn Write,
        ) -> std::io::Result<()> {
            let path = path.to_str().unwrap();
            write!(writer, "{path}")?;
            if let Some(line_ending) = line_ending {
                write!(writer, "{line_ending}")?;
            }
            writer.flush()
        }
    }
    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::fs::File;
        use std::os::unix::fs::symlink;
        use tempfile::tempdir;
        #[test]
        fn test_readlink_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = readlink_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = readlink_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_help_short() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_canonicalize_long() {
            let filename = "test_readlink_main_canonicalize_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--canonicalize", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_short() {
            let filename = "test_readlink_main_canonicalize_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-f", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_existing_long() {
            let filename = "test_readlink_main_canonicalize_existing_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--canonicalize-existing", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_existing_short() {
            let filename = "test_readlink_main_canonicalize_existing_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-e", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_missing_long() {
            let filename = "test_readlink_main_canonicalize_existing_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--canonicalize-missing", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_missing_short() {
            let filename = "test_readlink_main_canonicalize_missing_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-m", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_readlink_main_no_newline_long() {
            let filename = "test_readlink_main_no_newline_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--no-newline", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_no_newline_short() {
            let filename = "test_readlink_main_no_newline_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-n", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_quiet_long() {
            let filename = "test_readlink_main_quiet_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--quiet", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_quiet_short() {
            let filename = "test_readlink_main_quiet_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-q", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_silent_short() {
            let filename = "test_readlink_main_quiet_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-s", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_silent_long() {
            let filename = "test_readlink_main_silent_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--silent", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_verbose_long() {
            let filename = "test_readlink_main_verbose_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--verbose", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_verbose_short() {
            let filename = "test_readlink_main_verbose_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-v", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_zero_long() {
            let filename = "test_readlink_main_zero_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--zero", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_zero_short() {
            let filename = "test_readlink_main_zero_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-z", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        // -->         let symlink_path = tmp_dir.path().join("symlink_dir");
        //             symlink(&dir_path, &symlink_path).unwrap();

        #[test]
        fn test_readlink_main_no_newline_long_with_symlink() {
            let filename = "test_readlink_main_no_newline_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--no-newline", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_no_newline_short_with_symlink() {
            let filename = "test_readlink_main_no_newline_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-n", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_quiet_long_with_symlink() {
            let filename = "test_readlink_main_quiet_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--quiet", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_quiet_short_with_symlink() {
            let filename = "test_readlink_main_quiet_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-q", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_silent_short_with_symlink() {
            let filename = "test_readlink_main_silent_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-s", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_silent_long_with_symlink() {
            let filename = "test_readlink_main_silent_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--silent", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_verbose_long_with_symlink() {
            let filename = "test_readlink_main_verbose_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--verbose", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_verbose_short_with_symlink() {
            let filename = "test_readlink_main_verbose_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-v", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_zero_long_with_symlink() {
            let filename = "test_readlink_main_zero_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--zero", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_zero_short_with_symlink() {
            let filename = "test_readlink_main_zero_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-z", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
    }
    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // readlink 接口: readlink [OPTION]... FILE...
        //
        // Arguments:
        //   [files]...
        //
        // Options:
        //   -f, --canonicalize           canonicalize by following every symlink in every component of the given name recursively; all but the last component must exist
        //   -e, --canonicalize-existing  canonicalize by following every symlink in every component of the given name recursively, all components must exist
        //   -m, --canonicalize-missing   canonicalize by following every symlink in every component of the given name recursively, without requirements on components existence
        //   -n, --no-newline             do not output the trailing delimiter
        //   -q, --quiet                  suppress most error messages
        //   -s, --silent                 suppress most error messages
        //   -v, --verbose                report error message
        //   -z, --zero                   separate output with NUL rather than newline
        //   -h, --help                   Print help
        //   -V, --version                Print version

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

            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_long() {
            let file_name = "test_ct_app_canonicalize_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--canonicalize", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_short() {
            let file_name = "test_ct_app_canonicalize_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-f", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_existing_long() {
            let file_name = "test_ct_app_canonicalize_existing_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--canonicalize-existing", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_existing_short() {
            let file_name = "test_ct_app_canonicalize_existing_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-e", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_missing_long() {
            let file_name = "test_ct_app_canonicalize_existing_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--canonicalize-missing", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_missing_short() {
            let file_name = "test_ct_app_canonicalize_missing_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-m", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_newline_long() {
            let file_name = "test_ct_app_no_newline_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--no-newline", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_newline_short() {
            let file_name = "test_ct_app_no_newline_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-n", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_quiet_long() {
            let file_name = "test_ct_app_quiet_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--quiet", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_quiet_short() {
            let file_name = "test_ct_app_quiet_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-q", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_silent_short() {
            let file_name = "test_ct_app_quiet_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-s", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_silent_long() {
            let file_name = "test_ct_app_silent_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--silent", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_verbose_long() {
            let file_name = "test_ct_app_verbose_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--verbose", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_verbose_short() {
            let file_name = "test_ct_app_verbose_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-v", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_zero_long() {
            let file_name = "test_ct_app_zero_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--zero", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_zero_short() {
            let file_name = "test_ct_app_zero_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-z", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }
    }
}
