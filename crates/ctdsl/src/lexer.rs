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

//! `Lexer` — ctdsl 手写词法分析器。
//!
//! 输入：一个管线表达式字符串（UTF-8）。
//! 输出：`Vec<SpannedToken>`，末尾自动追加 `Token::Eof`。
//!
//! 支持的词元（M1b 子集）：
//! - 整数 / 浮点 / 字符串（双引号或单引号）/ 布尔（true/false）
//! - 标识符
//! - `|` `==` `!=` `>` `>=` `<` `<=`
//! - `--flag`（长） / `-f`（短）
//! - 跳过空白（空格、制表符）

use ctpipeline::{CtSourceRef, CtSpan};

use crate::error::ParseError;
use crate::token::{SpannedToken, Token};

/// 词法分析器
pub struct Lexer<'s> {
    src: &'s str,
    /// 当前字节偏移
    pos: usize,
    /// 当前行号（1-base）
    line: u32,
    /// 当前列号（1-base，byte 语义）
    col: u32,
    /// 用于构建 CtSpan 的来源
    source_ref: CtSourceRef,
}

impl<'s> Lexer<'s> {
    /// 创建词法分析器（来源标记为 `InlineExpr`）
    pub fn new(src: &'s str) -> Self {
        Self {
            src,
            pos: 0,
            line: 1,
            col: 1,
            source_ref: CtSourceRef::InlineExpr,
        }
    }

