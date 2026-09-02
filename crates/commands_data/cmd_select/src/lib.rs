/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_select` — 从 Record / List<Record> 投影字段。

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::collections::HashMap;

#[derive(Default)]
pub struct CmdSelect;

const SELECT_HELP: &str = r#"syskits data select

This is the syskits structured data pipeline select command.
It projects fields from a Record or each Record in a List<Record>.

Usage:
  select <column> [columns...]
  select --help
  select --version

Examples:
  ps | select pid name cpu
  whoami | select username
"#;

impl DataCommand for CmdSelect {
    fn signature(&self) -> DataSignature {
        DataSignature::new("select", "project columns from a Record or List<Record>")
            .positional(CtPositionalArg::required(
                "columns",
                "column names to keep",
                CtType::String,
            ))
            .rest(CtPositionalArg::optional(
                "columns",
                "additional column names to keep",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "help",
                Some('h'),
                "show help for syskits data select",
            ))
            .flag(CtFlag::switch(
                "version",
                None,
                "show syskits data select version",
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
            return Ok(meta_text_output(SELECT_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data select {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        let cols: Vec<String> = call
            .rest::<String>(0)
            .map_err(|e| CtDiagnosticError::simple(format!("select: {e}")))?;
        if cols.is_empty() {
            return Err(CtDiagnosticError::simple(
                "select: at least one column name required",
            ));
        }
        let meta = CtPipelineMetadata::default();

        match input {
            CtPipelineData::Value(CtValue::Record(fields), _) => Ok(CtPipelineData::Value(
                CtValue::Record(project(fields, &cols)),
                meta,
            )),
            CtPipelineData::Value(CtValue::List(items), _) => {
                let projected: Vec<CtValue> = items
                    .into_iter()
                    .map(|item| {
                        if let CtValue::Record(fields) = item {
                            CtValue::Record(project(fields, &cols))
                        } else {
                            item
                        }
                    })
                    .collect();
                Ok(CtPipelineData::Value(CtValue::List(projected), meta))
            }
            CtPipelineData::Empty => Err(CtDiagnosticError::simple("select: empty input")),
            _ => Err(CtDiagnosticError::simple("select: expected Record or List")),
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
            source: Some("select".into()),
            ..Default::default()
        },
    )
}

fn project(fields: Vec<(String, CtValue)>, cols: &[String]) -> Vec<(String, CtValue)> {
    let mut field_index = HashMap::with_capacity(fields.len());
    for (idx, (key, _)) in fields.iter().enumerate() {
        field_index.entry(key.as_str()).or_insert(idx);
    }

    cols.iter()
        .filter_map(|col| {
            field_index.get(col.as_str()).map(|idx| {
                let (key, value) = &fields[*idx];
                (key.clone(), value.clone())
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn cols_call(cols: &[&str]) -> DataCall {
        let mut c = DataCall::empty();
        for col in cols {
            c.positionals
                .push(ctsig::BoundArg::new(CtValue::String(col.to_string()), None));
        }
        c
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
    fn test_select_single() {
        let r = CmdSelect
            .run(
                &cols_call(&["name"]),
                rec(vec![
                    ("name", CtValue::String("Alice".into())),
                    ("age", CtValue::Int(30)),
                ]),
                &ctx(),
            )
            .unwrap();
        if let CtPipelineData::Value(CtValue::Record(f), _) = r {
            assert_eq!(f.len(), 1);
        } else {
            panic!("expected Record");
        }
    }

    #[test]
    fn test_select_signature_allows_multiple_columns() {
        let sig = CmdSelect.signature();
        assert!(sig.rest_positional_arg().is_some());
    }

    #[test]
    fn test_select_multiple() {
        let r = CmdSelect
            .run(
                &cols_call(&["a", "c"]),
                rec(vec![
                    ("a", CtValue::Int(1)),
                    ("b", CtValue::Int(2)),
                    ("c", CtValue::Int(3)),
                ]),
                &ctx(),
            )
            .unwrap();
        if let CtPipelineData::Value(CtValue::Record(f), _) = r {
            assert_eq!(f.len(), 2);
            assert_eq!(f[0].0, "a");
            assert_eq!(f[1].0, "c");
        } else {
            panic!("expected Record");
        }
    }
    #[test]
    fn test_select_list() {
        let list = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("x".into(), CtValue::Int(1)),
                    ("y".into(), CtValue::Int(2)),
                ]),
                CtValue::Record(vec![
                    ("x".into(), CtValue::Int(3)),
                    ("y".into(), CtValue::Int(4)),
                ]),
            ]),
            CtPipelineMetadata::default(),
        );
        let r = CmdSelect.run(&cols_call(&["x"]), list, &ctx()).unwrap();
        if let CtPipelineData::Value(CtValue::List(items), _) = r {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected List");
        }
    }
    #[test]
    fn test_select_empty_cols() {
        let e = CmdSelect
            .run(
                &DataCall::empty(),
                rec(vec![("a", CtValue::Int(1))]),
                &ctx(),
            )
            .unwrap_err();
        assert!(e.to_string().contains("at least one column"));
    }
    #[test]
    fn test_select_missing_col_skipped() {
        let r = CmdSelect
            .run(
                &cols_call(&["name", "missing"]),
                rec(vec![("name", CtValue::String("Bob".into()))]),
                &ctx(),
            )
            .unwrap();
        if let CtPipelineData::Value(CtValue::Record(f), _) = r {
            assert_eq!(f.len(), 1);
        } else {
            panic!("expected Record");
        }
    }

    #[test]
    fn test_select_project_preserves_order_duplicates_missing_and_first_wins() {
        let r = CmdSelect
            .run(
                &cols_call(&["c", "missing", "a", "a"]),
                rec(vec![
                    ("a", CtValue::Int(1)),
                    ("b", CtValue::Int(2)),
                    ("a", CtValue::Int(99)),
                    ("c", CtValue::Int(3)),
                ]),
                &ctx(),
            )
            .unwrap();

        let CtPipelineData::Value(CtValue::Record(f), _) = r else {
            panic!("expected Record");
        };
        assert_eq!(
            f,
            vec![
                ("c".to_string(), CtValue::Int(3)),
                ("a".to_string(), CtValue::Int(1)),
                ("a".to_string(), CtValue::Int(1)),
            ]
        );
    }
}
