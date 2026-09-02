/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_agg` - 对 List<Record> 或 group-by 结果做聚合。

use ctengine::command::DataCommand;
use ctengine::compare::{ct_cmp, resolve_field_path};
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::cmp::Ordering;
use std::collections::HashSet;

#[derive(Default)]
pub struct CmdAgg;

const AGG_HELP: &str = r#"syskits data agg

This is the syskits structured data pipeline agg command.
It aggregates List<Record> input, including grouped rows from group-by.

Usage:
  agg <op> [ops...]
  agg --help
  agg --version

Operations:
  count
  sum:<field>[=<alias>]
  avg:<field>[=<alias>]
  min:<field>[=<alias>]
  max:<field>[=<alias>]

Examples:
  ps | agg count avg:cpu=max_avg
  ps | group-by status | agg count avg:cpu
"#;

impl DataCommand for CmdAgg {
    fn signature(&self) -> DataSignature {
        DataSignature::new(
            "agg",
            "aggregate data with count/sum/avg/min/max (supports grouped input)",
        )
        .positional(CtPositionalArg::required(
            "ops",
            "aggregation specs, e.g. count sum:bytes avg:bytes=max_avg",
            CtType::String,
        ))
        .rest(CtPositionalArg::optional(
            "ops",
            "additional aggregation specs",
            CtType::String,
        ))
        .flag(CtFlag::switch(
            "help",
            Some('h'),
            "show help for syskits data agg",
        ))
        .flag(CtFlag::switch(
            "version",
            None,
            "show syskits data agg version",
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
            return Ok(meta_text_output(AGG_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data agg {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        let ops = parse_ops(call)?;
        let rows = normalize_input_rows(input)?;

        if is_grouped_rows(&rows)? {
            let mut out = Vec::new();
            for item in rows {
                let CtValue::Record(fields) = item else {
                    continue;
                };
                let group_fields = extract_group_fields(&fields)?;
                let grouped_rows = extract_rows(&fields)?;
                let agg_fields = aggregate_fields(&grouped_rows, &ops)?;
                let merged = merge_grouped_agg_output(group_fields, agg_fields)?;
                out.push(CtValue::Record(merged));
            }
            Ok(CtPipelineData::Value(
                CtValue::List(out),
                CtPipelineMetadata::default(),
            ))
        } else {
            let agg = aggregate_fields(&rows, &ops)?;
            Ok(CtPipelineData::Value(
                CtValue::Record(agg),
                CtPipelineMetadata::default(),
            ))
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
            source: Some("agg".into()),
            ..Default::default()
        },
    )
}

#[derive(Debug, Clone)]
enum AggKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(Debug, Clone)]
struct AggOp {
    kind: AggKind,
    field: Option<String>,
    alias: String,
}

fn parse_ops(call: &DataCall) -> Result<Vec<AggOp>, CtDiagnosticError> {
    let specs = call
        .rest::<String>(0)
        .map_err(|e| CtDiagnosticError::simple(format!("agg: {e}")))?;
    if specs.is_empty() {
        return Err(CtDiagnosticError::simple(
            "agg: at least one aggregation spec is required",
        ));
    }

    specs.into_iter().map(|spec| parse_spec(&spec)).collect()
}

fn parse_spec(raw: &str) -> Result<AggOp, CtDiagnosticError> {
    let spec = raw.trim();
    if spec.is_empty() {
        return Err(CtDiagnosticError::simple(
            "agg: aggregation spec must not be empty",
        ));
    }

    let (expr, alias_opt) = match spec.split_once('=') {
        Some((left, right)) => (left.trim(), Some(right.trim())),
        None => (spec, None),
    };
    let alias_opt = alias_opt.filter(|s| !s.is_empty()).map(|s| s.to_string());

    if expr.eq_ignore_ascii_case("count") {
        return Ok(AggOp {
            kind: AggKind::Count,
            field: None,
            alias: alias_opt.unwrap_or_else(|| "count".to_string()),
        });
    }

    let (func, field) = expr
        .split_once(':')
        .ok_or_else(|| CtDiagnosticError::simple(format!("agg: invalid spec `{spec}`")))?;
    let func = func.trim().to_ascii_lowercase();
    let field = field.trim();
    if field.is_empty() {
        return Err(CtDiagnosticError::simple(format!(
            "agg: missing field in spec `{spec}`"
        )));
    }

    let kind = match func.as_str() {
        "sum" => AggKind::Sum,
        "avg" => AggKind::Avg,
        "min" => AggKind::Min,
        "max" => AggKind::Max,
        _ => {
            return Err(CtDiagnosticError::simple(format!(
                "agg: unsupported function `{func}`"
            )));
        }
    };
    let alias = alias_opt.unwrap_or_else(|| default_alias(&func, field));
    Ok(AggOp {
        kind,
        field: Some(field.to_string()),
        alias,
    })
}

fn default_alias(func: &str, field: &str) -> String {
    let normalized = field
        .trim_start_matches("$it.")
        .trim_start_matches("it.")
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    format!("{func}_{normalized}")
}

fn normalize_input_rows(input: CtPipelineData) -> Result<Vec<CtValue>, CtDiagnosticError> {
    match input {
        CtPipelineData::Value(CtValue::List(items), _) => Ok(items),
        CtPipelineData::ListStream(stream) => Ok(stream.collect()),
        CtPipelineData::Empty => Ok(Vec::new()),
        _ => Err(CtDiagnosticError::simple("agg: expected List input")),
    }
}

fn is_grouped_rows(items: &[CtValue]) -> Result<bool, CtDiagnosticError> {
    if items.is_empty() {
        return Ok(false);
    }
    for item in items {
        let CtValue::Record(fields) = item else {
            return Ok(false);
        };
        let group = fields.iter().find(|(k, _)| k == "group").map(|(_, v)| v);
        let rows = fields.iter().find(|(k, _)| k == "rows").map(|(_, v)| v);
        match (group, rows) {
            (Some(CtValue::Record(_)), Some(CtValue::List(_))) => {}
            (None, None) => return Ok(false),
            _ => {
                return Err(CtDiagnosticError::simple(
                    "agg: grouped input item must contain `group: Record` and `rows: List`",
                ));
            }
        }
    }
    Ok(true)
}

fn extract_group_fields(
    fields: &[(String, CtValue)],
) -> Result<Vec<(String, CtValue)>, CtDiagnosticError> {
    let group = fields
        .iter()
        .find(|(k, _)| k == "group")
        .map(|(_, v)| v)
        .ok_or_else(|| CtDiagnosticError::simple("agg: missing `group` field"))?;
    let CtValue::Record(group_fields) = group else {
        return Err(CtDiagnosticError::simple(
            "agg: `group` field must be Record",
        ));
    };
    Ok(group_fields.clone())
}

fn extract_rows(fields: &[(String, CtValue)]) -> Result<Vec<CtValue>, CtDiagnosticError> {
    let rows = fields
        .iter()
        .find(|(k, _)| k == "rows")
        .map(|(_, v)| v)
        .ok_or_else(|| CtDiagnosticError::simple("agg: missing `rows` field"))?;
    let CtValue::List(rows) = rows else {
        return Err(CtDiagnosticError::simple("agg: `rows` field must be List"));
    };
    Ok(rows.clone())
}

fn aggregate_fields(
    rows: &[CtValue],
    ops: &[AggOp],
) -> Result<Vec<(String, CtValue)>, CtDiagnosticError> {
    let mut out = Vec::with_capacity(ops.len());
    let mut seen_aliases: HashSet<String> = HashSet::with_capacity(ops.len());
    for op in ops {
        if !seen_aliases.insert(op.alias.clone()) {
            return Err(CtDiagnosticError::simple(format!(
                "agg: duplicate output field `{}`; use explicit aliases to disambiguate",
                op.alias
            )));
        }
        let value = match op.kind {
            AggKind::Count => CtValue::Int(rows.len() as i64),
            AggKind::Sum => aggregate_sum(rows, op.field.as_deref().unwrap_or_default())?,
            AggKind::Avg => aggregate_avg(rows, op.field.as_deref().unwrap_or_default())?,
            AggKind::Min => aggregate_minmax(rows, op.field.as_deref().unwrap_or_default(), true)?,
            AggKind::Max => aggregate_minmax(rows, op.field.as_deref().unwrap_or_default(), false)?,
        };
        out.push((op.alias.clone(), value));
    }
    Ok(out)
}

fn merge_grouped_agg_output(
    group_fields: Vec<(String, CtValue)>,
    agg_fields: Vec<(String, CtValue)>,
) -> Result<Vec<(String, CtValue)>, CtDiagnosticError> {
    let mut merged = Vec::with_capacity(group_fields.len() + agg_fields.len());
    let mut seen: HashSet<String> = HashSet::with_capacity(group_fields.len() + agg_fields.len());

    for (key, value) in group_fields {
        if !seen.insert(key.clone()) {
            return Err(CtDiagnosticError::simple(format!(
                "agg: duplicate grouped key `{key}` in grouped input"
            )));
        }
        merged.push((key, value));
    }

    for (key, value) in agg_fields {
        if !seen.insert(key.clone()) {
            return Err(CtDiagnosticError::simple(format!(
                "agg: aggregation output field `{key}` conflicts with grouped key; use alias to rename it"
            )));
        }
        merged.push((key, value));
    }

    Ok(merged)
}

fn aggregate_sum(rows: &[CtValue], field: &str) -> Result<CtValue, CtDiagnosticError> {
    let values = collect_field_values(rows, field, "sum")?;
    if values.is_empty() {
        return Ok(CtValue::Nothing);
    }

    let mut int_sum: i128 = 0;
    let mut float_sum = 0.0f64;
    let mut seen_float = false;

    for value in values {
        match value {
            CtValue::Int(n) => {
                if seen_float {
                    float_sum += *n as f64;
                } else {
                    int_sum += *n as i128;
                }
            }
            CtValue::Float(n) => {
                if !seen_float {
                    float_sum = int_sum as f64;
                    seen_float = true;
                }
                float_sum += *n;
            }
            _ => {
                return Err(CtDiagnosticError::simple(format!(
                    "agg: sum expects numeric field `{field}`"
                )));
            }
        }
    }

    if seen_float {
        Ok(CtValue::Float(float_sum))
    } else if int_sum > i64::MAX as i128 || int_sum < i64::MIN as i128 {
        Ok(CtValue::Float(int_sum as f64))
    } else {
        Ok(CtValue::Int(int_sum as i64))
    }
}

fn aggregate_avg(rows: &[CtValue], field: &str) -> Result<CtValue, CtDiagnosticError> {
    let values = collect_field_values(rows, field, "avg")?;
    if values.is_empty() {
        return Ok(CtValue::Nothing);
    }

    let mut sum = 0.0f64;
    let mut count = 0usize;
    for value in values {
        match value {
            CtValue::Int(n) => {
                sum += *n as f64;
                count += 1;
            }
            CtValue::Float(n) => {
                sum += *n;
                count += 1;
            }
            _ => {
                return Err(CtDiagnosticError::simple(format!(
                    "agg: avg expects numeric field `{field}`"
                )));
            }
        }
    }
    Ok(CtValue::Float(sum / (count as f64)))
}

fn aggregate_minmax(
    rows: &[CtValue],
    field: &str,
    min_mode: bool,
) -> Result<CtValue, CtDiagnosticError> {
    let values = collect_field_values(rows, field, if min_mode { "min" } else { "max" })?;
    let mut iter = values.into_iter();
    let Some(first) = iter.next() else {
        return Ok(CtValue::Nothing);
    };
    let mut best = first.clone();

    for value in iter {
        let ord = compare_for_minmax(&best, value, field)?;
        let should_replace = if min_mode {
            ord == Ordering::Greater
        } else {
            ord == Ordering::Less
        };
        if should_replace {
            best = value.clone();
        }
    }

    Ok(best)
}

fn collect_field_values<'a>(
    rows: &'a [CtValue],
    field: &str,
    func: &str,
) -> Result<Vec<&'a CtValue>, CtDiagnosticError> {
    let mut values = Vec::new();
    for item in rows {
        let CtValue::Record(fields) = item else {
            return Err(CtDiagnosticError::simple(format!(
                "agg: {func} expects List<Record> input"
            )));
        };
        if let Some(value) = resolve_field_path(fields, field)
            && !matches!(value, CtValue::Nothing)
        {
            values.push(value);
        }
    }
    Ok(values)
}

fn compare_for_minmax(
    left: &CtValue,
    right: &CtValue,
    field: &str,
) -> Result<Ordering, CtDiagnosticError> {
    match (left, right) {
        (CtValue::Bool(a), CtValue::Bool(b)) => Ok(a.cmp(b)),
        (CtValue::Int(a), CtValue::Float(b)) => (*a as f64).partial_cmp(b).ok_or_else(|| {
            CtDiagnosticError::simple(format!("agg: cannot compare field `{field}`"))
        }),
        (CtValue::Float(a), CtValue::Int(b)) => a.partial_cmp(&(*b as f64)).ok_or_else(|| {
            CtDiagnosticError::simple(format!("agg: cannot compare field `{field}`"))
        }),
        _ => ct_cmp(left, right).ok_or_else(|| {
            CtDiagnosticError::simple(format!(
                "agg: incompatible value types for field `{field}` in min/max"
            ))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn call(specs: &[&str]) -> DataCall {
        let mut c = DataCall::named("agg");
        for spec in specs {
            c.positionals.push(ctsig::BoundArg::new(
                CtValue::String((*spec).to_string()),
                None,
            ));
        }
        c
    }

    fn list(records: Vec<Vec<(&str, CtValue)>>) -> CtPipelineData {
        let items = records
            .into_iter()
            .map(|fields| {
                CtValue::Record(
                    fields
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        CtPipelineData::Value(CtValue::List(items), CtPipelineMetadata::default())
    }

    fn get_field<'a>(fields: &'a [(String, CtValue)], name: &str) -> &'a CtValue {
        &fields
            .iter()
            .find(|(k, _)| k == name)
            .expect("field exists")
            .1
    }

    #[test]
    fn test_agg_signature_allows_multiple_ops() {
        let sig = CmdAgg.signature();
        assert!(sig.rest_positional_arg().is_some());
    }

    #[test]
    fn test_agg_count_sum_avg() {
        let data = list(vec![
            vec![("size", CtValue::Int(1))],
            vec![("size", CtValue::Int(2))],
            vec![("size", CtValue::Int(3))],
        ]);
        let out = CmdAgg
            .run(&call(&["count", "sum:size", "avg:size"]), data, &ctx())
            .unwrap();

        let CtPipelineData::Value(CtValue::Record(fields), _) = out else {
            panic!("record output");
        };
        assert!(matches!(get_field(&fields, "count"), CtValue::Int(3)));
        assert!(matches!(get_field(&fields, "sum_size"), CtValue::Int(6)));
        match get_field(&fields, "avg_size") {
            CtValue::Float(v) => assert!((*v - 2.0).abs() < 1e-9),
            _ => panic!("avg must be float"),
        }
    }

    #[test]
    fn test_agg_grouped_input() {
        let grouped = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![
                    (
                        "group".into(),
                        CtValue::Record(vec![("team".into(), CtValue::String("a".into()))]),
                    ),
                    (
                        "rows".into(),
                        CtValue::List(vec![
                            CtValue::Record(vec![("score".into(), CtValue::Int(1))]),
                            CtValue::Record(vec![("score".into(), CtValue::Int(3))]),
                        ]),
                    ),
                ]),
                CtValue::Record(vec![
                    (
                        "group".into(),
                        CtValue::Record(vec![("team".into(), CtValue::String("b".into()))]),
                    ),
                    (
                        "rows".into(),
                        CtValue::List(vec![CtValue::Record(vec![(
                            "score".into(),
                            CtValue::Int(2),
                        )])]),
                    ),
                ]),
            ]),
            CtPipelineMetadata::default(),
        );

        let out = CmdAgg
            .run(&call(&["count", "max:score"]), grouped, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = out else {
            panic!("list output");
        };
        assert_eq!(items.len(), 2);
        let CtValue::Record(first) = &items[0] else {
            panic!("record");
        };
        assert!(matches!(get_field(first, "team"), CtValue::String(v) if v == "a"));
        assert!(matches!(get_field(first, "count"), CtValue::Int(2)));
        assert!(matches!(get_field(first, "max_score"), CtValue::Int(3)));
    }

    #[test]
    fn test_agg_alias_supported() {
        let data = list(vec![
            vec![("bytes", CtValue::Int(10))],
            vec![("bytes", CtValue::Int(5))],
        ]);
        let out = CmdAgg
            .run(&call(&["sum:bytes=total_bytes"]), data, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::Record(fields), _) = out else {
            panic!("record");
        };
        assert!(matches!(
            get_field(&fields, "total_bytes"),
            CtValue::Int(15)
        ));
    }

    #[test]
    fn test_agg_sum_non_numeric_fails() {
        let data = list(vec![vec![("bytes", CtValue::String("x".into()))]]);
        let err = CmdAgg.run(&call(&["sum:bytes"]), data, &ctx()).unwrap_err();
        assert!(err.to_string().contains("sum expects numeric"));
    }

    #[test]
    fn test_agg_count_non_record_list() {
        let data = CtPipelineData::Value(
            CtValue::List(vec![CtValue::Int(1), CtValue::Int(2), CtValue::Int(3)]),
            CtPipelineMetadata::default(),
        );
        let out = CmdAgg.run(&call(&["count"]), data, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::Record(fields), _) = out else {
            panic!("record");
        };
        assert!(matches!(get_field(&fields, "count"), CtValue::Int(3)));
    }

    #[test]
    fn test_agg_empty_list() {
        let data = CtPipelineData::Value(CtValue::List(vec![]), CtPipelineMetadata::default());
        let out = CmdAgg
            .run(&call(&["count", "sum:value"]), data, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::Record(fields), _) = out else {
            panic!("record");
        };
        assert!(matches!(get_field(&fields, "count"), CtValue::Int(0)));
        assert!(matches!(get_field(&fields, "sum_value"), CtValue::Nothing));
    }

    #[test]
    fn test_agg_grouped_alias_conflict_fails() {
        let grouped = CtPipelineData::Value(
            CtValue::List(vec![CtValue::Record(vec![
                (
                    "group".into(),
                    CtValue::Record(vec![("count".into(), CtValue::String("x".into()))]),
                ),
                (
                    "rows".into(),
                    CtValue::List(vec![CtValue::Record(vec![(
                        "score".into(),
                        CtValue::Int(1),
                    )])]),
                ),
            ])]),
            CtPipelineMetadata::default(),
        );

        let err = CmdAgg.run(&call(&["count"]), grouped, &ctx()).unwrap_err();
        assert!(err.to_string().contains("conflicts with grouped key"));
    }

    #[test]
    fn test_agg_duplicate_output_alias_fails() {
        let data = list(vec![
            vec![("score", CtValue::Int(1))],
            vec![("score", CtValue::Int(2))],
        ]);

        let err = CmdAgg
            .run(&call(&["count", "sum:score=count"]), data, &ctx())
            .unwrap_err();
        assert!(err.to_string().contains("duplicate output field `count`"));
    }
}
