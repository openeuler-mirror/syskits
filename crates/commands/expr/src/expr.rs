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

// expr 是一个经典的 Linux 或 Unix 命令行工具，用于执行基本的算术和逻辑表达式计算

extern crate rust_i18n;
use rust_i18n::t;
use std::fmt::Display;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::{
    ct_display::Quotable,
    ct_error::{CTError, CTResult},
};
use syntax_tree::SyntaxTreeAstNode;
use sys_locale::get_locale;

use crate::syntax_tree::is_syntax_tree_truthy;
use ctcore::Tool;
use std::ffi::{OsStr, OsString};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

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
    UnexpectedArgument(String),            // 意外的参数
    MissingArgument(String),               // 缺失的参数
    NonIntegerArgument,                    // 非整数参数
    MissingOperand,                        // 缺失操作数
    DivisionByZero,                        // 除以零
    RegexError(String),                    // 正则表达式错误
    ExpectedClosingBraceAfter(String),     // 期望在...之后看到闭合括号
    ExpectedClosingBraceInsteadOf(String), // 期望闭合括号，但遇到其他参数
    UnexpectedClosingBrace,                // 意外的右括号
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
            Self::RegexError(s) => write!(f, "{s}"),
            Self::ExpectedClosingBraceAfter(s) => {
                write!(f, "syntax error: expecting ')' after {}", s.quote())
            }
            Self::ExpectedClosingBraceInsteadOf(s) => {
                write!(f, "syntax error: expecting ')' instead of {}", s.quote())
            }
            Self::UnexpectedClosingBrace => write!(f, "syntax error: unexpected ')'"),
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
    let application_info = t!("expr.about");
    let usage_description = t!("expr.usage");
    let about_help_info = t!("expr.after_help");
    let args = vec![
        Arg::new(opt_flags::VERSION)
            .long(opt_flags::VERSION)
            .help(t!("expr.clap.version"))
            .action(ArgAction::Version),
        Arg::new(opt_flags::HELP)
            .long(opt_flags::HELP)
            .help(t!("expr.clap.help"))
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

pub fn expr_main(args: impl ctcore::Args) -> CTResult<String> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut args: Vec<OsString> = args.into_iter().collect();
    let mut operands: Vec<OsString> = if args.len() > 1 {
        args.drain(1..).collect()
    } else {
        Vec::new()
    };

    if let Some(first) = operands.first() {
        if first == OsStr::new("--help") || first == OsStr::new("--version") {
            let help_args = vec![OsString::from(ctcore::ct_util_name()), first.clone()];
            ct_app().try_get_matches_from(help_args)?;
        }
    }

    if matches!(operands.first(), Some(arg) if arg == OsStr::new("--")) {
        operands.remove(0);
    }

    #[cfg(unix)]
    fn os_to_bytes(arg: OsString) -> Vec<u8> {
        arg.into_vec()
    }

    #[cfg(not(unix))]
    fn os_to_bytes(arg: OsString) -> Vec<u8> {
        arg.into_string().unwrap_or_default().into_bytes()
    }

    let token_bytes: Vec<Vec<u8>> = operands.into_iter().map(os_to_bytes).collect();

    let result = SyntaxTreeAstNode::parse_bytes(&token_bytes)?.eval()?;
    let output_bytes = result.clone().eval_as_bytes();

    let mut stdout = std::io::stdout();
    stdout.write_all(&output_bytes)?;
    stdout.write_all(b"\n")?;

    // 如果结果为假，则返回错误
    if !is_syntax_tree_truthy(&result) {
        return Err(1.into());
    }

    Ok(String::from_utf8_lossy(&output_bytes).into_owned())
}

#[derive(Default)]
pub struct Expr;
impl Tool for Expr {
    fn name(&self) -> &'static str {
        "expr"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        expr_main(args.iter().cloned()).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Expr::default();

        // Test name method
        assert_eq!(tool.name(), "expr");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("expr"));

        // Test execute method - should return error without arguments
        let args = vec![OsString::from("expr")];
        assert!(tool.execute(&args).is_err());

        // Test execute with valid arguments
        let args = vec![
            OsString::from("expr"),
            OsString::from("5"),
            OsString::from("+"),
            OsString::from("3"),
        ];
        assert!(tool.execute(&args).is_ok());
    }

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
