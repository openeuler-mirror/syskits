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

//! `LegacyToolAdapter` — legacy Tool resolver marker.
//!
//! 设计：
//! - 通过 resolver 查找 legacy `Tool`
//! - 不再 in-process 执行 legacy `Tool`
//! - 未注册的 data 命令统一交给 external fallback，避免全局 stdin/stdout FD 劫持

use crate::error::CtDiagnosticError;
use ctcore::Tool;
use ctdsl::ast::Call;
#[cfg(test)]
use ctdsl::ast::{Arg, Lit};
use ctpipeline::CtPipelineData;
#[cfg(test)]
use std::ffi::OsString;

/// legacy 工具查找函数类型
pub type LegacyToolResolver = fn(&str) -> Option<Box<dyn Tool>>;

/// legacy 工具 in-process 适配器
#[derive(Clone, Copy)]
pub struct LegacyToolAdapter {
    resolver: LegacyToolResolver,
}

impl LegacyToolAdapter {
    pub fn new(resolver: LegacyToolResolver) -> Self {
        Self { resolver }
    }

    pub fn can_resolve(&self, name: &str) -> bool {
        let _ = name;
        let _ = self.resolver;
        false
    }

    /// Legacy in-process execution is intentionally disabled.
    ///
    /// The data interpreter keeps this type only as a compatibility marker for
    /// older entry points. Unknown commands fall through to the external binary
    /// adapter, which uses child-process stdio pipes instead of mutating the
    /// current process' stdin/stdout file descriptors.
    pub fn run_call(
        &self,
        _call: &Call,
        _input: CtPipelineData,
    ) -> Result<Option<CtPipelineData>, CtDiagnosticError> {
        Ok(None)
    }
}

impl std::fmt::Debug for LegacyToolAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LegacyToolAdapter").finish_non_exhaustive()
    }
}

#[cfg(test)]
fn build_legacy_args(call: &Call) -> Vec<OsString> {
    let mut args = Vec::with_capacity(call.args.len() + 1);
    args.push(OsString::from(call.name.clone()));
    for arg in &call.args {
        match arg {
            Arg::Positional { value, .. } => args.push(lit_to_os(value)),
            Arg::LongFlag { name, .. } => args.push(OsString::from(format!("--{name}"))),
            Arg::LongFlagValue { name, value, .. } => {
                args.push(OsString::from(format!("--{name}")));
                args.push(lit_to_os(value));
            }
            Arg::ShortFlag { name, .. } => args.push(OsString::from(format!("-{name}"))),
            Arg::Comparison { field, op, rhs, .. } => {
                args.push(OsString::from(field));
                args.push(OsString::from(op.symbol()));
                args.push(lit_to_os(rhs));
            }
            Arg::WhereExpr {
                conditions,
                logic_ops,
                ..
            } => {
                for (idx, (field, op, rhs)) in conditions.iter().enumerate() {
                    if idx > 0
                        && let Some(logic) = logic_ops.get(idx - 1)
                    {
                        args.push(OsString::from(logic));
                    }
                    args.push(OsString::from(field));
                    args.push(OsString::from(op.symbol()));
                    args.push(lit_to_os(rhs));
                }
            }
        }
    }
    args
}

#[cfg(test)]
fn lit_to_os(lit: &Lit) -> OsString {
    match lit {
        Lit::Int(n) => OsString::from(n.to_string()),
        Lit::Float(f) => OsString::from(f.to_string()),
        Lit::String(s) => OsString::from(s),
        Lit::Bool(b) => OsString::from(b.to_string()),
        Lit::Size(b) => OsString::from(b.to_string()),
        Lit::Duration(ns) => OsString::from(ns.to_string()),
        Lit::DateTime(ns) => OsString::from(ns.to_string()),
        Lit::Ident(s) => OsString::from(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctdsl::ast::CompOp;
    use ctpipeline::CtSpan;

    fn inline_span(start: usize, end: usize) -> CtSpan {
        CtSpan::inline(start, end, 1, (start + 1).try_into().unwrap())
    }

    fn call_with_args(args: Vec<Arg>) -> Call {
        Call {
            name: "echo".to_string(),
            args,
            span: inline_span(0, 4),
        }
    }

    #[test]
    fn test_build_legacy_args_all_variants() {
        let call = call_with_args(vec![
            Arg::Positional {
                value: Lit::String("hello".to_string()),
                span: inline_span(5, 10),
            },
            Arg::LongFlag {
                name: "n".to_string(),
                span: inline_span(11, 14),
            },
            Arg::LongFlagValue {
                name: "fmt".to_string(),
                value: Lit::String("json".to_string()),
                value_span: inline_span(21, 25),
                span: inline_span(15, 25),
            },
            Arg::ShortFlag {
                name: 'v',
                span: inline_span(26, 28),
            },
            Arg::Comparison {
                field: "age".to_string(),
                op: CompOp::Ge,
                rhs: Lit::Int(18),
                span: inline_span(29, 39),
            },
        ]);

        let args = build_legacy_args(&call);
        let args: Vec<String> = args
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();

        assert_eq!(
            args,
            vec![
                "echo", "hello", "--n", "--fmt", "json", "-v", "age", ">=", "18"
            ]
        );
    }

    #[test]
    fn test_run_call_unknown_returns_none() {
        fn resolver(_name: &str) -> Option<Box<dyn Tool>> {
            None
        }

        let adapter = LegacyToolAdapter::new(resolver);
        let call = call_with_args(vec![]);
        let out = adapter.run_call(&call, CtPipelineData::Empty).unwrap();
        assert!(out.is_none());
    }
}
