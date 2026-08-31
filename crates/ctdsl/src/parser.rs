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

//! `Parser` — ctdsl 递归下降语法分析器。
//!
//! 语法（M1b 子集，EBNF 表示）：
//! ```text
//! pipeline  ::= call ( '|' call )*
//! call      ::= IDENT arg*
//! arg       ::= long_flag | short_flag | positional | comparison
//! long_flag ::= '--' IDENT
//! short_flag::= '-' CHAR
//! positional::= lit
//! comparison::= IDENT comp_op lit          (* where 命令专用 *)
//! where-expr ::= comparison ( ('and'|'or') comparison )*
//! comp_op   ::= '==' | '!=' | '>' | '>=' | '<' | '<='
//! lit       ::= INT | FLOAT | STRING | BOOL | IDENT
//! ```
//!
//! 解析终止条件：`Token::Eof` 或 `Token::Pipe`（下一阶段分隔符）。

use ctpipeline::CtSpan;

use crate::ast::{Arg, Call, CompOp, Expr, Lit};
use crate::error::ParseError;
use crate::token::{SpannedToken, Token};

/// 递归下降语法分析器
pub struct Parser {
    tokens: Vec<SpannedToken>,
    /// 当前游标（指向下一个待消费词元）
    pos: usize,
}

impl Parser {
    /// 从已词法分析的 token 序列构造 parser
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Self { tokens, pos: 0 }
    }

    /// 解析整个输入为管线表达式
    pub fn parse_pipeline(&mut self) -> Result<Expr, ParseError> {
        let mut calls = Vec::new();
        // 至少要有一个 call
        calls.push(self.parse_call()?);
        while self.peek_is(Token::Pipe) {
            self.advance(); // 消费 `|`
            calls.push(self.parse_call()?);
        }
        // 确认已到 Eof
        if !self.peek_is(Token::Eof) {
            let st = &self.tokens[self.pos];
            return Err(ParseError::syntax(
                format!("unexpected token `{}`", st.token),
                st.span.clone(),
            ));
        }
        Ok(Expr::Pipeline(calls))
    }

    // ── 主要规则 ─────────────────────────────────────────

    fn parse_call(&mut self) -> Result<Call, ParseError> {
        // 命令名：通常是 Ident；`true`/`false` 在命令名位置按
        // coreutils 命令名处理，在参数位置仍保持 BoolLit。
        let name_tok = self.expect_command_name()?;
        let name = name_tok.0;
        let call_span = name_tok.1.clone();

        let mut args = Vec::new();

        // where 命令优先解析逻辑条件表达式
        if name == "where" && !matches!(self.peek_token(), Token::Eof | Token::Pipe) {
            args.push(self.parse_where_expr()?);
            return Ok(Call {
                name,
                args,
                span: call_span,
            });
        }

        loop {
            match self.peek_token() {
                Token::Eof | Token::Pipe => break,
                Token::LongFlag(_) => args.push(self.parse_long_flag()?),
                Token::LongFlagValue(_, _) => args.push(self.parse_long_flag_value()?),
                Token::ShortFlag(_) => args.push(self.parse_short_flag()?),
                Token::Ident(_) => {
                    // 判断是否是 comparison：IDENT comp_op lit
                    if self.is_comparison_ahead() {
                        args.push(self.parse_comparison()?);
                    } else {
                        args.push(self.parse_positional()?);
                    }
                }
                _ => args.push(self.parse_positional()?),
            }
        }

        Ok(Call {
            name,
            args,
            span: call_span,
        })
    }

    fn parse_where_expr(&mut self) -> Result<Arg, ParseError> {
        let (field, op, rhs, first_field_span, first_rhs_span) = self.parse_comparison_atom()?;
        let mut conditions = vec![(field.clone(), op.clone(), rhs.clone())];
        let mut logic_ops = Vec::new();
        let mut last_end = first_rhs_span.end;

        while !matches!(self.peek_token(), Token::Eof | Token::Pipe) {
            let logic_st = self.advance();
            let logic = match &logic_st.token {
                Token::Ident(s) if s.eq_ignore_ascii_case("and") => "and".to_string(),
                Token::Ident(s) if s.eq_ignore_ascii_case("or") => "or".to_string(),
                other => {
                    return Err(ParseError::syntax(
                        format!("expected logical operator `and`/`or`, got `{other}`"),
                        logic_st.span.clone(),
                    ));
                }
            };
            let (field, op, rhs, _field_span, rhs_span) = self.parse_comparison_atom()?;
            conditions.push((field, op, rhs));
            logic_ops.push(logic);
            last_end = rhs_span.end;
        }

        if logic_ops.is_empty() {
            return Ok(Arg::Comparison {
                field,
                op,
                rhs,
                span: CtSpan {
                    end: first_rhs_span.end,
                    ..first_field_span
                },
            });
        }

        Ok(Arg::WhereExpr {
            conditions,
            logic_ops,
            span: CtSpan {
                end: last_end,
                ..first_field_span
            },
        })
    }

    fn parse_comparison_atom(
        &mut self,
    ) -> Result<(String, CompOp, Lit, CtSpan, CtSpan), ParseError> {
        let field_st = self.advance();
        let field = match &field_st.token {
            Token::Ident(s) => s.clone(),
            other => {
                return Err(ParseError::syntax(
                    format!("expected field name, got `{other}`"),
                    field_st.span.clone(),
                ));
            }
        };
        let field_span = field_st.span.clone();
        let op = self.parse_comp_op()?;
        let (rhs, rhs_span) = self.parse_lit()?;
        Ok((field, op, rhs, field_span, rhs_span))
    }

    fn parse_long_flag(&mut self) -> Result<Arg, ParseError> {
        let st = self.advance();
        let flag_name = match &st.token {
            Token::LongFlag(n) => n.clone(),
            _ => unreachable!(),
        };
        Ok(Arg::LongFlag {
            name: flag_name,
            span: st.span.clone(),
        })
    }

    fn parse_long_flag_value(&mut self) -> Result<Arg, ParseError> {
        let st = self.advance();
        let (flag_name, value) = match &st.token {
            Token::LongFlagValue(name, value) => (name.clone(), value.clone()),
            _ => unreachable!(),
        };
        Ok(Arg::LongFlagValue {
            name: flag_name,
            value: Lit::Ident(value),
            value_span: st.span.clone(),
            span: st.span.clone(),
        })
    }

    fn parse_short_flag(&mut self) -> Result<Arg, ParseError> {
        let st = self.advance();
        let ch = match &st.token {
            Token::ShortFlag(c) => *c,
            _ => unreachable!(),
        };
        Ok(Arg::ShortFlag {
            name: ch,
            span: st.span.clone(),
        })
    }

    fn parse_positional(&mut self) -> Result<Arg, ParseError> {
        let (value, span) = self.parse_lit()?;
        Ok(Arg::Positional { value, span })
    }

    fn parse_comparison(&mut self) -> Result<Arg, ParseError> {
        let (field, op, rhs, field_span, rhs_span) = self.parse_comparison_atom()?;
        let span = CtSpan {
            end: rhs_span.end,
            ..field_span
        };
        Ok(Arg::Comparison {
            field,
            op,
            rhs,
            span,
        })
    }

    fn parse_comp_op(&mut self) -> Result<CompOp, ParseError> {
        let st = self.advance();
        match &st.token {
            Token::Eq => Ok(CompOp::Eq),
            Token::Ne => Ok(CompOp::Ne),
            Token::Gt => Ok(CompOp::Gt),
            Token::Ge => Ok(CompOp::Ge),
            Token::Lt => Ok(CompOp::Lt),
            Token::Le => Ok(CompOp::Le),
            other => Err(ParseError::syntax(
                format!("expected comparison operator, got `{other}`"),
                st.span.clone(),
            )),
        }
    }

    fn parse_lit(&mut self) -> Result<(Lit, CtSpan), ParseError> {
        let st = self.advance();
        let span = st.span.clone();
        let lit = match &st.token {
            Token::IntLit(n) => Lit::Int(*n),
            Token::FloatLit(f) => Lit::Float(*f),
            Token::StrLit(s) => Lit::String(s.clone()),
            Token::BoolLit(b) => Lit::Bool(*b),
            Token::SizeLit(b) => Lit::Size(*b),
            Token::DurationLit(ns) => Lit::Duration(*ns),
            Token::DateTimeLit(ns) => Lit::DateTime(*ns),
            Token::Ident(s) => Lit::Ident(s.clone()),
            other => {
                return Err(ParseError::syntax(
                    format!("expected literal, got `{other}`"),
                    st.span.clone(),
                ));
            }
        };
        Ok((lit, span))
    }

    // ── 辅助方法 ─────────────────────────────────────────

    fn peek_token(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn peek_is(&self, expected: Token) -> bool {
        std::mem::discriminant(self.peek_token()) == std::mem::discriminant(&expected)
    }

    /// 检测是否是 comparison 模式：IDENT comp_op ...
    fn is_comparison_ahead(&self) -> bool {
        if self.pos + 1 >= self.tokens.len() {
            return false;
        }
        matches!(
            self.tokens[self.pos + 1].token,
            Token::Eq | Token::Ne | Token::Gt | Token::Ge | Token::Lt | Token::Le
        )
    }

    fn advance(&mut self) -> &SpannedToken {
        let tok = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect_command_name(&mut self) -> Result<(String, CtSpan), ParseError> {
        let st = self.advance();
        match &st.token {
            Token::Ident(s) => Ok((s.clone(), st.span.clone())),
            Token::BoolLit(value) => Ok((value.to_string(), st.span.clone())),
            other => Err(ParseError::syntax(
                format!("expected command name, got `{other}`"),
                st.span.clone(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(s: &str) -> Expr {
        let tokens = Lexer::new(s).tokenize().expect("lex failed");
        Parser::new(tokens).parse_pipeline().expect("parse failed")
    }

    fn parse_err(s: &str) -> ParseError {
        let tokens = Lexer::new(s).tokenize().expect("lex failed");
        Parser::new(tokens)
            .parse_pipeline()
            .expect_err("expected parse error")
    }

    #[test]
    fn test_parse_single_call() {
        let expr = parse("ls");
        let stages = expr.stages();
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].name, "ls");
        assert!(stages[0].args.is_empty());
    }

    #[test]
    fn test_parse_bool_keywords_as_command_names() {
        let expr = parse("false | true");
        let stages = expr.stages();
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].name, "false");
        assert_eq!(stages[1].name, "true");
    }

    #[test]
    fn test_parse_tilde_prefixed_command_name() {
        let expr = parse("~uname -a");
        let stages = expr.stages();
        assert_eq!(stages[0].name, "~uname");
        assert!(matches!(
            &stages[0].args[0],
            Arg::ShortFlag { name: 'a', .. }
        ));
    }

    #[test]
    fn test_parse_bool_keywords_as_positional_literals() {
        let expr = parse("echo false true");
        let args = &expr.stages()[0].args;
        assert!(matches!(
            &args[0],
            Arg::Positional {
                value: Lit::Bool(false),
                ..
            }
        ));
        assert!(matches!(
            &args[1],
            Arg::Positional {
                value: Lit::Bool(true),
                ..
            }
        ));
    }

    #[test]
    fn test_parse_pipeline_two_stages() {
        let expr = parse("from json | to json");
        let stages = expr.stages();
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].name, "from");
        assert_eq!(stages[1].name, "to");
    }

    #[test]
    fn test_parse_pipeline_three_stages() {
        let expr = parse("from json | select name | to json");
        assert_eq!(expr.stages().len(), 3);
    }

    #[test]
    fn test_parse_positional_arg() {
        let expr = parse("get name");
        let args = &expr.stages()[0].args;
        assert_eq!(args.len(), 1);
        assert!(matches!(&args[0], Arg::Positional { value: Lit::Ident(s), .. } if s == "name"));
    }

    #[test]
    fn test_parse_long_flag_switch() {
        let expr = parse("ls --all");
        let args = &expr.stages()[0].args;
        assert!(matches!(&args[0], Arg::LongFlag { name, .. } if name == "all"));
    }

    #[test]
    fn test_parse_long_flag_value_kept_as_positional() {
        let expr = parse("to --format json");
        let args = &expr.stages()[0].args;
        assert!(matches!(
            &args[0],
            Arg::LongFlag { name, .. } if name == "format"
        ));
        assert!(matches!(
            &args[1],
            Arg::Positional { value: Lit::Ident(v), .. } if v == "json"
        ));
    }

    #[test]
    fn test_parse_long_flag_equals_value() {
        let expr = parse("numfmt --to=si");
        let args = &expr.stages()[0].args;
        assert!(matches!(
            &args[0],
            Arg::LongFlagValue {
                name,
                value: Lit::Ident(value),
                ..
            } if name == "to" && value == "si"
        ));
    }

    #[test]
    fn test_parse_short_flag() {
        let expr = parse("ls -l");
        let args = &expr.stages()[0].args;
        assert!(matches!(&args[0], Arg::ShortFlag { name: 'l', .. }));
    }

    #[test]
    fn test_parse_comparison() {
        let expr = parse("where size > 100");
        let args = &expr.stages()[0].args;
        assert_eq!(args.len(), 1);
        assert!(matches!(
            &args[0],
            Arg::Comparison { field, op: CompOp::Gt, rhs: Lit::Int(100), .. }
            if field == "size"
        ));
    }

    #[test]
    fn test_parse_where_logic_expr() {
        let expr = parse("where size > 100 and name == \"foo\"");
        let args = &expr.stages()[0].args;
        assert_eq!(args.len(), 1);
        assert!(matches!(
            &args[0],
            Arg::WhereExpr { conditions, logic_ops, .. }
            if conditions.len() == 2 && logic_ops == &vec!["and".to_string()]
        ));
    }

    #[test]
    fn test_parse_where_it_field_expr() {
        let expr = parse("where $it.user.age >= 18");
        let args = &expr.stages()[0].args;
        assert_eq!(args.len(), 1);
        assert!(matches!(
            &args[0],
            Arg::Comparison { field, op: CompOp::Ge, rhs: Lit::Int(18), .. }
            if field == "$it.user.age"
        ));
    }

    #[test]
    fn test_parse_select_multiple_cols() {
        let expr = parse("select name size type");
        let args = &expr.stages()[0].args;
        assert_eq!(args.len(), 3);
    }

    #[test]
    fn test_parse_string_literal() {
        let expr = parse(r#"where name == "foo""#);
        let args = &expr.stages()[0].args;
        assert!(matches!(
            &args[0],
            Arg::Comparison { rhs: Lit::String(s), .. } if s == "foo"
        ));
    }

    #[test]
    fn test_parse_single_quoted_string_literal() {
        let expr = parse("from json '{\"a\":1}'");
        let args = &expr.stages()[0].args;
        assert!(matches!(
            &args[1],
            Arg::Positional { value: Lit::String(s), .. } if s == "{\"a\":1}"
        ));
    }

    #[test]
    fn test_parse_error_bad_start() {
        let err = parse_err("| missing");
        assert!(matches!(err, ParseError::SyntaxError { .. }));
    }
}
