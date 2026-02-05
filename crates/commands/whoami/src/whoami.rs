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

extern crate rust_i18n;
use clap::{Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::ct_println_verbatim;
use ctcore::ct_error::{CTResult, FromIo};
use std::ffi::OsString;
use sys_locale::get_locale;

mod platform;

pub fn whoami_main(args: impl ctcore::Args) -> CTResult<String> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    ct_app().try_get_matches_from(args)?;
    let username = whoami_exec()?;
    ct_println_verbatim(username.clone())
        .map_err_context(|| t!("whoami.errors.failed_print_username"))?;

    let result = username.into_string().unwrap();
    Ok(result)
}

/// 获取当前用户名
pub fn whoami_exec() -> CTResult<OsString> {
    let username_result = platform::get_username();

    username_result.map_err_context(|| t!("whoami.errors.failed_get_username"))
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("whoami.about");
    let usage_description = t!("whoami.usage");

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            clap::Arg::new("help")
                .short('h')
                .long("help")
                .help(t!("whoami.clap.help"))
                .action(clap::ArgAction::Help),
        )
        .arg(
            clap::Arg::new("version")
                .short('V')
                .long("version")
                .help(t!("whoami.clap.version"))
                .action(clap::ArgAction::Version),
        )
}

#[derive(Default)]
pub struct Whoami;
impl Tool for Whoami {
    fn name(&self) -> &'static str {
        "whoami"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        whoami_main(args.iter().cloned()).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;

    use super::*;
    rust_i18n::i18n!("locales", fallback = "en-US");

    #[test]
    fn test_tool_implementation() {
        let tool = Whoami;

        // Test name method
        assert_eq!(tool.name(), "whoami");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("whoami"));

        // Test execute method with help flag (should work)
        let args = vec![OsString::from("whoami"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use clap::error::ErrorKind;
        use std::ffi::OsString;

        #[test]
        fn test_ctmain_input_h() {
            {
                let args = ["-h", ""];
                let result = whoami_main(args.iter().map(OsString::from));
                assert!(result.is_err());
            }

            {
                let command = ct_app();
                let args = vec![ctcore::ct_util_name(), "-h"];

                let result = command.try_get_matches_from(args);
                assert!(result.is_err());
                assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
            }
        }

        #[test]
        fn test_ctmain_input_v() {
            {
                let args = ["--version", ""];
                let result = whoami_main(args.iter().map(OsString::from));
                assert!(result.is_err());
            }
            {
                let command = ct_app();
                let args = vec![ctcore::ct_util_name(), "--version"];
                // let result = ct_main(args.iter().map(|s| OsString::from(s)));

                let result = command.try_get_matches_from(args);
                assert!(result.is_err());
                assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
            }
        }

        #[test]
        fn test_ctmain_input_uppercase_v() {
            {
                let args = ["-V", ""];
                let result = whoami_main(args.iter().map(OsString::from));
                assert!(result.is_err());
            }
            {
                let command = ct_app();
                let args = vec![ctcore::ct_util_name(), "-V"];
                // let result = ct_main(args.iter().map(|s| OsString::from(s)));

                let result = command.try_get_matches_from(args);
                assert!(result.is_err());
                assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
            }
        }

        #[test]
        fn test_ctmain_return() {
            // println!("当前操作系统架构：{}", expected_arch);
            let expected = if let Ok(username) = std::env::var("USER") {
                username
            } else {
                "root".to_string()
            };
            let args = [ctcore::ct_util_name()];
            let result = whoami_main(args.iter().map(OsString::from));
            let mut s = String::new();
            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {code}");
                    println!("Error message: {message}");
                }
                Ok(output) => {
                    s = output.to_string();
                    println!("result:{s}");
                    // //assert_eq!(s,expected_output);
                }
            }
            assert_eq!(s, expected);
        }
    }

    ///////////////////////////////

    // whoami 接口: whoami [OPTION]...
    //       --help     display this help and exit
    //       --version  output version information and exit
    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--version"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_other_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-V"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
        let help_args = vec![ctcore::ct_util_name(), "--help"];
        let result = command.try_get_matches_from(help_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_ct_app_execution_unsupport_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
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
