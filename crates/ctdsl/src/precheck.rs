/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

use crate::Expr;
use ctpipeline::{CtSpan, CtType};
use ctsig::DataSignature;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecheckLevel {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecheckDiagnostic {
    pub level: PrecheckLevel,
    pub message: String,
    pub stage_index: usize,
    pub span: Option<CtSpan>,
}

impl PrecheckDiagnostic {
    pub(crate) fn error(
        stage_index: usize,
        span: Option<CtSpan>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            level: PrecheckLevel::Error,
            message: message.into(),
            stage_index,
            span,
        }
    }

    pub(crate) fn warning(
        stage_index: usize,
        span: Option<CtSpan>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            level: PrecheckLevel::Warning,
            message: message.into(),
            stage_index,
            span,
        }
    }
}

fn compatible(expected: CtType, actual: CtType) -> bool {
    if expected == CtType::Any || actual == CtType::Any {
        return true;
    }
    if expected == actual {
        return true;
    }
    matches!(
        (expected, actual),
        (CtType::List, CtType::ListStream) | (CtType::ListStream, CtType::List)
    )
}

/// 轻量静态 precheck：基于签名的 stage 输入输出链检查。
pub fn precheck_expr(
    expr: &Expr,
    signatures: &HashMap<String, DataSignature>,
) -> Vec<PrecheckDiagnostic> {
    let mut diags = Vec::new();
    let mut current = CtType::Any;

    for (idx, stage) in expr.stages().iter().enumerate() {
        if stage.force_external {
            diags.push(PrecheckDiagnostic::warning(
                idx,
                Some(stage.span.clone()),
                format!(
                    "precheck: external command `{}`; skip type-chain check",
                    stage.name
                ),
            ));
            current = CtType::Any;
            continue;
        }

        let Some(sig) = signatures.get(&stage.name) else {
            diags.push(PrecheckDiagnostic::warning(
                idx,
                Some(stage.span.clone()),
                format!(
                    "precheck: unknown command `{}`; skip type-chain check",
                    stage.name
                ),
            ));
            // Unknown stage may transform data shape; break the inferred chain
            // so downstream checks are conservative.
            current = CtType::Any;
            continue;
        };

        if let Some(expected) = sig.input_type {
            if !compatible(expected, current) {
                diags.push(PrecheckDiagnostic::error(
                    idx,
                    Some(stage.span.clone()),
                    format!(
                        "precheck: stage `{}` expects input {:?}, got {:?}",
                        stage.name, expected, current
                    ),
                ));
            }
        }

        if let Some(out) = sig.output_type {
            current = out;
        } else if sig.input_type.is_none() {
            current = CtType::Any;
        }
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;
    use ctsig::DataSignature;

    fn sig(name: &'static str, input: Option<CtType>, output: Option<CtType>) -> DataSignature {
        let mut s = DataSignature::new(name, "test");
        if let Some(i) = input {
            s = s.input(i);
        }
        if let Some(o) = output {
            s = s.output(o);
        }
        s
    }

    #[test]
    fn test_precheck_error_and_warning_levels() {
        let expr = parse("from json | to json | unknown_cmd").expect("parse should pass");
        let mut table = HashMap::new();
        table.insert(
            "from".to_string(),
            sig("from", Some(CtType::String), Some(CtType::Record)),
        );
        table.insert(
            "to".to_string(),
            sig("to", Some(CtType::String), Some(CtType::String)),
        );
        let diags = precheck_expr(&expr, &table);
        assert_eq!(diags.len(), 2);
        assert!(
            diags
                .iter()
                .any(|d| matches!(d.level, PrecheckLevel::Warning))
        );
        assert!(
            diags
                .iter()
                .any(|d| matches!(d.level, PrecheckLevel::Error))
        );
    }

    #[test]
    fn test_precheck_deterministic_for_same_input() {
        let expr = parse("from json | select name").expect("parse should pass");
        let mut table = HashMap::new();
        table.insert(
            "from".to_string(),
            sig("from", Some(CtType::String), Some(CtType::Record)),
        );
        table.insert(
            "select".to_string(),
            sig("select", Some(CtType::Record), Some(CtType::Record)),
        );
        let a = precheck_expr(&expr, &table);
        let b = precheck_expr(&expr, &table);
        assert_eq!(a, b);
    }

    #[test]
    fn test_precheck_unknown_stage_resets_inferred_type_chain() {
        let expr = parse("from json | plugin_x | to json").expect("parse should pass");
        let mut table = HashMap::new();
        table.insert(
            "from".to_string(),
            sig("from", Some(CtType::String), Some(CtType::Record)),
        );
        table.insert(
            "to".to_string(),
            sig("to", Some(CtType::String), Some(CtType::String)),
        );

        let diags = precheck_expr(&expr, &table);
        assert!(
            diags
                .iter()
                .any(|d| matches!(d.level, PrecheckLevel::Warning) && d.stage_index == 1)
        );
        assert!(
            !diags
                .iter()
                .any(|d| matches!(d.level, PrecheckLevel::Error) && d.stage_index == 2),
            "unknown stage should invalidate upstream inferred type to avoid false hard errors"
        );
    }

    #[test]
    fn test_precheck_forced_external_skips_matching_internal_signature() {
        let expr = parse("~select name").expect("parse should pass");
        let mut table = HashMap::new();
        table.insert(
            "select".to_string(),
            sig("select", Some(CtType::Record), Some(CtType::Record)),
        );

        let diags = precheck_expr(&expr, &table);
        assert_eq!(diags.len(), 1);
        assert!(matches!(diags[0].level, PrecheckLevel::Warning));
        assert!(diags[0].message.contains("external command `select`"));
    }

    #[test]
    fn test_precheck_list_to_record_is_not_implicitly_compatible() {
        let expr = parse("listify | select name").expect("parse should pass");
        let mut table = HashMap::new();
        table.insert(
            "listify".to_string(),
            sig("listify", Some(CtType::Any), Some(CtType::List)),
        );
        table.insert(
            "select".to_string(),
            sig("select", Some(CtType::Record), Some(CtType::Record)),
        );

        let diags = precheck_expr(&expr, &table);
        assert!(
            diags.iter().any(|d| {
                matches!(d.level, PrecheckLevel::Error)
                    && d.message.contains("expects input Record")
                    && d.message.contains("got List")
            }),
            "list input should not silently pass record expectation"
        );
    }
}
