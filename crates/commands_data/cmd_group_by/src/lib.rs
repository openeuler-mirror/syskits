/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_group_by` - 对 List<Record> 按一个或多个字段分组。

use ctengine::command::DataCommand;
use ctengine::compare::resolve_field_path;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::collections::HashMap;

#[derive(Default)]
pub struct CmdGroupBy;

const GROUP_BY_HELP: &str = r#"syskits data group-by

This is the syskits structured data pipeline group-by command.
It groups List<Record> input by one or more field paths.

Usage:
  group-by <key> [keys...]
  group-by --help
  group-by --version

Output:
  List<Record{group: Record, rows: List<Record>}>

Examples:
  ps | group-by status
  ps | group-by status name
"#;

impl DataCommand for CmdGroupBy {
    fn signature(&self) -> DataSignature {
        DataSignature::new("group-by", "group List<Record> by one or more field paths")
            .positional(CtPositionalArg::required(
                "keys",
                "field path(s), e.g. region meta.zone",
                CtType::String,
            ))
            .rest(CtPositionalArg::optional(
                "keys",
                "additional field paths to group by",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "help",
                Some('h'),
                "show help for syskits data group-by",
            ))
            .flag(CtFlag::switch(
                "version",
                None,
                "show syskits data group-by version",
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
            return Ok(meta_text_output(GROUP_BY_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data group-by {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        let keys = call
            .rest::<String>(0)
            .map_err(|e| CtDiagnosticError::simple(format!("group-by: {e}")))?;
        if keys.is_empty() {
            return Err(CtDiagnosticError::simple(
                "group-by: at least one key is required",
            ));
        }

        let items = match input {
            CtPipelineData::Value(CtValue::List(items), _) => items,
            CtPipelineData::ListStream(stream) => stream.collect(),
            CtPipelineData::Empty => return Ok(CtPipelineData::Empty),
            _ => {
                return Err(CtDiagnosticError::simple(
                    "group-by: expected List<Record> input",
                ));
            }
        };

        if !items.iter().all(|v| matches!(v, CtValue::Record(_))) {
            return Err(CtDiagnosticError::simple(
                "group-by: all list items must be Record",
            ));
        }

        let mut index_by_key = HashMap::<String, usize>::new();
        let mut buckets = Vec::<GroupBucket>::new();

        for item in items {
            let CtValue::Record(fields) = &item else {
                continue;
            };
            let group_fields = build_group_fields(fields, &keys);
            let group_key = build_bucket_key(&group_fields);

            if let Some(index) = index_by_key.get(&group_key).copied() {
                buckets[index].rows.push(item);
            } else {
                let index = buckets.len();
                index_by_key.insert(group_key, index);
                buckets.push(GroupBucket {
                    group_fields,
                    rows: vec![item],
                });
            }
        }

        let output = buckets
            .into_iter()
            .map(|bucket| {
                CtValue::Record(vec![
                    ("group".to_string(), CtValue::Record(bucket.group_fields)),
                    ("rows".to_string(), CtValue::List(bucket.rows)),
                ])
            })
            .collect::<Vec<_>>();

        Ok(CtPipelineData::Value(
            CtValue::List(output),
            CtPipelineMetadata::default(),
        ))
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
            source: Some("group-by".into()),
            ..Default::default()
        },
    )
}

struct GroupBucket {
    group_fields: Vec<(String, CtValue)>,
    rows: Vec<CtValue>,
}

fn build_group_fields(fields: &[(String, CtValue)], keys: &[String]) -> Vec<(String, CtValue)> {
    keys.iter()
        .map(|path| {
            let label = normalize_key_label(path);
            let value = resolve_field_path(fields, path)
                .cloned()
                .unwrap_or(CtValue::Nothing);
            (label, value)
        })
        .collect()
}

fn normalize_key_label(path: &str) -> String {
    path.trim()
        .trim_start_matches("$it.")
        .trim_start_matches("it.")
        .replace('.', "_")
}

fn build_bucket_key(fields: &[(String, CtValue)]) -> String {
    let mut key = String::new();
    for (idx, (name, value)) in fields.iter().enumerate() {
        if idx > 0 {
            key.push('|');
        }
        key.push_str(name);
        key.push('=');
        key.push_str(&value_key_repr(value));
    }
    key
}

fn value_key_repr(value: &CtValue) -> String {
    match value {
        CtValue::Nothing => "null".to_string(),
        CtValue::Bool(v) => format!("bool:{v}"),
        CtValue::Int(v) => format!("int:{v}"),
        CtValue::Float(v) => format!("float:{v:?}"),
        CtValue::String(v) => format!("string:{v:?}"),
        CtValue::Binary(v) => format!("binary:{v:?}"),
        CtValue::DateTime(v) => format!("datetime:{v}"),
        CtValue::Duration(v) => format!("duration:{v}"),
        CtValue::Size(v) => format!("size:{v}"),
        CtValue::Record(fields) => {
            let mut out = String::from("record:{");
            for (idx, (k, v)) in fields.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push_str(k);
                out.push(':');
                out.push_str(&value_key_repr(v));
            }
            out.push('}');
            out
        }
        CtValue::List(items) => {
            let mut out = String::from("list:[");
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push_str(&value_key_repr(item));
            }
            out.push(']');
            out
        }
        CtValue::Error(err) => format!("error:{err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn call(keys: &[&str]) -> DataCall {
        let mut c = DataCall::named("group-by");
        for key in keys {
            c.positionals.push(ctsig::BoundArg::new(
                CtValue::String((*key).to_string()),
                None,
            ));
        }
        c
    }

    fn input(records: Vec<Vec<(&str, CtValue)>>) -> CtPipelineData {
        let vals = records
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
        CtPipelineData::Value(CtValue::List(vals), CtPipelineMetadata::default())
    }

    fn find_field<'a>(fields: &'a [(String, CtValue)], name: &str) -> &'a CtValue {
        &fields
            .iter()
            .find(|(k, _)| k == name)
            .expect("field exists")
            .1
    }

    #[test]
    fn test_group_by_signature_allows_multiple_keys() {
        let sig = CmdGroupBy.signature();
        assert!(sig.rest_positional_arg().is_some());
    }

    #[test]
    fn test_group_by_single_key() {
        let data = input(vec![
            vec![
                ("region", CtValue::String("cn".into())),
                ("id", CtValue::Int(1)),
            ],
            vec![
                ("region", CtValue::String("us".into())),
                ("id", CtValue::Int(2)),
            ],
            vec![
                ("region", CtValue::String("cn".into())),
                ("id", CtValue::Int(3)),
            ],
        ]);
        let out = CmdGroupBy.run(&call(&["region"]), data, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::List(groups), _) = out else {
            panic!("expected list");
        };
        assert_eq!(groups.len(), 2);

        let CtValue::Record(first_group) = &groups[0] else {
            panic!("group record");
        };
        let CtValue::Record(group_fields) = find_field(first_group, "group") else {
            panic!("group field");
        };
        let CtValue::List(rows) = find_field(first_group, "rows") else {
            panic!("rows field");
        };
        let CtValue::String(region) = find_field(group_fields, "region") else {
            panic!("region");
        };
        assert_eq!(region, "cn");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_group_by_nested_path_uses_flattened_label() {
        let data = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![(
                    "meta".into(),
                    CtValue::Record(vec![("zone".into(), CtValue::String("a".into()))]),
                )]),
                CtValue::Record(vec![(
                    "meta".into(),
                    CtValue::Record(vec![("zone".into(), CtValue::String("b".into()))]),
                )]),
            ]),
            CtPipelineMetadata::default(),
        );
        let out = CmdGroupBy.run(&call(&["meta.zone"]), data, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::List(groups), _) = out else {
            panic!("expected list");
        };
        let CtValue::Record(group0) = &groups[0] else {
            panic!("group");
        };
        let CtValue::Record(group_fields) = find_field(group0, "group") else {
            panic!("group fields");
        };
        assert!(group_fields.iter().any(|(k, _)| k == "meta_zone"));
    }

    #[test]
    fn test_group_by_missing_field_becomes_nothing() {
        let data = input(vec![
            vec![("id", CtValue::Int(1))],
            vec![("id", CtValue::Int(2))],
        ]);
        let out = CmdGroupBy.run(&call(&["region"]), data, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::List(groups), _) = out else {
            panic!("expected list");
        };
        assert_eq!(groups.len(), 1);
        let CtValue::Record(group) = &groups[0] else {
            panic!("group");
        };
        let CtValue::Record(group_fields) = find_field(group, "group") else {
            panic!("group fields");
        };
        assert!(matches!(
            find_field(group_fields, "region"),
            CtValue::Nothing
        ));
    }

    #[test]
    fn test_group_by_non_record_input_fails() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![CtValue::Int(1), CtValue::Int(2)]),
            CtPipelineMetadata::default(),
        );
        let err = CmdGroupBy.run(&call(&["id"]), input, &ctx()).unwrap_err();
        assert!(err.to_string().contains("all list items must be Record"));
    }
}
