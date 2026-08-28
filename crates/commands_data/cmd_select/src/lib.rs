/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_select` — 从 Record / List<Record> 投影字段。

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};

#[derive(Default)]
pub struct CmdSelect;

impl DataCommand for CmdSelect {
    fn signature(&self) -> DataSignature {
        DataSignature::new("select", "project columns from a Record or List<Record>")
            .positional(CtPositionalArg::required(
                "columns",
                "column names to keep",
                CtType::String,
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

fn project(fields: Vec<(String, CtValue)>, cols: &[String]) -> Vec<(String, CtValue)> {
    cols.iter()
        .filter_map(|col| {
            fields
                .iter()
                .find(|(k, _)| k == col)
                .map(|(k, v)| (k.clone(), v.clone()))
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
}
