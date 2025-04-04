// This file is part of the cttils coreutils package.
//
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.

use clap::{Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::ct_println_verbatim;
use ctcore::ct_error::{CTResult, FromIo};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
use std::ffi::OsString;
mod platform;

const WHOAMI_ABOUT: &str = ct_help_about!("whoami.md");
const WHOAMI_USAGE: &str = ct_help_usage!("whoami.md");

pub fn ctmain(args: impl ctcore::Args) -> i32 {
    pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
        whoami_main(args).map(|_| ())
    }

    let result = ctmain(args);
    match result {
        Ok(()) => ctcore::ct_error::get_ct_exit_code(),
        Err(err) => {
            let s_err = {
                let res = format!("{}", err);
                res
            };
            if !s_err.is_empty() {
                {
                    eprintln!("{}: ", ctcore::ct_util_name());
                    eprintln!("{}", s_err);
                }
            }
            if err.usage() {
                eprintln!(
                    "Try '{} --help' for more information.",
                    ctcore::ct_execute_phrase()
                );
            }
            err.code()
        }
    }
}

pub fn whoami_main(args: impl ctcore::Args) -> CTResult<String> {
    ct_app().try_get_matches_from(args)?;
    let username = whoami_exec()?;
    ct_println_verbatim(username.clone()).map_err_context(|| "failed to print username".into())?;

    let result = username.into_string().unwrap();
    Ok(result)
}

/// 获取当前用户名
pub fn whoami_exec() -> CTResult<OsString> {
    let username_result = platform::get_username();

    username_result.map_err_context(|| "failed to get username".into())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = WHOAMI_ABOUT;
    let usage_description = ct_format_usage(WHOAMI_USAGE);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
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

    #[cfg(test)]
    mod tests {
        use std::ffi::OsString;

        use clap::error::ErrorKind;

        use super::*;

        #[test]
        fn test_ctmain_input_h() {
            {
                let args = ["-h", ""];
                let result = ctmain(args.iter().map(|s| OsString::from(s)));
                println!("{}", result);
                assert_eq!(result, 1);
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
                let result = ctmain(args.iter().map(|s| OsString::from(s)));
                println!("{}", result);
                assert_eq!(result, 1);
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
                let result = ctmain(args.iter().map(|s| OsString::from(s)));
                println!("{}", result);
                assert_eq!(result, 1);
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
            let expected = if let Some(username) = std::env::var("USER").ok() {
                username
            } else {
                "root".to_string()
            };
            let args = vec![ctcore::ct_util_name()];
            let result = whoami_main(args.iter().map(|s| OsString::from(s)));
            let mut s = String::new();
            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    s = output.to_string();
                    println!("result:{}", s);
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