    /// 对整个输入进行词法分析，返回所有词元（含 Eof）
    pub fn tokenize(mut self) -> Result<Vec<SpannedToken>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.src.len() {
                tokens.push(SpannedToken::new(
                    Token::Eof,
                    self.span_at(self.pos, self.pos),
                ));
                break;
            }
            let start = self.pos;
            let start_line = self.line;
            let start_col = self.col;
            let tok = self.next_token()?;
            let end = self.pos;
            let span = CtSpan {
                source: self.source_ref.clone(),
                start,
                end,
                start_line,
                start_col,
            };
            tokens.push(SpannedToken::new(tok, span));
        }
        Ok(tokens)
    }

    // ── 内部辅助 ─────────────────────────────────────────

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek2(&self) -> Option<char> {
        let mut chars = self.src[self.pos..].chars();
        chars.next();
        chars.next()
    }

    fn peek3(&self) -> Option<char> {
        let mut chars = self.src[self.pos..].chars();
        chars.next();
        chars.next();
        chars.next()
    }

    fn peek4(&self) -> Option<char> {
        let mut chars = self.src[self.pos..].chars();
        chars.next();
        chars.next();
        chars.next();
        chars.next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += ch.len_utf8() as u32;
        }
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' || ch == '\r' || ch == '\n' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn span_at(&self, start: usize, end: usize) -> CtSpan {
        CtSpan {
            source: self.source_ref.clone(),
            start,
            end,
            start_line: self.line,
            start_col: self.col,
        }
    }

    fn current_err(&self, msg: impl Into<String>) -> ParseError {
        ParseError::LexError {
            message: msg.into(),
            span: self.span_at(self.pos, self.pos),
        }
    }

    fn col_at_pos(&self, pos: usize) -> u32 {
        let line_start = self.src[..pos].rfind('\n').map_or(0, |p| p + 1);
        1 + (pos - line_start) as u32
    }

    /// 前瞻检测：从当前位置开始是否像 `YYYY-MM-DD` 模式
    fn looks_like_datetime(&self) -> bool {
        let remaining = &self.src[self.pos..];
        let bytes = remaining.as_bytes();
        // 至少 10 字符: YYYY-MM-DD
        if bytes.len() < 10 {
            return false;
        }
        bytes[0..4].iter().all(|b| b.is_ascii_digit())
            && bytes[4] == b'-'
            && bytes[5..7].iter().all(|b| b.is_ascii_digit())
            && bytes[7] == b'-'
            && bytes[8..10].iter().all(|b| b.is_ascii_digit())
            // 后面必须是空白、管道符、比较符、EOF 或 T（接时间部分）
            && (bytes.len() == 10
                || bytes[10] == b' '
                || bytes[10] == b'\t'
                || bytes[10] == b'|'
                || bytes[10] == b'>'
                || bytes[10] == b'<'
                || bytes[10] == b'='
                || bytes[10] == b'!'
                || bytes[10] == b'T'
                || bytes[10] == b't')
    }

    fn looks_like_numeric_range_arg(&self) -> bool {
        let bytes = self.src.as_bytes();
        let mut i = self.pos;

        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }

        if i >= bytes.len() || bytes[i] != b'-' {
            return false;
        }

        let next = i + 1;
        next < bytes.len() && bytes[next].is_ascii_digit()
    }

    // ── 词元识别 ─────────────────────────────────────────

    fn next_token(&mut self) -> Result<Token, ParseError> {
        let ch = match self.peek() {
            Some(c) => c,
            None => return Ok(Token::Eof),
        };

        match ch {
            '|' => {
                self.advance();
                Ok(Token::Pipe)
            }

            '=' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token::Eq)
                } else {
                    Ok(Token::Ident("=".into()))
                }
            }

            '!' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token::Ne)
                } else {
                    Ok(Token::Ident("!".into()))
                }
            }

            '>' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token::Ge)
                } else {
                    Ok(Token::Gt)
                }
            }

            '<' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token::Le)
                } else {
                    Ok(Token::Lt)
                }
            }

            '"' => self.lex_string('"'),
            '\'' => self.lex_string('\''),

            '+' | '*' | '%' | ':' | '(' | ')' | '&' => {
                self.advance();
                Ok(Token::Ident(ch.to_string()))
            }

            '-' => {
                if self.peek2() == Some('-') {
                    self.lex_long_flag()
                } else if self
                    .peek2()
                    .map(|c| c.is_whitespace() || c == '|')
                    .unwrap_or(true)
                {
                    self.advance();
                    Ok(Token::Ident("-".into()))
                } else if self
                    .peek2()
                    .map(|c| c.is_ascii_alphabetic() || c == '0')
                    .unwrap_or(false)
                {
                    let after_dash = self.peek2();
                    let after_short = self.peek3();
                    let zero_prefixed_number = after_dash == Some('0')
                        && after_short.is_some_and(|c| {
                            c.is_ascii_digit()
                                || (c == '.' && self.peek4().is_some_and(|n| n.is_ascii_digit()))
                        });
                    if zero_prefixed_number {
                        self.lex_number()
                    } else if after_short
                        .map(|c| c.is_whitespace() || c == '|')
                        .unwrap_or(true)
                    {
                        self.lex_short_flag()
                    } else {
                        self.lex_dash_prefixed_ident()
                    }
                } else {
                    // 负数：- 接数字
                    self.lex_number()
                }
            }

            c if c.is_ascii_digit() => {
                // 前瞻检测 datetime: YYYY-MM-DD 模式
                if self.looks_like_datetime() || self.looks_like_numeric_range_arg() {
                    self.lex_ident_or_keyword()
                } else {
                    self.lex_number()
                }
            }

            c if c.is_ascii_alphabetic()
                || c == '_'
                || c == '$'
                || c == '/'
                || c == '.'
                || c == '~' =>
            {
                self.lex_ident_or_keyword()
            }

            other => Err(self.current_err(format!("unexpected character `{other}`"))),
        }
    }

    fn lex_string(&mut self, quote: char) -> Result<Token, ParseError> {
        self.advance(); // skip opening quote
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(self.current_err("unterminated string literal")),
                Some(ch) if ch == quote => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.peek() {
                        Some('n') => {
                            self.advance();
                            s.push('\n');
                        }
                        Some('t') => {
                            self.advance();
                            s.push('\t');
                        }
                        Some('\\') => {
                            self.advance();
                            s.push('\\');
                        }
                        Some('"') => {
                            self.advance();
                            s.push('"');
                        }
                        Some('\'') => {
                            self.advance();
                            s.push('\'');
                        }
                        other => {
                            return Err(self.current_err(format!(
                                "unknown escape `\\{}`",
                                other.unwrap_or('?')
                            )));
                        }
                    }
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
            }
        }
        Ok(Token::StrLit(s))
    }

    fn lex_number(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        let is_negative = self.peek() == Some('-');
        if is_negative {
            self.advance();
        }
        while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            self.advance();
        }
        // 浮点
        let is_float =
            self.peek() == Some('.') && self.peek2().map(|c| c.is_ascii_digit()).unwrap_or(false);
        if is_float {
            self.advance(); // `.`
            while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                self.advance();
            }
        }
        let num_str = &self.src[start..self.pos];

        // 检查后缀：size 或 duration 单位
        let suffix_start = self.pos;
        while self
            .peek()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
        {
            self.advance();
        }
        let suffix = &self.src[suffix_start..self.pos];

        if !suffix.is_empty() {
            // 尝试解析为数值（可能是 float 或 int）
            let num: f64 = num_str
                .parse()
                .map_err(|_| self.current_err(format!("invalid number `{num_str}`")))?;

            // 尝试 size 后缀
            let suffix_lower = suffix.to_ascii_lowercase();
            if let Some(multiplier) = size_suffix_multiplier(&suffix_lower) {
                if is_negative {
                    return Err(self.current_err("size literal cannot be negative"));
                }
                let bytes = (num * multiplier as f64) as u64;
                return Ok(Token::SizeLit(bytes));
            }

            // 尝试 duration 后缀
            if let Some(nanos_per_unit) = duration_suffix_nanos(&suffix_lower) {
                let nanos = (num * nanos_per_unit as f64) as i64;
                return Ok(Token::DurationLit(nanos));
            }

            // 后缀不识别，回退到后缀开始位置，按纯数字处理
            self.pos = suffix_start;
            // 直接从位置重算列号，避免先减后算的中间状态风险。
            self.col = self.col_at_pos(self.pos);
        }

        if is_float {
            num_str
                .parse::<f64>()
                .map(Token::FloatLit)
                .map_err(|_| self.current_err(format!("invalid float `{num_str}`")))
        } else {
            num_str
                .parse::<i64>()
                .map(Token::IntLit)
                .map_err(|_| self.current_err(format!("invalid integer `{num_str}`")))
        }
    }

    fn lex_ident_or_keyword(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        while self
            .peek()
            .map(|c| {
                c.is_ascii_alphanumeric()
                    || c == '_'
                    || c == '-'
                    || c == '.'
                    || c == '$'
                    || c == ':'
                    || c == '/'
                    || c == '~'
            })
            .unwrap_or(false)
        {
            self.advance();
        }
        let word = &self.src[start..self.pos];
        Ok(match word {
            "true" => Token::BoolLit(true),
            "false" => Token::BoolLit(false),
            s => {
                // 尝试解析为 DateTime（YYYY-MM-DD 或 YYYY-MM-DDThh:mm:ss）
                if let Some(nanos) = try_parse_datetime(s) {
                    Token::DateTimeLit(nanos)
                } else {
                    Token::Ident(s.to_string())
                }
            }
        })
    }

    fn lex_long_flag(&mut self) -> Result<Token, ParseError> {
        self.advance(); // `-`
        self.advance(); // `-`
        let start = self.pos;
        while self
            .peek()
            .map(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            .unwrap_or(false)
        {
            self.advance();
        }
        if self.pos == start {
            if self
                .peek()
                .map(|c| c.is_whitespace() || c == '|')
                .unwrap_or(true)
            {
                return Ok(Token::Ident("--".into()));
            }
            return Err(self.current_err("empty long flag after `--`"));
        }
        let name = self.src[start..self.pos].to_string();
        if self.peek() == Some('=') {
            self.advance();
            let value_start = self.pos;
            while self
                .peek()
                .map(|c| !c.is_whitespace() && c != '|')
                .unwrap_or(false)
            {
                self.advance();
            }
            return Ok(Token::LongFlagValue(
                name,
                self.src[value_start..self.pos].to_string(),
            ));
        }
        Ok(Token::LongFlag(name))
    }

    fn lex_short_flag(&mut self) -> Result<Token, ParseError> {
        self.advance(); // `-`
        let ch = self
            .peek()
            .ok_or_else(|| self.current_err("empty short flag after `-`"))?;
        self.advance();
        Ok(Token::ShortFlag(ch))
    }

    fn lex_dash_prefixed_ident(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        self.advance(); // `-`
        while self
            .peek()
            .map(|c| !c.is_whitespace() && c != '|')
            .unwrap_or(false)
        {
            self.advance();
        }
        Ok(Token::Ident(self.src[start..self.pos].to_string()))
    }
}

