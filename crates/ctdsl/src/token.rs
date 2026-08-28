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

//! `Token` — ctdsl 词元枚举与 `SpannedToken`。
//!
//! 词法分析器（`lexer.rs`）将输入字符串切分为 `SpannedToken` 序列，
//! 解析器（`parser.rs`）消费该序列构建 AST。

use ctpipeline::CtSpan;

/// 词元类型
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // ── 字面量 ────────────────────────────────────────────
    /// 整数字面量：`42`, `-7`
    IntLit(i64),
    /// 浮点字面量：`3.14`
    FloatLit(f64),
    /// 字符串字面量（去除引号）：`"hello"` → `hello`
    StrLit(String),
    /// 布尔字面量：`true` / `false`
    BoolLit(bool),
    /// 字节大小字面量：`10mb` → 10_485_760
    SizeLit(u64),
    /// 纳秒时长字面量：`2min` → 120_000_000_000
    DurationLit(i64),
    /// Unix 纳秒时间戳字面量：`2025-01-01` → epoch nanos
    DateTimeLit(i128),

    // ── 标识符与关键字 ─────────────────────────────────────
    /// 普通标识符（命令名、字段名）
    Ident(String),

    // ── 运算符 ────────────────────────────────────────────
    /// `|`
    Pipe,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `<`
    Lt,
    /// `<=`
    Le,

    // ── 标志参数 ──────────────────────────────────────────
    /// `--flag`（长标志，不含 `--`）
    LongFlag(String),
    /// `--flag=value`（长标志和值均不含 `--`）
    LongFlagValue(String, String),
    /// `-f`（短标志，单字符）
    ShortFlag(char),

    // ── 特殊 ──────────────────────────────────────────────
    /// 文件末尾
    Eof,
}

impl Token {
    /// 返回词元的可读名称（用于错误消息）
    pub fn name(&self) -> &'static str {
        match self {
            Token::IntLit(_) => "integer",
            Token::FloatLit(_) => "float",
            Token::StrLit(_) => "string",
            Token::BoolLit(_) => "boolean",
            Token::SizeLit(_) => "size",
            Token::DurationLit(_) => "duration",
            Token::DateTimeLit(_) => "datetime",
            Token::Ident(_) => "identifier",
            Token::Pipe => "`|`",
            Token::Eq => "`==`",
            Token::Ne => "`!=`",
            Token::Gt => "`>`",
            Token::Ge => "`>=`",
            Token::Lt => "`<`",
            Token::Le => "`<=`",
            Token::LongFlag(_) => "flag (`--name`)",
            Token::LongFlagValue(_, _) => "flag value (`--name=value`)",
            Token::ShortFlag(_) => "short flag (`-x`)",
            Token::Eof => "end of input",
        }
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::IntLit(n) => write!(f, "{n}"),
            Token::FloatLit(v) => write!(f, "{v}"),
            Token::StrLit(s) => write!(f, "\"{s}\""),
            Token::BoolLit(b) => write!(f, "{b}"),
            Token::SizeLit(b) => write!(f, "{b}B"),
            Token::DurationLit(ns) => write!(f, "{ns}ns"),
            Token::DateTimeLit(ns) => write!(f, "<datetime:{ns}>"),
            Token::Ident(s) => write!(f, "{s}"),
            Token::Pipe => write!(f, "|"),
            Token::Eq => write!(f, "=="),
            Token::Ne => write!(f, "!="),
            Token::Gt => write!(f, ">"),
            Token::Ge => write!(f, ">="),
            Token::Lt => write!(f, "<"),
            Token::Le => write!(f, "<="),
            Token::LongFlag(n) => write!(f, "--{n}"),
            Token::LongFlagValue(n, v) => write!(f, "--{n}={v}"),
            Token::ShortFlag(c) => write!(f, "-{c}"),
            Token::Eof => write!(f, "<eof>"),
        }
    }
}

/// 带 span 标注的词元
#[derive(Debug, Clone)]
pub struct SpannedToken {
    pub token: Token,
    pub span: CtSpan,
}

impl SpannedToken {
    pub fn new(token: Token, span: CtSpan) -> Self {
        Self { token, span }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_name() {
        assert_eq!(Token::Pipe.name(), "`|`");
        assert_eq!(Token::Ident("ls".into()).name(), "identifier");
        assert_eq!(Token::LongFlag("verbose".into()).name(), "flag (`--name`)");
        assert_eq!(Token::Eof.name(), "end of input");
    }

    #[test]
    fn test_token_display() {
        assert_eq!(Token::IntLit(42).to_string(), "42");
        assert_eq!(Token::StrLit("hello".into()).to_string(), "\"hello\"");
        assert_eq!(Token::LongFlag("output".into()).to_string(), "--output");
        assert_eq!(Token::ShortFlag('v').to_string(), "-v");
        assert_eq!(Token::Eq.to_string(), "==");
    }

    #[test]
    fn test_token_equality() {
        assert_eq!(Token::Pipe, Token::Pipe);
        assert_eq!(Token::IntLit(1), Token::IntLit(1));
        assert_ne!(Token::IntLit(1), Token::IntLit(2));
    }
}
