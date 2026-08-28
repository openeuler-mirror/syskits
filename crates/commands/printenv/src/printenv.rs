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
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::{Tool, ct_error::CTResult};
use std::env;
use std::ffi::OsString;
use sys_locale::get_locale;

static PRINTENV_OPT_NULL: &str = "null";

static PRINTENV_ARG_VARIABLES: &str = "variables";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintenvRow {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintenvSemantic {
    pub rows: Vec<PrintenvRow>,
    pub classic_text: String,
    pub exit_code: i32,
}

struct PrintenvOptions {
    separator: &'static str,
    variables: Vec<String>,
}

impl PrintenvOptions {
    fn from_matches(args_match: &ArgMatches) -> Self {
        let variables: Vec<String> = args_match
            .get_many::<String>(PRINTENV_ARG_VARIABLES)
            .map(|v| v.map(ToString::to_string).collect())
            .unwrap_or_default();
        let null = args_match.get_flag(PRINTENV_OPT_NULL);
        Self {
            separator: if null { "\x00" } else { "\n" },
            variables,
        }
    }
}

/// 主函数用于打印环境变量。
///
/// # 参数
/// `args`: 实现了 `ctcore::Args` 的参数对象，用于解析命令行参数。
///
/// # 返回值
/// 返回一个 `CTResult<()>`，成功则为 `Ok(())`，失败则为 `Err(1.into())`。
pub fn printenv_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 从命令行参数中获取匹配项
    let args_match = ct_app().get_matches_from(args);
    let options = PrintenvOptions::from_matches(&args_match);
    let semantic = printenv_semantic_from_options(&options);
    print!("{}", semantic.classic_text);
    if semantic.exit_code == 0 {
        Ok(())
    } else {
        Err(semantic.exit_code.into())
    }
}

pub fn printenv_native_semantic(args: impl ctcore::Args) -> CTResult<PrintenvSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app().get_matches_from(args);
    let options = PrintenvOptions::from_matches(&args_match);
    Ok(printenv_semantic_from_options(&options))
}

fn printenv_semantic_from_options(options: &PrintenvOptions) -> PrintenvSemantic {
    let mut rows = Vec::new();
    let mut classic_text = String::new();
    let mut error_found = false;

    if options.variables.is_empty() {
        for (name, value) in env::vars() {
            classic_text.push_str(&name);
            classic_text.push('=');
            classic_text.push_str(&value);
            classic_text.push_str(options.separator);
            rows.push(PrintenvRow { name, value });
        }
        return PrintenvSemantic {
            rows,
            classic_text,
            exit_code: 0,
        };
    }

    for variable in &options.variables {
        if variable.contains('=') {
            error_found = true;
            continue;
        }

        if let Ok(value) = env::var(variable) {
            classic_text.push_str(&value);
            classic_text.push_str(options.separator);
            rows.push(PrintenvRow {
                name: variable.clone(),
                value,
            });
        } else {
            error_found = true;
        }
    }

    PrintenvSemantic {
        rows,
        classic_text,
        exit_code: if error_found { 1 } else { 0 },
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("printenv.about");
    let usage_description = t!("printenv.usage");

    let args = vec![
        Arg::new(PRINTENV_OPT_NULL)
            .short('0')
            .long(PRINTENV_OPT_NULL)
            .help(t!("printenv.clap.printenv_opt_null"))
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

#[derive(Default)]
pub struct Printenv;
impl Tool for Printenv {
    fn name(&self) -> &'static str {
        "printenv"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        printenv_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Printenv;

        // 测试 name 方法
        assert_eq!(tool.name(), "printenv");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("printenv"));

        // 测试 execute 方法
        let args = vec![OsString::from("printenv"), OsString::from("--version")];
        assert!(tool.execute(&args).is_ok());
    }

    mod tests_printenv_main {
        use crate::printenv_main;

        use std::ffi::OsString;

        #[test]
        fn test_printenv_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = printenv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = printenv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = printenv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = printenv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_printenv_main() {
            let args = [ctcore::ct_util_name()];
            let result = printenv_main(args.iter().map(OsString::from));

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
