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

//! unlink 命令用于删除一个文件或者一个文件的硬链接

extern crate rust_i18n;
use clap::builder::ValueParser;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use clap::{Arg, Command, crate_version};

use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, FromIo};

use std::ffi::OsString;
use std::fs::remove_file;
use std::path::Path;
use sys_locale::get_locale;

static OPT_PATH: &str = "FILE";

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    unlink_main(args)
}

pub fn unlink_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;

    let path: &Path = matches.get_one::<OsString>(OPT_PATH).unwrap().as_ref();

    remove_file(path).map_err_context(|| format!("cannot unlink {}", path.quote()))
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("unlink.about");
    let usage_description = t!("unlink.usage");
    let arg = Arg::new(OPT_PATH)
        .required(true)
        .hide(true)
        .value_parser(ValueParser::os_string())
        .value_hint(clap::ValueHint::AnyPath);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
}

#[derive(Default)]
pub struct Unlink;
impl Tool for Unlink {
    fn name(&self) -> &'static str {
        "unlink"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        unlink_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Unlink::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "unlink");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("unlink"));

        // 测试 execute 方法
        let args = vec![OsString::from("unlink"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err()); // unlink需要参数，没有参数会失败
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::fs::File;
        use std::path::PathBuf;

        use super::*;

        #[test]
        fn test_unlink_main_argument_file_parsing() {
            let regular_file_path = "test_unlink_main_argument_file_parsing";
            File::create(regular_file_path).expect("Failed to create file");
            let args = vec![ctcore::ct_util_name(), regular_file_path];
            let result = unlink_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(_output) => {
                    assert!(!PathBuf::from(regular_file_path).exists());
                }
            }
        }

        #[test]
        fn test_unlink_main_argument_no_file_parsing() {
            let regular_file_path = "test_no_unlink_file";

            let args = vec![ctcore::ct_util_name(), regular_file_path];
            let result = unlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_argument_default() {
            let args = vec![ctcore::ct_util_name()];
            let result = unlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = unlink_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = unlink_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = unlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = unlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = unlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_unlink_main_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = unlink_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use std::fs;
        use std::fs::File;

        use clap::error::ErrorKind;

        use super::*;

        // unlink 接口, unlink FILE
        //       --help     display this help and exit
        //       --version  output version information and exit

        #[test]
        fn test_ct_app_argument_file_parsing() {
            // Create a file for testing , 默认带文件
            let regular_file_path = "test_ct_app_argument_file_parsing";
            File::create(regular_file_path).expect("Failed to create file");

            let command = ct_app();
            // 测试正确的文件路径参数解析
            let args = vec![ctcore::ct_util_name(), regular_file_path];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            fs::remove_file(regular_file_path).expect("Failed to remove file");
        }

        #[test]
        fn test_ct_app_argument_no_file_parsing() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name()];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
            assert_eq!(
                executable.unwrap_err().kind(),
                ErrorKind::MissingRequiredArgument
            );
        }

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
            assert!(result.is_err());
        }
    }
}
