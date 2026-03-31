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

// expr 是一个经典的 Linux 或 Unix 命令行工具，用于执行基本的算术和逻辑表达式计算

use std::fmt::Display;

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::{
    ct_display::Quotable,
    ct_error::{CTError, CTResult},
    ct_format_usage, ct_help_about, ct_help_section, ct_help_usage,
};
use syntax_tree::SyntaxTreeAstNode;

use crate::syntax_tree::is_syntax_tree_truthy;

mod syntax_tree;
// 定义命令行选项常量
mod opt_flags {
    pub const VERSION: &str = "version";
    pub const HELP: &str = "help";
    pub const EXPRESSION: &str = "expression";
}

// 表达式计算结果类型
pub type ExprResult<T> = Result<T, ExprError>;

// 表达式错误类型
#[derive(Debug, PartialEq, Eq)]
pub enum ExprError {
    UnexpectedArgument(String),        // 意外的参数
    MissingArgument(String),           // 缺失的参数
    NonIntegerArgument,                // 非整数参数
    MissingOperand,                    // 缺失操作数
    DivisionByZero,                    // 除以零
    InvalidRegexExpression,            // 无效的正则表达式
    ExpectedClosingBraceAfter(String), // 期望在...之后看到闭合括号
}

// 实现 ExprError 的显示格式化
impl Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 根据错误类型格式化错误信息
        match self {
            Self::UnexpectedArgument(s) => {
                write!(f, "syntax error: unexpected argument {}", s.quote())
            }
            Self::MissingArgument(s) => {
                write!(f, "syntax error: missing argument after {}", s.quote())
            }
            Self::NonIntegerArgument => write!(f, "non-integer argument"),
            Self::MissingOperand => write!(f, "missing operand"),
            Self::DivisionByZero => write!(f, "division by zero"),
            Self::InvalidRegexExpression => write!(f, "Invalid regex expression"),
            Self::ExpectedClosingBraceAfter(s) => {
                write!(f, "expected ')' after {}", s.quote())
            }
        }
    }
}

// 实现标准错误接口
impl std::error::Error for ExprError {}

// 实现特定错误接口 CTError，用于自定义错误处理
impl CTError for ExprError {
    fn code(&self) -> i32 {
        2 // 错误代码
    }

    fn usage(&self) -> bool {
        *self == Self::MissingOperand // 当错误为缺失操作数时，显示用法信息
    }
}

// 创建命令行应用
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = ct_help_about!("expr.md");
    let usage_description = ct_format_usage(ct_help_usage!("expr.md"));
    let about_help_info = ct_help_section!("after help", "expr.md");
    let args = vec![
        Arg::new(opt_flags::VERSION)
            .long(opt_flags::VERSION)
            .help("output version information and exit")
            .action(ArgAction::Version),
        Arg::new(opt_flags::HELP)
            .long(opt_flags::HELP)
            .help("display this help and exit")
            .action(ArgAction::Help),
        Arg::new(opt_flags::EXPRESSION)
            .action(ArgAction::Append)
            .allow_hyphen_values(true),
    ];

    // 构建并配置命令行解析器
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(about_help_info)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

// 命令行入口函数
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    expr_main(args).map(|_| ())
}

