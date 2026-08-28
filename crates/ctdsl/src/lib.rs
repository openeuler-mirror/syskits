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

//! `ctdsl` — syskits 数据管线 DSL（Lexer / Parser / AST）。
//!
//! 主要公开接口：
//! - [`parse`]：一步完成词法分析 + 语法分析，返回顶层 `Expr`
//! - [`ast`]：AST 节点类型
//! - [`error`]：`ParseError`
//! - [`token`]：Token 类型（高级用法）

pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod precheck;
pub mod token;

pub use ast::{Arg, Call, CompOp, Expr, Lit};
pub use error::ParseError;
pub use lexer::Lexer;
pub use parser::Parser;
pub use precheck::{PrecheckDiagnostic, PrecheckLevel, precheck_expr};

/// 解析管线表达式字符串
///
/// # 示例
/// ```rust,no_run
/// let expr = ctdsl::parse("from json | select name | to json").unwrap();
/// ```
pub fn parse(input: &str) -> Result<Expr, ParseError> {
    let tokens = Lexer::new(input).tokenize()?;
    Parser::new(tokens).parse_pipeline()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_pipeline() {
        let expr = parse("from json | to json").unwrap();
        assert_eq!(expr.stages().len(), 2);
    }

    #[test]
    fn test_parse_empty_input_error() {
        // 空输入：无命令名
        let result = parse("");
        assert!(result.is_err(), "empty input should return error");
    }

    #[test]
    fn test_parse_error_returns_span() {
        // 单独的 `|` 开头应该返回有 span 的错误
        let err = parse("| bad").unwrap_err();
        assert!(matches!(err, ParseError::SyntaxError { .. }));
    }
}
