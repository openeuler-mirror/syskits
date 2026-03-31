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

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::{ct_error::CTResult, ct_format_usage, ct_help_about, ct_help_usage};
use std::env;

const PRINTENV_ABOUT: &str = ct_help_about!("printenv.md");
const PRINTENV_SAGE: &str = ct_help_usage!("printenv.md");

static PRINTENV_OPT_NULL: &str = "null";

static PRINTENV_ARG_VARIABLES: &str = "variables";

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    printenv_main(args).map(|_| ())
}
/// 主函数用于打印环境变量。
///
/// # 参数
/// `args`: 实现了 `ctcore::Args` 的参数对象，用于解析命令行参数。
///
/// # 返回值
/// 返回一个 `CTResult<()>`，成功则为 `Ok(())`，失败则为 `Err(1.into())`。
pub fn printenv_main(args: impl ctcore::Args) -> CTResult<()> {
    // 从命令行参数中获取匹配项
    let args_match = ct_app().get_matches_from(args);

    // 解析命令行参数中指定的环境变量名列表
    let var: Vec<String> = args_match
        .get_many::<String>(PRINTENV_ARG_VARIABLES)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    // 根据命令行参数决定环境变量值之间的分隔符
    let separator = if args_match.get_flag(PRINTENV_OPT_NULL) {
        "\x00"
    } else {
        "\n"
    };

    // 若未指定环境变量名，则打印所有环境变量
    if var.is_empty() {
        for (env_var, value) in env::vars() {
            print!("{env_var}={value}{separator}");
        }
        return Ok(());
    }

    // 检查并处理指定的环境变量
    let mut error_found = false;

    printenv_processing(var, separator, &mut error_found)
}

fn printenv_processing(var: Vec<String>, separator: &str, error_found: &mut bool) -> CTResult<()> {
    for env_var in var {
        // 忽略形如 "a=b" 的变量，但对此发出错误
        if env_var.contains('=') {
            *error_found = true;
            continue;
        }
        // 尝试获取环境变量的值并打印
        if let Ok(var) = env::var(env_var) {
            print!("{var}{separator}");
        } else {
            // 若环境变量不存在，则标记错误
            *error_found = true;
        }
    }

    // 若存在错误，则返回错误码
    if *error_found { Err(1.into()) } else { Ok(()) }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = PRINTENV_ABOUT;
    let usage_description = ct_format_usage(PRINTENV_SAGE);

    let args = vec![
        Arg::new(PRINTENV_OPT_NULL)
            .short('0')
            .long(PRINTENV_OPT_NULL)
            .help("end each output line with 0 byte rather than newline")
            .action(ArgAction::SetTrue),
        Arg::new(PRINTENV_ARG_VARIABLES)
            .action(ArgAction::Append)
            .num_args(1..),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

#[cfg(test)]
mod tests {

    mod tests_printenv_main {
        use crate::printenv_main;

        use std::ffi::OsString;

        #[test]
        fn test_printenv_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = printenv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = printenv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = printenv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = printenv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main() {
            let args = vec![ctcore::ct_util_name()];
            let result = printenv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }

    mod tests_printenv_app {
        use crate::ct_app;

        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
    }
}
