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

//! `AST` — ctdsl 抽象语法树节点定义。
//!
//! M1b 支持的语法子集：
//! - `Pipeline`：`a | b | c`
//! - `Call`：命令调用（名称 + 参数列表）
//! - `Arg`：位置参数（字面量）/ 标志参数（--flag [value]）
//! - `Lit`：整数、浮点、字符串、布尔、标识符引用
//! - `Comparison`：`field op value`（用于 `where` 命令）

use ctpipeline::CtSpan;

/// 比较运算符
#[derive(Debug, Clone, PartialEq)]
pub enum CompOp {
    Eq, // ==
    Ne, // !=
    Gt, // >
    Ge, // >=
    Lt, // <
    Le, // <=
}

impl CompOp {
    /// 运算符符号字符串
    pub fn symbol(&self) -> &'static str {
        match self {
            CompOp::Eq => "==",
            CompOp::Ne => "!=",
            CompOp::Gt => ">",
            CompOp::Ge => ">=",
            CompOp::Lt => "<",
            CompOp::Le => "<=",
        }
    }
}

/// 字面量（叶节点）
#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    /// 字节大小（存储字节数）
    Size(u64),
    /// 纳秒时长
    Duration(i64),
    /// Unix 纳秒时间戳
    DateTime(i128),
    /// 字段名或命令名引用（未解析的标识符）
    Ident(String),
}

impl std::fmt::Display for Lit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Lit::Int(n) => write!(f, "{n}"),
            Lit::Float(v) => write!(f, "{v}"),
            Lit::String(s) => write!(f, "{s}"),
            Lit::Bool(b) => write!(f, "{b}"),
            Lit::Size(b) => write!(f, "{b}B"),
            Lit::Duration(ns) => write!(f, "{ns}ns"),
            Lit::DateTime(ns) => write!(f, "<datetime:{ns}>"),
            Lit::Ident(s) => write!(f, "{s}"),
        }
    }
}

/// 命令参数
#[derive(Debug, Clone)]
pub enum Arg {
    /// 位置参数：紧跟命令名后的字面量
    Positional { value: Lit, span: CtSpan },
    /// 长标志 `--name`（无值）
    LongFlag { name: String, span: CtSpan },
    /// 长标志带值 `--name value`
    LongFlagValue {
        name: String,
        value: Lit,
        value_span: CtSpan,
        span: CtSpan,
    },
    /// 短标志 `-x`
    ShortFlag { name: char, span: CtSpan },
    /// `where` 专用：比较表达式 `field op value`
    Comparison {
        field: String,
        op: CompOp,
        rhs: Lit,
        span: CtSpan,
    },
    /// `where` 扩展：逻辑组合条件 `a > 1 and b < 2`
    WhereExpr {
        conditions: Vec<(String, CompOp, Lit)>,
        logic_ops: Vec<String>,
        span: CtSpan,
    },
}

/// 单条命令调用节点
#[derive(Debug, Clone)]
pub struct Call {
    /// 命令名（已解析为字符串）
    pub name: String,
    /// 是否强制通过 PATH 执行外部命令（由 `~cmd` 语法触发）
    pub force_external: bool,
    /// 参数列表（顺序与原始表达式一致）
    pub args: Vec<Arg>,
    /// 整条调用的 span（从命令名开始）
    pub span: CtSpan,
}

/// 顶层表达式（管线）
#[derive(Debug, Clone)]
pub enum Expr {
    /// 管线：`a | b | c`（至少有一个 Call）
    Pipeline(Vec<Call>),
}

impl Expr {
    /// 是否为空管线（不应出现，防御性检查）
    pub fn is_empty_pipeline(&self) -> bool {
        match self {
            Expr::Pipeline(calls) => calls.is_empty(),
        }
    }

    /// 返回管线各阶段 call 列表（便于解释器迭代）
    pub fn stages(&self) -> &[Call] {
        match self {
            Expr::Pipeline(calls) => calls,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comp_op_symbol() {
        assert_eq!(CompOp::Eq.symbol(), "==");
        assert_eq!(CompOp::Le.symbol(), "<=");
    }

    #[test]
    fn test_lit_display() {
        assert_eq!(Lit::Int(42).to_string(), "42");
        assert_eq!(Lit::String("hello".into()).to_string(), "hello");
        assert_eq!(Lit::Bool(true).to_string(), "true");
        assert_eq!(Lit::Ident("name".into()).to_string(), "name");
    }

    #[test]
    fn test_expr_stages() {
        use ctpipeline::CtSpan;
        let call = Call {
            name: "ls".into(),
            force_external: false,
            args: vec![],
            span: CtSpan::inline(0, 2, 1, 1),
        };
        let expr = Expr::Pipeline(vec![call]);
        assert_eq!(expr.stages().len(), 1);
        assert!(!expr.is_empty_pipeline());
    }
}
