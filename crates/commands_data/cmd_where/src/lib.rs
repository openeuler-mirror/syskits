/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_where` — 按字段比较条件过滤 Record / List<Record>。

use ctengine::command::DataCommand;
use ctengine::compare::{compare_values, resolve_field_path};
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};

#[derive(Default)]
pub struct CmdWhere;

const WHERE_HELP: &str = r#"syskits data where

This is the syskits structured data pipeline where command.
It filters Record or List<Record> input by a field comparison expression.

Usage:
  where <field> <op> <value>
  where <cond> and <cond>
  where <cond> or <cond>
  where --help
  where --version

Operators:
  ==  !=  >  >=  <  <=

Examples:
  ps | where cpu > 1
  ps | where status == "S" and cpu < 1
"#;

#[derive(Debug, Clone)]
struct Predicate {
    field: String,
    op: String,
    rhs: CtValue,
}

#[derive(Debug, Clone)]
struct ConditionExpr {
    predicates: Vec<Predicate>,
    logic_ops: Vec<String>,
}

impl DataCommand for CmdWhere {
    fn signature(&self) -> DataSignature {
        DataSignature::new("where", "filter records by field comparison")
            .positional(CtPositionalArg::required(
                "condition",
                "comparison expr as Record {field, op, rhs}",
                CtType::Record,
            ))
            .flag(CtFlag::switch(
                "help",
                Some('h'),
                "show help for syskits data where",
            ))
            .flag(CtFlag::switch(
                "version",
                None,
                "show syskits data where version",
            ))
            .input(CtType::Any)
            .output(CtType::Any)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        if call.has_flag("help") || call.has_flag("h") {
            return Ok(meta_text_output(WHERE_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data where {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        let cond: CtValue = call
            .req::<CtValue>(0)
            .map_err(|e| CtDiagnosticError::simple(format!("where: {e}")))?;
        let expr = extract_condition(cond)?;
        let meta = CtPipelineMetadata::default();

        match input {
            CtPipelineData::Value(CtValue::Record(fields), _) => {
                if record_matches_expr(&fields, &expr) {
                    Ok(CtPipelineData::Value(CtValue::Record(fields), meta))
                } else {
                    Ok(CtPipelineData::Empty)
                }
            }
            CtPipelineData::Value(CtValue::List(items), _) => {
                let filtered: Vec<CtValue> = items
                    .into_iter()
                    .filter(|item| {
                        if let CtValue::Record(f) = item {
                            record_matches_expr(f, &expr)
                        } else {
                            false
                        }
                    })
                    .collect();
                Ok(CtPipelineData::Value(CtValue::List(filtered), meta))
            }
            CtPipelineData::Empty => Ok(CtPipelineData::Empty),
            _ => Err(CtDiagnosticError::simple(
                "where: expected Record or List input",
            )),
        }
    }
}

fn meta_text_output(text: String) -> CtPipelineData {
    CtPipelineData::Value(
        CtValue::String(text.clone()),
        CtPipelineMetadata {
            classic_text: Some(text),
            classic_bytes: None,
            classic_append_newline: false,
            exit_code: 0,
            source: Some("where".into()),
            ..Default::default()
        },
    )
}

fn extract_condition(cond: CtValue) -> Result<ConditionExpr, CtDiagnosticError> {
    if let CtValue::Record(fields) = cond {
        let get = |key: &str| -> Result<CtValue, CtDiagnosticError> {
            fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| {
                    CtDiagnosticError::simple(format!("where: missing `{key}` in condition"))
                })
        };

        // Composite where expression: {conditions: List<Record{field/op/rhs}>, logic: List<String>}
        if let Some((_, CtValue::List(raw_conds))) = fields.iter().find(|(k, _)| k == "conditions")
        {
            let logic_ops = match fields.iter().find(|(k, _)| k == "logic") {
                Some((_, CtValue::List(items))) => items
                    .iter()
                    .map(|v| match v {
                        CtValue::String(s) => Ok(s.to_ascii_lowercase()),
                        _ => Err(CtDiagnosticError::simple(
                            "where: `logic` must be a List<String>",
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => {
                    return Err(CtDiagnosticError::simple(
                        "where: `logic` must be a List<String>",
                    ));
                }
                None => Vec::new(),
            };

            let predicates = raw_conds
                .iter()
                .map(|item| match item {
                    CtValue::Record(cond_fields) => {
                        let get_cond = |key: &str| -> Result<CtValue, CtDiagnosticError> {
                            cond_fields
                                .iter()
                                .find(|(k, _)| k == key)
                                .map(|(_, v)| v.clone())
                                .ok_or_else(|| {
                                    CtDiagnosticError::simple(format!(
                                        "where: missing `{key}` in composite condition"
                                    ))
                                })
                        };
                        let field = match get_cond("field")? {
                            CtValue::String(s) => s,
                            _ => {
                                return Err(CtDiagnosticError::simple(
                                    "where: `field` must be a string",
                                ));
                            }
                        };
                        let op = match get_cond("op")? {
                            CtValue::String(s) => s,
                            _ => {
                                return Err(CtDiagnosticError::simple(
                                    "where: `op` must be a string",
                                ));
                            }
                        };
                        let rhs = get_cond("rhs")?;
                        Ok(Predicate { field, op, rhs })
                    }
                    _ => Err(CtDiagnosticError::simple(
                        "where: `conditions` must be List<Record>",
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;

            if predicates.is_empty() {
                return Err(CtDiagnosticError::simple(
                    "where: `conditions` must not be empty",
                ));
            }
            if logic_ops.len() != predicates.len().saturating_sub(1) {
                return Err(CtDiagnosticError::simple(
                    "where: `logic` count must be conditions count - 1",
                ));
            }

            return Ok(ConditionExpr {
                predicates,
                logic_ops,
            });
        }

        // Backward-compatible single condition.
        let field = match get("field")? {
            CtValue::String(s) => s,
            _ => return Err(CtDiagnosticError::simple("where: `field` must be a string")),
        };
        let op = match get("op")? {
            CtValue::String(s) => s,
            _ => return Err(CtDiagnosticError::simple("where: `op` must be a string")),
        };
        let rhs = get("rhs")?;
        Ok(ConditionExpr {
            predicates: vec![Predicate { field, op, rhs }],
            logic_ops: Vec::new(),
        })
    } else {
        Err(CtDiagnosticError::simple(
            "where: condition must be a Record",
        ))
    }
}

fn record_matches(fields: &[(String, CtValue)], field: &str, op: &str, rhs: &CtValue) -> bool {
    resolve_field_path(fields, field)
        .map(|v| compare_values(v, op, rhs))
        .unwrap_or(false)
}

fn record_matches_expr(fields: &[(String, CtValue)], expr: &ConditionExpr) -> bool {
    if expr.predicates.is_empty() {
        return false;
    }
    let mut result = record_matches(
        fields,
        &expr.predicates[0].field,
        &expr.predicates[0].op,
        &expr.predicates[0].rhs,
    );
    for (idx, pred) in expr.predicates.iter().enumerate().skip(1) {
        let current = record_matches(fields, &pred.field, &pred.op, &pred.rhs);
        match expr.logic_ops.get(idx - 1).map(|s| s.as_str()) {
            Some("and") => result = result && current,
            Some("or") => result = result || current,
            _ => return false,
        }
    }
    result
}

/// 构建测试用位置参数 DataCall（条件打包为 Record）
#[cfg(test)]
pub fn where_call(field: &str, op: &str, rhs: CtValue) -> DataCall {
    let cond = CtValue::Record(vec![
        ("field".into(), CtValue::String(field.to_string())),
        ("op".into(), CtValue::String(op.to_string())),
        ("rhs".into(), rhs),
    ]);
    let mut c = DataCall::empty();
    c.positionals.push(ctsig::BoundArg::new(cond, None));
    c
}

#[cfg(test)]
pub fn where_logic_call(conditions: Vec<(&str, &str, CtValue)>, logic: Vec<&str>) -> DataCall {
    let conds = conditions
        .into_iter()
        .map(|(field, op, rhs)| {
            CtValue::Record(vec![
                ("field".into(), CtValue::String(field.to_string())),
                ("op".into(), CtValue::String(op.to_string())),
                ("rhs".into(), rhs),
            ])
        })
        .collect::<Vec<_>>();
    let logic = logic
        .into_iter()
        .map(|s| CtValue::String(s.to_string()))
        .collect::<Vec<_>>();

    let cond = CtValue::Record(vec![
        ("conditions".into(), CtValue::List(conds)),
        ("logic".into(), CtValue::List(logic)),
    ]);
    let mut c = DataCall::empty();
    c.positionals.push(ctsig::BoundArg::new(cond, None));
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn flag_call(name: &str) -> DataCall {
        let mut call = DataCall::named("where");
        call.flags.insert(name.to_string(), None);
        call
    }

    fn rec(fields: Vec<(&str, CtValue)>) -> CtPipelineData {
        CtPipelineData::Value(
            CtValue::Record(
                fields
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            ),
            CtPipelineMetadata::default(),
        )
    }

    #[test]
    fn test_where_eq_match() {
        let r = CmdWhere
            .run(
                &where_call("s", "==", CtValue::String("a".into())),
                rec(vec![("s", CtValue::String("a".into()))]),
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(CtValue::Record(_), _)));
    }
    #[test]
    fn test_where_eq_no_match() {
        let r = CmdWhere
            .run(
                &where_call("s", "==", CtValue::String("b".into())),
                rec(vec![("s", CtValue::String("a".into()))]),
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Empty));
    }
    #[test]
    fn test_where_gt_int() {
        let r = CmdWhere
            .run(
                &where_call("n", ">", CtValue::Int(10)),
                rec(vec![("n", CtValue::Int(20))]),
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(_, _)));
    }
    #[test]
    fn test_where_list_filter() {
        let list = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![("v".into(), CtValue::Int(1))]),
                CtValue::Record(vec![("v".into(), CtValue::Int(5))]),
                CtValue::Record(vec![("v".into(), CtValue::Int(3))]),
            ]),
            CtPipelineMetadata::default(),
        );
        let r = CmdWhere
            .run(&where_call("v", ">=", CtValue::Int(3)), list, &ctx())
            .unwrap();
        if let CtPipelineData::Value(CtValue::List(items), _) = r {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected List");
        }
    }
    #[test]
    fn test_where_ne() {
        let r = CmdWhere
            .run(
                &where_call("f", "!=", CtValue::Bool(true)),
                rec(vec![("f", CtValue::Bool(false))]),
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(_, _)));
    }

    #[test]
    fn test_where_logic_and() {
        let call = where_logic_call(
            vec![("n", ">", CtValue::Int(1)), ("m", "<", CtValue::Int(10))],
            vec!["and"],
        );
        let r = CmdWhere
            .run(
                &call,
                rec(vec![("n", CtValue::Int(3)), ("m", CtValue::Int(5))]),
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(_, _)));
    }

    #[test]
    fn test_where_logic_or() {
        let call = where_logic_call(
            vec![("n", ">", CtValue::Int(100)), ("m", "<", CtValue::Int(10))],
            vec!["or"],
        );
        let r = CmdWhere
            .run(
                &call,
                rec(vec![("n", CtValue::Int(3)), ("m", CtValue::Int(5))]),
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(_, _)));
    }

    #[test]
    fn test_where_nested_field_access() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![(
                "user".into(),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("alice".into())),
                    ("age".into(), CtValue::Int(20)),
                ]),
            )]),
            CtPipelineMetadata::default(),
        );
        let r = CmdWhere
            .run(
                &where_call("user.age", ">=", CtValue::Int(18)),
                input,
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(_, _)));
    }

    #[test]
    fn test_where_it_prefixed_field_access() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![(
                "user".into(),
                CtValue::Record(vec![("age".into(), CtValue::Int(20))]),
            )]),
            CtPipelineMetadata::default(),
        );
        let r = CmdWhere
            .run(
                &where_call("$it.user.age", ">=", CtValue::Int(18)),
                input,
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(_, _)));
    }

    #[test]
    fn test_where_help_output() {
        let out = CmdWhere
            .run(&flag_call("help"), CtPipelineData::Empty, &ctx())
            .expect("help should not require input");
        let CtPipelineData::Value(CtValue::String(text), meta) = out else {
            panic!("expected help text");
        };
        assert!(text.contains("syskits structured data pipeline where command"));
        assert_eq!(meta.exit_code, 0);
    }

    #[test]
    fn test_where_version_output() {
        let out = CmdWhere
            .run(&flag_call("version"), CtPipelineData::Empty, &ctx())
            .expect("version should not require input");
        let CtPipelineData::Value(CtValue::String(text), meta) = out else {
            panic!("expected version text");
        };
        assert!(text.starts_with("syskits data where "));
        assert_eq!(meta.exit_code, 0);
    }
}
