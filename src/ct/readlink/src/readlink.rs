/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! readlink命令是Linux中用于读取符号链接（symlink）并显示其指向的文件或目录的命令。

use clap::{crate_version, Arg, ArgAction, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_fs::{canonicalize, MissingHandling, ResolveMode};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show_error};
use std::fs;
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};

const READLINK_ABOUT: &str = ct_help_about!("readlink.md");
const READLINK_USAGE: &str = ct_help_usage!("readlink.md");
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

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    readlink_main(args)
}
pub fn readlink_main(args: impl ctcore::Args) -> CTResult<()> {
    let arg_matches = ct_app().try_get_matches_from(args)?;

    let mut is_no_trailing_delimiter = arg_matches.get_flag(readlink_flags::READLINK_NO_NEWLINE);
    let is_use_zero = arg_matches.get_flag(readlink_flags::READLINK_ZERO);
    let is_silent = arg_matches.get_flag(readlink_flags::READLINK_SILENT)
        || arg_matches.get_flag(readlink_flags::READLINK_QUIET);
    let is_verbose = arg_matches.get_flag(readlink_flags::READLINK_VERBOSE);

    let resovle_mode = if arg_matches.get_flag(readlink_flags::READLINK_CANONICALIZE)
        || arg_matches.get_flag(readlink_flags::READLINK_CANONICALIZE_EXISTING)
        || arg_matches.get_flag(readlink_flags::READLINK_CANONICALIZE_MISSING)
    {
        ResolveMode::Logical
    } else {
        ResolveMode::None
    };

    let miss_handle = if arg_matches.get_flag(readlink_flags::READLINK_CANONICALIZE_EXISTING) {
        MissingHandling::Existing
    } else if arg_matches.get_flag(readlink_flags::READLINK_CANONICALIZE_MISSING) {
        MissingHandling::Missing
    } else {
        MissingHandling::Normal
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

    for f in &files {
        let path_buf = PathBuf::from(f);
        let path_result = match resovle_mode {
            ResolveMode::None => fs::read_link(&path_buf),
            _ => canonicalize(&path_buf, miss_handle, resovle_mode),
        };

        match path_result {
            Ok(path) => {
                readlink_show(&path, line_ending).map_err_context(String::new)?;
            }
            Err(err) => {
                if is_verbose {
                    return Err(CtSimpleError::new(
                        1,
                        err.map_err_context(move || f.maybe_quote().to_string())
                            .to_string(),
                    ));
                } else {
                    return Err(1.into());
                }
            }
        }
    }
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = READLINK_ABOUT;
    let usage_description = ct_format_usage(READLINK_USAGE);
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
            .help("do not output the trailing delimiter")
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_QUIET)
            .short('q')
            .long(readlink_flags::READLINK_QUIET)
            .help("suppress most error messages")
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_SILENT)
            .short('s')
            .long(readlink_flags::READLINK_SILENT)
            .help("suppress most error messages")
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_VERBOSE)
            .short('v')
            .long(readlink_flags::READLINK_VERBOSE)
            .help("report error message")
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_ZERO)
            .short('z')
            .long(readlink_flags::READLINK_ZERO)
            .help("separate output with NUL rather than newline")
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

    #[cfg(test)]
    mod show_tests {
        use super::*;

        fn test_show_output(path: &str, line_ending: Option<CtLineEnding>, expected_output: &str) {
            let path = Path::new(path);
            let mut output = Vec::new();
            show_with_writer(&path, line_ending, &mut output).unwrap();
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
            let expected_output = format!("{}\n", long_path);
            test_show_output(&long_path, Some(CtLineEnding::Newline), &expected_output);
        }

        #[test]
        fn test_show_multiple_calls() {
            let path = "repeated_call";
            let mut output = Vec::new();
            let line_ending = Some(CtLineEnding::Newline);
            for _ in 0..3 {
                show_with_writer(&Path::new(path), line_ending, &mut output).unwrap();
            }
            let expected_output = format!("{0}\n{0}\n{0}\n", path);
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
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = readlink_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_help_short() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_canonicalize_long() {
            let filename = "test_readlink_main_canonicalize_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--canonicalize", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_short() {
            let filename = "test_readlink_main_canonicalize_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-f", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        
        #[test]
        fn test_readlink_main_canonicalize_existing_long() {
            let filename = "test_readlink_main_canonicalize_existing_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--canonicalize-existing", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_existing_short() {
            let filename = "test_readlink_main_canonicalize_existing_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-e", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_missing_long() {
            let filename = "test_readlink_main_canonicalize_existing_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--canonicalize-missing", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_missing_short() {
            let filename = "test_readlink_main_canonicalize_missing_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-m", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }
        #[test]
        fn test_readlink_main_no_newline_long() {
            let filename = "test_readlink_main_no_newline_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--no-newline", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_no_newline_short() {
            let filename = "test_readlink_main_no_newline_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-n", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_quiet_long() {
            let filename = "test_readlink_main_quiet_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--quiet", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_quiet_short() {
            let filename = "test_readlink_main_quiet_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-q", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_silent_short() {
            let filename = "test_readlink_main_quiet_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-s", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_silent_long() {
            let filename = "test_readlink_main_silent_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--silent", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_verbose_long() {
            let filename = "test_readlink_main_verbose_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--verbose", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_verbose_short() {
            let filename = "test_readlink_main_verbose_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-v", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_zero_long() {
            let filename = "test_readlink_main_zero_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--zero", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_zero_short() {
            let filename = "test_readlink_main_zero_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-z", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "--no-newline", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "-n", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "--quiet", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "-q", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "-s", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "--silent", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "--verbose", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "-v", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "--zero", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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

            let args = vec![ctcore::ct_util_name(), "-z", file_name];
            let result = readlink_main(args.iter().map(|s| OsString::from(s)));
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
