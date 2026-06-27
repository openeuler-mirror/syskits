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

//dirname命令主要用于从给定的文件或目录路径中剥离出目录部分，去掉路径末尾的文件名（或最后一个组件），仅保留上级目录的路径。

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::ct_print_verbatim;
use ctcore::ct_error::{CTResult, CTsageError};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
use std::ffi::OsString;
use std::path::Path;

const DIRNAME_ABOUT: &str = ct_help_about!("dirname.md");
const DIRNAME_USAGE: &str = ct_help_usage!("dirname.md");
const DIRNAME_AFTER_HELP: &str = ct_help_section!("after help", "dirname.md");

mod opt_flags {
    pub const ZERO: &str = "zero";
    pub const DIR: &str = "dir";
}

#[derive(Default)]
pub struct Dirname;
impl Tool for Dirname {
    fn name(&self) -> &'static str {
        "dirname"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        dirname_main(args.iter().cloned())
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    dirname_main(args).map(|_| ())
}

pub fn dirname_main(args: impl ctcore::Args) -> CTResult<()> {
    let args_match = ct_app()
        .after_help(DIRNAME_AFTER_HELP)
        .try_get_matches_from(args)?;

    let line_ending = CtLineEnding::from_zero_flag(args_match.get_flag(opt_flags::ZERO));

    let dirnames: Vec<String> = args_match
        .get_many::<String>(opt_flags::DIR)
        .unwrap_or_default()
        .cloned()
        .collect();

    if let Some(value) = dirname_process(line_ending, &dirnames) {
        return value;
    }

    Ok(())
}

fn dirname_process(line_ending: CtLineEnding, dirnames: &Vec<String>) -> Option<CTResult<()>> {
    if dirnames.is_empty() {
        return Some(Err(CTsageError::new(1, "missing operand")));
    } else {
        for item in dirnames {
            let path = Path::new(item);
            match path.parent() {
                Some(dir) => {
                    if dir.components().next().is_none() {
                        print!(".");
                    } else {
                        ct_print_verbatim(dir).unwrap();
                    }
                }
                None => {
                    if path.is_absolute() || item == "/" {
                        print!("/");
                    } else {
                        print!(".");
                    }
                }
            }
            print!("{line_ending}");
        }
    }
    None
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = DIRNAME_ABOUT;
    let usage_description = ct_format_usage(DIRNAME_USAGE);

    let args = vec![
        Arg::new(opt_flags::ZERO)
            .long(opt_flags::ZERO)
            .short('z')
            .help("separate output with NUL rather than newline")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::DIR)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .about(application_info)
        .version(command_version)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Dirname::default();

        // Test name method
        assert_eq!(tool.name(), "dirname");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("dirname"));

        // Test execute method - should fail without arguments
        let args = vec![OsString::from("dirname"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());

        // Test execute method with valid argument
        let args = vec![OsString::from("dirname"), OsString::from("/path/to/file")];
        assert!(tool.execute(&args).is_ok());
    }

    mod tests_dirname_main {
        use crate::dirname_main;

        use std::ffi::OsString;

        #[test]
        fn test_dirname_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = dirname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_dirname_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = dirname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_dirname_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = dirname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_dirname_main_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = dirname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_dirname_main_z() {
            let args = vec![ctcore::ct_util_name(), "-z", "3/etc/audi-efwe/few/35/2"];
            let result = dirname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dirname_main_zero() {
            let args = vec![
                ctcore::ct_util_name(),
                "--zero",
                " 3/etc/audi-efwe/few/35/2",
            ];
            let result = dirname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }

    mod tests_ct_app {
        use crate::ct_app;

        use crate::opt_flags::ZERO;
        use clap::error::ErrorKind;

        #[test]
        fn test_dirname_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
        #[test]
        fn test_dirname_zpp_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_dirname_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_dirname_app_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_dirname_app_z() {
            let args = vec![ctcore::ct_util_name(), "-z", "3/etc/audi-efwe/few/35/2"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(ZERO));
        }

        #[test]
        fn test_dirname_app_zero() {
            let args = vec![ctcore::ct_util_name(), "--zero", "3/etc/audi-efwe/few/35/2"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(ZERO));
        }
    }

    mod tests_dirname_process {

        use crate::dirname_process;
        use ctcore::ct_line_ending::CtLineEnding;

        use std::vec;

        #[test]
        fn test_dirname_process_with_empty_dirnames() {
            let line_ending = CtLineEnding::default();
            let dirnames = vec![];
            let result = dirname_process(line_ending, &dirnames);

            assert!(result.is_some());
            assert!(result.unwrap().is_err());
        }

        #[test]
        fn test_dirname_process_with_non_empty_dirnames() {
            let line_ending = CtLineEnding::default();
            let dirnames = vec!["dir1", "dir2", "dir3"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>();

            let result = dirname_process(line_ending, &dirnames);

            assert!(result.is_none());
        }
    }
}