// ── 字面量后缀解析辅助 ───────────────────────────────────

/// 将 size 后缀映射为字节乘数
fn size_suffix_multiplier(suffix: &str) -> Option<u64> {
    match suffix {
        "b" => Some(1),
        "kb" => Some(1024),
        "mb" => Some(1024 * 1024),
        "gb" => Some(1024 * 1024 * 1024),
        "tb" => Some(1024u64 * 1024 * 1024 * 1024),
        _ => None,
    }
}

/// 将 duration 后缀映射为纳秒乘数
fn duration_suffix_nanos(suffix: &str) -> Option<u64> {
    match suffix {
        "ns" => Some(1),
        "us" => Some(1_000),
        "ms" => Some(1_000_000),
        "s" | "sec" => Some(1_000_000_000),
        "min" => Some(60_000_000_000),
        "h" | "hr" => Some(3_600_000_000_000),
        "d" | "day" => Some(86_400_000_000_000),
        _ => None,
    }
}

/// 尝试解析 datetime 为 Unix 纳秒时间戳。
/// 支持：
/// - YYYY-MM-DD
/// - YYYY-MM-DDThh:mm:ss
/// - YYYY-MM-DDThh:mm:ss.fffffffff（1~9 位小数秒）
/// - 可选尾部 `Z`/`z`
fn try_parse_datetime(s: &str) -> Option<i128> {
    // 最短格式: YYYY-MM-DD（10 字符）
    if s.len() != 10 && s.len() < 19 {
        return None;
    }
    let bytes = s.as_bytes();
    // 校验并解析日期部分：YYYY-MM-DD
    if !(bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes.get(0..4)?.iter().all(|b| b.is_ascii_digit())
        && bytes.get(5..7)?.iter().all(|b| b.is_ascii_digit())
        && bytes.get(8..10)?.iter().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let year: i64 = s[0..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    let max_day = days_in_month(year, month);
    if day == 0 || day > max_day {
        return None;
    }

    let (hour, minute, second, subsec_nanos) = if s.len() == 10 {
        (0, 0, 0, 0)
    } else {
        // YYYY-MM-DDThh:mm:ss[.fffffffff][Z]
        if !(bytes.get(10) == Some(&b'T') || bytes.get(10) == Some(&b't')) {
            return None;
        }
        if !(bytes.get(13) == Some(&b':') && bytes.get(16) == Some(&b':')) {
            return None;
        }
        if !(bytes.get(11..13)?.iter().all(|b| b.is_ascii_digit())
            && bytes.get(14..16)?.iter().all(|b| b.is_ascii_digit())
            && bytes.get(17..19)?.iter().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        let h: u32 = s[11..13].parse().ok()?;
        let m: u32 = s[14..16].parse().ok()?;
        let sec: u32 = s[17..19].parse().ok()?;
        if h >= 24 || m >= 60 || sec >= 60 {
            return None;
        }

        let mut idx = 19usize;
        let mut nanos = 0u32;

        // 可选 .fffffffff（1~9 位）
        if bytes.get(idx) == Some(&b'.') {
            idx += 1;
            let frac_start = idx;
            while idx < bytes.len() && bytes[idx].is_ascii_digit() {
                idx += 1;
            }
            let frac_len = idx - frac_start;
            if frac_len == 0 || frac_len > 9 {
                return None;
            }
            let frac_digits = &s[frac_start..idx];
            nanos = frac_digits.parse::<u32>().ok()?;
            for _ in 0..(9 - frac_len) {
                nanos *= 10;
            }
        }

        // 可选 Z/z，且不允许任何其他尾随字符
        if idx < bytes.len() {
            if (bytes[idx] == b'Z' || bytes[idx] == b'z') && idx + 1 == bytes.len() {
                idx += 1;
            } else {
                return None;
            }
        }
        if idx != bytes.len() {
            return None;
        }

        (h, m, sec, nanos)
    };

    let days = ymd_to_days(year, month, day);
    let secs = days as i128 * 86400 + hour as i128 * 3600 + minute as i128 * 60 + second as i128;
    Some(secs * 1_000_000_000 + subsec_nanos as i128)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// 将 (year, month, day) 转换为从 1970-01-01 起的天数偏移 (Howard Hinnant civil_from_days 的逆)
fn ymd_to_days(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(s: &str) -> Vec<Token> {
        Lexer::new(s)
            .tokenize()
            .expect("lex failed")
            .into_iter()
            .map(|st| st.token)
            .collect()
    }

    #[test]
    fn test_lex_pipe() {
        let toks = lex("ls | select");
        assert_eq!(toks[0], Token::Ident("ls".into()));
        assert_eq!(toks[1], Token::Pipe);
        assert_eq!(toks[2], Token::Ident("select".into()));
        assert_eq!(toks[3], Token::Eof);
    }

    #[test]
    fn test_lex_integer() {
        let toks = lex("42");
        assert_eq!(toks[0], Token::IntLit(42));
    }

    #[test]
    fn test_lex_negative_integer() {
        let toks = lex("-7");
        assert_eq!(toks[0], Token::IntLit(-7));
    }

    #[test]
    fn test_lex_negative_zero_prefixed_number_literals() {
        let toks = lex("-05 -0.5");
        assert_eq!(toks[0], Token::IntLit(-5));
        assert_eq!(toks[1], Token::FloatLit(-0.5));
    }

    #[test]
    fn test_lex_float() {
        let toks = lex("2.5");
        assert_eq!(toks[0], Token::FloatLit(2.5));
    }

    #[test]
    fn test_lex_numeric_range_arg_as_ident() {
        let toks = lex("1-3");
        assert_eq!(toks[0], Token::Ident("1-3".into()));
    }

    #[test]
    fn test_lex_string() {
        let toks = lex("\"hello world\"");
        assert_eq!(toks[0], Token::StrLit("hello world".into()));
    }

    #[test]
    fn test_lex_single_quoted_string() {
        let toks = lex("'hello world'");
        assert_eq!(toks[0], Token::StrLit("hello world".into()));
    }

    #[test]
    fn test_lex_bool() {
        let toks = lex("true false");
        assert_eq!(toks[0], Token::BoolLit(true));
        assert_eq!(toks[1], Token::BoolLit(false));
    }

    #[test]
    fn test_lex_long_flag() {
        let toks = lex("--output");
        assert_eq!(toks[0], Token::LongFlag("output".into()));
    }

    #[test]
    fn test_lex_standalone_double_dash_as_positional() {
        let toks = lex("--");
        assert_eq!(toks[0], Token::Ident("--".into()));
    }

    #[test]
    fn test_lex_long_flag_equals_value() {
        let toks = lex("--to=si");
        assert_eq!(toks[0], Token::LongFlagValue("to".into(), "si".into()));
    }

    #[test]
    fn test_lex_long_flag_equals_empty_value() {
        let toks = lex("--output-delimiter=");
        assert_eq!(
            toks[0],
            Token::LongFlagValue("output-delimiter".into(), "".into())
        );
    }

    #[test]
    fn test_lex_short_flag() {
        let toks = lex("-v");
        assert_eq!(toks[0], Token::ShortFlag('v'));
    }

    #[test]
    fn test_lex_numeric_zero_short_flag() {
        let toks = lex("-0");
        assert_eq!(toks[0], Token::ShortFlag('0'));
    }

    #[test]
    fn test_lex_tilde_prefixed_command_name() {
        let toks = lex("~uname -a");
        assert_eq!(toks[0], Token::Ident("~uname".into()));
        assert_eq!(toks[1], Token::ShortFlag('a'));
    }

    #[test]
    fn test_lex_clustered_short_flags_as_single_argv_token() {
        let toks = lex("-la");
        assert_eq!(toks[0], Token::Ident("-la".into()));
    }

    #[test]
    fn test_lex_standalone_dash_as_positional() {
        let toks = lex("join - right.txt");
        assert_eq!(toks[0], Token::Ident("join".into()));
        assert_eq!(toks[1], Token::Ident("-".into()));
        assert_eq!(toks[2], Token::Ident("right.txt".into()));
    }

    #[test]
    fn test_lex_comparisons() {
        let toks = lex("== != > >= < <=");
        assert_eq!(toks[0], Token::Eq);
        assert_eq!(toks[1], Token::Ne);
        assert_eq!(toks[2], Token::Gt);
        assert_eq!(toks[3], Token::Ge);
        assert_eq!(toks[4], Token::Lt);
        assert_eq!(toks[5], Token::Le);
    }

    #[test]
    fn test_lex_expr_operator_positionals() {
        let toks = lex("expr 10 + 1 * 2 % 3 = 1 & 1");
        assert_eq!(toks[0], Token::Ident("expr".into()));
        assert_eq!(toks[2], Token::Ident("+".into()));
        assert_eq!(toks[4], Token::Ident("*".into()));
        assert_eq!(toks[6], Token::Ident("%".into()));
        assert_eq!(toks[8], Token::Ident("=".into()));
        assert_eq!(toks[10], Token::Ident("&".into()));
    }

    #[test]
    fn test_lex_pipeline_expr() {
        let toks = lex("from json | select name | to json");
        let kinds: Vec<_> = toks.iter().map(|t| t.name()).collect();
        assert!(kinds.contains(&"identifier"));
        assert!(kinds.contains(&"`|`"));
    }

    #[test]
    fn test_lex_field_path_ident() {
        let toks = lex("where user.name == \"alice\"");
        assert!(matches!(toks[1], Token::Ident(ref s) if s == "user.name"));
    }

    #[test]
    fn test_lex_it_field_path_ident() {
        let toks = lex("where $it.user.name == \"alice\"");
        assert!(matches!(toks[1], Token::Ident(ref s) if s == "$it.user.name"));
    }

    #[test]
    fn test_lex_unquoted_absolute_path_ident() {
        let toks = lex("ls /tmp");
        assert_eq!(toks[0], Token::Ident("ls".into()));
        assert_eq!(toks[1], Token::Ident("/tmp".into()));
    }

    #[test]
    fn test_lex_unquoted_url_ident() {
        let toks = lex("http get https://example.com");
        assert_eq!(toks[0], Token::Ident("http".into()));
        assert_eq!(toks[1], Token::Ident("get".into()));
        assert_eq!(toks[2], Token::Ident("https://example.com".into()));
    }

    #[test]
    fn test_lex_unquoted_relative_path_ident() {
        let toks = lex("run ./tool ../bin/tool");
        assert_eq!(toks[0], Token::Ident("run".into()));
        assert_eq!(toks[1], Token::Ident("./tool".into()));
        assert_eq!(toks[2], Token::Ident("../bin/tool".into()));
    }

    #[test]
    fn test_lex_string_escape() {
        let toks = lex(r#""foo\nbar""#);
        assert_eq!(toks[0], Token::StrLit("foo\nbar".into()));
    }

    #[test]
    fn test_lex_single_quote_escape() {
        let toks = lex(r#"'it\'s fine'"#);
        assert_eq!(toks[0], Token::StrLit("it's fine".into()));
    }

    #[test]
    fn test_lex_unterminated_string() {
        let result = Lexer::new("\"unclosed").tokenize();
        assert!(result.is_err());
    }

    // ── Size 字面量测试 ─────────────────────────────────────

    #[test]
    fn test_lex_size_mb() {
        let toks = lex("10mb");
        assert_eq!(toks[0], Token::SizeLit(10 * 1024 * 1024));
    }

    #[test]
    fn test_lex_size_kb() {
        let toks = lex("512kb");
        assert_eq!(toks[0], Token::SizeLit(512 * 1024));
    }

    #[test]
    fn test_lex_size_gb() {
        let toks = lex("2gb");
        assert_eq!(toks[0], Token::SizeLit(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn test_lex_size_bytes() {
        let toks = lex("100b");
        assert_eq!(toks[0], Token::SizeLit(100));
    }

    // ── Duration 字面量测试 ──────────────────────────────────

    #[test]
    fn test_lex_duration_sec() {
        let toks = lex("30sec");
        assert_eq!(toks[0], Token::DurationLit(30_000_000_000));
    }

    #[test]
    fn test_lex_duration_min() {
        let toks = lex("2min");
        assert_eq!(toks[0], Token::DurationLit(120_000_000_000));
    }

    #[test]
    fn test_lex_duration_hr() {
        let toks = lex("1hr");
        assert_eq!(toks[0], Token::DurationLit(3_600_000_000_000));
    }

    #[test]
    fn test_lex_duration_ms() {
        let toks = lex("500ms");
        assert_eq!(toks[0], Token::DurationLit(500_000_000));
    }

    #[test]
    fn test_lex_duration_s() {
        let toks = lex("5s");
        assert_eq!(toks[0], Token::DurationLit(5_000_000_000));
    }

    // ── DateTime 字面量测试 ─────────────────────────────────

    #[test]
    fn test_lex_datetime_date_only() {
        let toks = lex("2025-01-01");
        // 2025-01-01T00:00:00Z = 1735689600 seconds from epoch
        let expected_nanos: i128 = 1_735_689_600 * 1_000_000_000;
        assert_eq!(toks[0], Token::DateTimeLit(expected_nanos));
    }

    #[test]
    fn test_lex_datetime_with_time() {
        let toks = lex("2025-01-01T12:30:00");
        // 2025-01-01T12:30:00Z = 1735689600 + 12*3600 + 30*60 = 1735689600 + 45000 = 1735734600
        let expected_nanos: i128 = 1_735_734_600 * 1_000_000_000;
        assert_eq!(toks[0], Token::DateTimeLit(expected_nanos));
    }

    #[test]
    fn test_lex_datetime_with_fractional_seconds() {
        let toks = lex("2025-01-01T00:00:00.500");
        let expected_nanos: i128 = 1_735_689_600_500_000_000;
        assert_eq!(toks[0], Token::DateTimeLit(expected_nanos));
    }

    #[test]
    fn test_lex_datetime_with_invalid_trailing_text_stays_ident() {
        let toks = lex("2025-01-01T00:00:00oops");
        assert!(matches!(toks[0], Token::Ident(_)));
    }

    #[test]
    fn test_lex_datetime_not_matching_stays_ident() {
        // 不匹配 datetime 格式的应该保持为 Ident
        let toks = lex("some-ident");
        assert!(matches!(toks[0], Token::Ident(_)));
    }

    #[test]
    fn test_lex_datetime_invalid_calendar_date_stays_ident() {
        let toks = lex("2025-02-31");
        assert!(matches!(toks[0], Token::Ident(_)));
    }

    #[test]
    fn test_lex_datetime_invalid_non_leap_day_stays_ident() {
        let toks = lex("2025-02-29");
        assert!(matches!(toks[0], Token::Ident(_)));
    }

    #[test]
    fn test_lex_datetime_valid_leap_day() {
        let toks = lex("2024-02-29");
        assert!(matches!(toks[0], Token::DateTimeLit(_)));
    }

    // ── 复合表达式测试 ─────────────────────────────────

    #[test]
    fn test_lex_where_with_size() {
        let toks = lex("where size > 10mb");
        assert_eq!(toks[0], Token::Ident("where".into()));
        assert_eq!(toks[1], Token::Ident("size".into()));
        assert_eq!(toks[2], Token::Gt);
        assert_eq!(toks[3], Token::SizeLit(10 * 1024 * 1024));
    }

    #[test]
    fn test_lex_plain_number_unchanged() {
        // 没有后缀的纯数字保持原样
        let toks = lex("42");
        assert_eq!(toks[0], Token::IntLit(42));
    }
}
