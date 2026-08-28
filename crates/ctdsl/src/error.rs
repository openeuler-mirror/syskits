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

//! `ParseError` — ctdsl 词法/语法错误。

use ctpipeline::CtSpan;
use thiserror::Error;

/// ctdsl 解析错误
#[derive(Debug, Clone, Error)]
pub enum ParseError {
    #[error("{span}: {message}")]
    LexError { message: String, span: CtSpan },

    #[error("{span}: {message}")]
    SyntaxError { message: String, span: CtSpan },

    #[error("unexpected end of input")]
    UnexpectedEof,
}

impl ParseError {
    /// 附带 span 的语法错误
    pub fn syntax(message: impl Into<String>, span: CtSpan) -> Self {
        ParseError::SyntaxError {
            message: message.into(),
            span,
        }
    }

    /// 退出码（始终为 1）
    pub fn code(&self) -> i32 {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctpipeline::CtSpan;

    #[test]
    fn test_parse_error_display_lex() {
        let span = CtSpan::inline(0, 1, 1, 1);
        let err = ParseError::LexError {
            message: "bad char".into(),
            span,
        };
        let s = err.to_string();
        assert!(s.contains("bad char"), "got: {s}");
    }

    #[test]
    fn test_parse_error_code() {
        assert_eq!(ParseError::UnexpectedEof.code(), 1);
    }
}