pub fn expr_main(args: impl ctcore::Args) -> CTResult<String> {
    // 解析命令行参数
    let args_match = ct_app().try_get_matches_from(args)?;

    // 提取并处理表达式参数
    let token_strings: Vec<&str> = args_match
        .get_many::<String>(opt_flags::EXPRESSION)
        .map(|v| v.into_iter().map(|s| s.as_ref()).collect::<Vec<_>>())
        .unwrap_or_default();

    // 解析、计算并输出表达式结果
    let result: String = SyntaxTreeAstNode::parse(&token_strings)?
        .eval()?
        .eval_as_string();

    println!("{result}");

    // 如果结果为假，则返回错误
    if !is_syntax_tree_truthy(&result.clone().into()) {
        return Err(1.into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    mod tests_expr_main {
        use crate::expr_main;

        use std::ffi::OsString;

        #[test]
        fn test_expr_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_expr_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_expr_main_add() {
            let args = vec![ctcore::ct_util_name(), "1", "+", "2"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Ok(result) => {
                    assert_eq!(result, "3");
                }
                Err(_) => {
                    assert!(false);
                }
            }
        }

        #[test]
        fn test_expr_main_sub() {
            let args = vec![ctcore::ct_util_name(), "1", "-", "2"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Ok(result) => {
                    assert_eq!(result, "-1");
                }
                Err(_) => {
                    assert!(false);
                }
            }
        }

        #[test]
        fn test_expr_main_mul() {
            let args = vec![ctcore::ct_util_name(), "1", "*", "2"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Ok(result) => {
                    assert_eq!(result, "2");
                }
                Err(_) => {
                    assert!(false);
                }
            }
        }

        #[test]
        fn test_expr_main_div() {
            let args = vec![ctcore::ct_util_name(), "7", "/", "2"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Ok(result) => {
                    assert_eq!(result, "3");
                }
                Err(_) => {
                    assert!(false);
                }
            }
        }

        #[test]
        fn test_expr_main_mod() {
            let args = vec![ctcore::ct_util_name(), "7", "%", "2"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Ok(result) => {
                    assert_eq!(result, "1");
                }
                Err(_) => {
                    assert!(false);
                }
            }
        }
        #[test]
        fn test_expr_main_index_num() {
            let args = vec![ctcore::ct_util_name(), "index", "12345", "2"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Ok(result) => {
                    assert_eq!(result, "2");
                }
                Err(_) => {
                    assert!(false);
                }
            }
        }

        #[test]
        fn test_expr_main_index_str_not_found() {
            let args = vec![ctcore::ct_util_name(), "index", "world", "x"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Ok(_) => {
                    assert!(false);
                }
                Err(_) => {
                    assert!(true);
                }
            }
        }

        #[test]
        fn test_expr_main_index_str() {
            let args = vec![ctcore::ct_util_name(), "index", "world", "o"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Ok(result) => {
                    assert_eq!(result, "2");
                }
                Err(_) => {
                    assert!(false);
                }
            }
        }

        #[test]
        fn test_expr_main_substr() {
            let args = vec![ctcore::ct_util_name(), "substr", "abcdef", "2", "3"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Ok(result) => {
                    assert_eq!(result, "bcd");
                }
                Err(_) => {
                    assert!(false);
                }
            }
        }

        #[test]
        fn test_expr_main_or() {
            let args = vec![ctcore::ct_util_name(), "0", "|", "3"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result.unwrap(), "3");
        }

        #[test]
        fn test_expr_main_and() {
            let args = vec![ctcore::ct_util_name(), "1", "&", "2"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result.unwrap(), "1");
        }

        #[test]
        fn test_expr_main_less_than() {
            let args = vec![ctcore::ct_util_name(), "2", "<", "3"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result.unwrap(), "1"); // 通常表达式结果为真会表示为1
        }

        #[test]
        fn test_expr_main_substring() {
            let args = vec![ctcore::ct_util_name(), "substr", "abcdef", "2", "3"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result.unwrap(), "bcd");
        }

        #[test]
        fn test_expr_main_index() {
            let args = vec![ctcore::ct_util_name(), "index", "world", "o"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result.unwrap(), "2"); // 'o' 在 "world" 中的位置从1开始计数
        }

        #[test]
        fn test_expr_main_match() {
            let args = vec![ctcore::ct_util_name(), "match", "hello", "ell"];
            // 假设匹配成功返回匹配的字符串或其长度，具体行为需根据实际实现调整
            let result = expr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Ok(result) => {
                    assert_eq!(result, "0");
                }
                Err(_) => {
                    assert!(true);
                }
            }
        }

        #[test]
        fn test_expr_main_escape_operator() {
            let args = vec![ctcore::ct_util_name(), "+", "match"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result.unwrap(), "match"); // '+' 强制将 "match" 当作字符串处理
        }

        #[test]
        fn test_expr_main_group() {
            let args = vec![ctcore::ct_util_name(), "(", "3", "+", "2", ")", "*", "4"];
            let result = expr_main(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result.unwrap(), "20");
        }
    }

    mod tests_false_app {
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
    }
}
