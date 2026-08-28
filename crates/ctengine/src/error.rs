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

//! `CtDiagnosticError` — 数据管线的诊断错误类型。
//!
//! 实现了 `CTError` 以兼容现有退出码机制，同时提供精确的源码位置（`CtSpan`）和原因链。
//!
//! Display 格式：
//! - 有 span：`<source>:<line>:<col>: <message>` (+ 可选的 `caused by: ...` 附加行)
//! - 无 span：`<message>` (+ 可选的 `caused by: ...` 附加行)

use ctcore::ct_error::CTError;
use ctpipeline::CtSpan;
use std::fmt;

/// 数据管线诊断错误
#[derive(Debug)]
pub struct CtDiagnosticError {
    /// 错误消息（人类可读）
    pub message: String,
    /// 源码位置（可选）
    pub span: Option<CtSpan>,
    /// 根因（可选，支持链式 `caused by`）
    pub cause: Option<Box<dyn std::error::Error + Send + Sync>>,
    /// 进程退出码（默认 1）
    pub code: i32,
}

impl CtDiagnosticError {
    /// 创建带 span 的诊断错误（退出码 1）
    pub fn with_span(message: impl Into<String>, span: CtSpan) -> Self {
        Self {
            message: message.into(),
            span: Some(span),
            cause: None,
            code: 1,
        }
    }

    /// 创建无 span 的诊断错误（退出码 1）
    pub fn simple(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
            cause: None,
            code: 1,
        }
    }

    /// 附加根因
    pub fn caused_by(mut self, cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }

    /// 设置自定义退出码
    pub fn with_code(mut self, code: i32) -> Self {
        self.code = code;
        self
    }
}

impl fmt::Display for CtDiagnosticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = &self.span {
            write!(f, "{}: {}", span, self.message)?;
        } else {
            write!(f, "{}", self.message)?;
        }
        if let Some(cause) = &self.cause {
            write!(f, "\ncaused by: {cause}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CtDiagnosticError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl CTError for CtDiagnosticError {
    fn code(&self) -> i32 {
        self.code
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctpipeline::CtSpan;
    use std::error::Error as StdError; // 使 .source() 方法可见

    #[test]
    fn test_diagnostic_error_display_with_span() {
        let span = CtSpan::file("script.ct", 0, 5, 1, 1);
        let err = CtDiagnosticError::with_span("unexpected token", span);
        let s = err.to_string();
        assert!(s.contains("script.ct:1:1"), "got: {s}");
        assert!(s.contains("unexpected token"), "got: {s}");
    }

    #[test]
    fn test_diagnostic_error_display_no_span() {
        let err = CtDiagnosticError::simple("command not found: foo");
        let s = err.to_string();
        assert_eq!(s, "command not found: foo");
    }

    #[test]
    fn test_diagnostic_error_implements_cterror() {
        let err = CtDiagnosticError::simple("err").with_code(42);
        assert_eq!(err.code(), 42);
    }

    #[test]
    fn test_diagnostic_error_default_code() {
        let err = CtDiagnosticError::simple("err");
        assert_eq!(err.code(), 1);
    }

    #[test]
    fn test_diagnostic_error_caused_by() {
        use std::fmt;
        #[derive(Debug)]
        struct Cause;
        impl fmt::Display for Cause {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "root cause")
            }
        }
        impl std::error::Error for Cause {}

        let err = CtDiagnosticError::simple("outer").caused_by(Cause);
        let s = err.to_string();
        assert!(s.contains("outer"));
        assert!(s.contains("root cause"));
    }

    #[test]
    fn test_diagnostic_error_source_chain() {
        use std::fmt;
        #[derive(Debug)]
        struct Inner;
        impl fmt::Display for Inner {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "inner")
            }
        }
        impl std::error::Error for Inner {}

        let err = CtDiagnosticError::simple("outer").caused_by(Inner);
        assert!(err.source().is_some());
    }
}
