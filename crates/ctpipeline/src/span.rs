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

//! `CtSpan` — 源码位置标注，用于精确定位诊断错误。
//!
//! 设计约束：
//! - `start`/`end` 为字节偏移量（非字符偏移），与 UTF-8 切片直接对应
//! - `start_line`/`start_col` 为 1-based 行列号，仅用于显示
//! - `CtSourceRef::Internal` 用于运行时内部生成的 span，无文件来源

/// 源码位置引用
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CtSourceRef {
    /// 来自标准输入
    Stdin,
    /// 来自具名文件
    File(String),
    /// 来自内联表达式字符串（例如 `syskits data -c '...'`）
    InlineExpr,
    /// 运行时内部生成，无对应源码位置
    Internal,
}

impl std::fmt::Display for CtSourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CtSourceRef::Stdin => write!(f, "<stdin>"),
            CtSourceRef::File(path) => write!(f, "{path}"),
            CtSourceRef::InlineExpr => write!(f, "<expr>"),
            CtSourceRef::Internal => write!(f, "<internal>"),
        }
    }
}

/// 源码位置标注（字节偏移 + 行列号）
///
/// 不变量：`start <= end`，行列号均为 1-based。
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CtSpan {
    /// 源码引用
    pub source: CtSourceRef,
    /// 起始字节偏移（inclusive）
    pub start: usize,
    /// 结束字节偏移（exclusive）
    pub end: usize,
    /// 起始行号（1-based）
    pub start_line: u32,
    /// 起始列号（1-based，按字节计）
    pub start_col: u32,
}

impl CtSpan {
    /// 创建一个指向文件的 span
    pub fn file(path: impl Into<String>, start: usize, end: usize, line: u32, col: u32) -> Self {
        debug_assert!(start <= end, "CtSpan: start must be <= end");
        Self {
            source: CtSourceRef::File(path.into()),
            start,
            end,
            start_line: line,
            start_col: col,
        }
    }

    /// 创建一个内部（无来源）span
    pub fn internal() -> Self {
        Self {
            source: CtSourceRef::Internal,
            start: 0,
            end: 0,
            start_line: 0,
            start_col: 0,
        }
    }

    /// 创建一个来自内联表达式的 span
    pub fn inline(start: usize, end: usize, line: u32, col: u32) -> Self {
        debug_assert!(start <= end, "CtSpan: start must be <= end");
        Self {
            source: CtSourceRef::InlineExpr,
            start,
            end,
            start_line: line,
            start_col: col,
        }
    }

    /// 返回 span 的字节长度
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// 是否为空 span
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

impl std::fmt::Display for CtSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.source, self.start_line, self.start_col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ctspan_byte_offsets() {
        let span = CtSpan::file("test.ct", 0, 5, 1, 1);
        assert_eq!(span.start, 0);
        assert_eq!(span.end, 5);
        assert_eq!(span.start_line, 1);
        assert_eq!(span.start_col, 1);
        assert_eq!(span.len(), 5);
        assert!(!span.is_empty());
    }

    #[test]
    fn test_ctspan_internal() {
        let span = CtSpan::internal();
        assert_eq!(span.source, CtSourceRef::Internal);
        assert_eq!(span.len(), 0);
        assert!(span.is_empty());
    }

    #[test]
    fn test_ctspan_display_file() {
        let span = CtSpan::file("script.ct", 10, 20, 3, 5);
        assert_eq!(span.to_string(), "script.ct:3:5");
    }

    #[test]
    fn test_ctspan_display_stdin() {
        let span = CtSpan {
            source: CtSourceRef::Stdin,
            start: 0,
            end: 0,
            start_line: 1,
            start_col: 1,
        };
        assert_eq!(span.to_string(), "<stdin>:1:1");
    }

    #[test]
    fn test_ctsourceref_display() {
        assert_eq!(CtSourceRef::Stdin.to_string(), "<stdin>");
        assert_eq!(CtSourceRef::File("f.ct".into()).to_string(), "f.ct");
        assert_eq!(CtSourceRef::InlineExpr.to_string(), "<expr>");
        assert_eq!(CtSourceRef::Internal.to_string(), "<internal>");
    }

    #[test]
    fn test_ctspan_inline() {
        let span = CtSpan::inline(2, 8, 1, 3);
        assert_eq!(span.source, CtSourceRef::InlineExpr);
        assert_eq!(span.len(), 6);
    }
}
