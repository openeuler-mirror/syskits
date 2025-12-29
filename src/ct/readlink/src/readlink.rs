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
}
