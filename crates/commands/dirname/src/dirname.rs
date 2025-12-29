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

//dirname命令主要用于从给定的文件或目录路径中剥离出目录部分，去掉路径末尾的文件名（或最后一个组件），仅保留上级目录的路径。

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
use ctcore::ct_display::ct_print_verbatim;
use ctcore::ct_error::{CTResult, CTsageError};
use ctcore::ct_line_ending::CtLineEnding;
use std::ffi::OsString;
use std::path::Path;
use sys_locale::get_locale;

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

pub fn dirname_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app()
        .after_help(t!("dirname.after_help"))
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
            let dirname = compute_dirname(item);
            ct_print_verbatim(&dirname).unwrap();
            print!("{line_ending}");
        }
    }
    None
}

pub fn compute_dirname(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    // 1. 去掉末尾所有 '/'
    let mut s = path.trim_end_matches('/');
    // 如果去掉后变为空（说明全是 /），设为 "/"
    if s.is_empty() {
        return "/".to_string();
    }
    // 2. 找最后一个 '/'
    if let Some(pos) = s.rfind('/') {
        // 截断到该 '/'
        s = &s[..pos];

        // 3. 再去掉末尾 '/'
        s = s.trim_end_matches('/');

        // 4. 处理截断结果为空的情况
        if s.is_empty() {
            return "/".to_string();
        } else {
            return s.to_string();
        }
    } else {
        // 没有 '/' → 返回 "."
        return ".".to_string();
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("dirname.about");
    let usage_description = t!("dirname.usage");

    let args = vec![
        Arg::new(opt_flags::ZERO)
            .long(opt_flags::ZERO)
            .short('z')
            .help(t!("dirname.clap.zero"))
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
