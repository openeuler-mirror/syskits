/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_sort_by` — 对 List<Record> 按一个或多个字段排序。

use ctengine::command::DataCommand;
use ctengine::compare::{ct_cmp, resolve_field_path};
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::cmp::Ordering;

#[derive(Default)]
pub struct CmdSortBy;

impl DataCommand for CmdSortBy {
    fn signature(&self) -> DataSignature {
        DataSignature::new("sort-by", "sort List<Record> by one or more field paths")
            .positional(CtPositionalArg::required(
                "keys",
                "field path(s), e.g. age metadata.name",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "desc",
                Some('r'),
                "sort in descending order",
            ))
            .flag(CtFlag::switch(
                "nulls-last",
                None,
                "place null/missing values at the end",
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
        let keys: Vec<String> = call
            .rest::<String>(0)
            .map_err(|e| CtDiagnosticError::simple(format!("sort-by: {e}")))?;
        if keys.is_empty() {
            return Err(CtDiagnosticError::simple(
                "sort-by: at least one key is required",
            ));
        }

        let desc = call.has_flag("desc") || call.has_flag("r");
        let nulls_last = call.has_flag("nulls-last");

        let mut items = match input {
            CtPipelineData::Value(CtValue::List(items), _) => items,
            CtPipelineData::ListStream(stream) => stream.collect(),
            CtPipelineData::Empty => return Ok(CtPipelineData::Empty),
            _ => {
                return Err(CtDiagnosticError::simple(
                    "sort-by: expected List<Record> input",
                ));
            }
        };

        if !items.iter().all(|v| matches!(v, CtValue::Record(_))) {
            return Err(CtDiagnosticError::simple(
                "sort-by: all list items must be Record",
            ));
        }

        items.sort_by(|a, b| compare_records(a, b, &keys, desc, nulls_last));

        Ok(CtPipelineData::Value(
            CtValue::List(items),
            CtPipelineMetadata::default(),
        ))
    }
}

fn compare_records(
    left: &CtValue,
    right: &CtValue,
    keys: &[String],
    desc: bool,
    nulls_last: bool,
) -> Ordering {
    let CtValue::Record(left_fields) = left else {
        return Ordering::Equal;
    };
    let CtValue::Record(right_fields) = right else {
        return Ordering::Equal;
    };

    for key in keys {
        let l = resolve_field_path(left_fields, key);
        let r = resolve_field_path(right_fields, key);
        let ord = compare_nullable_values(l, r, nulls_last, desc);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn compare_nullable_values(
    left: Option<&CtValue>,
    right: Option<&CtValue>,
    nulls_last: bool,
    desc: bool,
) -> Ordering {
    let lnull = left.is_none_or(is_null_like);
    let rnull = right.is_none_or(is_null_like);

    match (lnull, rnull) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if nulls_last {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (false, true) => {
            if nulls_last {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, false) => {
            let Some(lv) = left else {
                return Ordering::Equal;
            };
            let Some(rv) = right else {
                return Ordering::Equal;
            };
            let ord = compare_values(lv, rv).unwrap_or(Ordering::Equal);
            if desc { ord.reverse() } else { ord }
        }
    }
}

fn is_null_like(v: &CtValue) -> bool {
    matches!(v, CtValue::Nothing)
}

fn compare_values(left: &CtValue, right: &CtValue) -> Option<Ordering> {
    match (left, right) {
        (CtValue::Bool(a), CtValue::Bool(b)) => Some(a.cmp(b)),
        _ => ct_cmp(left, right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn call(keys: &[&str], desc: bool, nulls_last: bool) -> DataCall {
        let mut c = DataCall::named("sort-by");
        for key in keys {
            c.positionals.push(ctsig::BoundArg::new(
                CtValue::String((*key).to_string()),
                None,
            ));
        }
        if desc {
            c.flags.insert("desc".to_string(), None);
        }
        if nulls_last {
            c.flags.insert("nulls-last".to_string(), None);
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

    fn field_int(rec: &CtValue, key: &str) -> i64 {
        let CtValue::Record(fields) = rec else {
            panic!("expected record");
        };
        let v = fields.iter().find(|(k, _)| k == key).expect("key exists");
        let CtValue::Int(n) = &v.1 else {
            panic!("expected int");
        };
        *n
    }

    #[test]
    fn test_sort_by_asc() {
        let data = input(vec![
            vec![
                ("name", CtValue::String("b".into())),
                ("age", CtValue::Int(3)),
            ],
            vec![
                ("name", CtValue::String("a".into())),
                ("age", CtValue::Int(1)),
            ],
            vec![
                ("name", CtValue::String("c".into())),
                ("age", CtValue::Int(2)),
            ],
        ]);
        let out = CmdSortBy
            .run(&call(&["age"], false, false), data, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = out else {
            panic!("expected list");
        };
        assert_eq!(field_int(&items[0], "age"), 1);
        assert_eq!(field_int(&items[1], "age"), 2);
        assert_eq!(field_int(&items[2], "age"), 3);
    }

    #[test]
    fn test_sort_by_desc() {
        let data = input(vec![
            vec![("age", CtValue::Int(1))],
            vec![("age", CtValue::Int(3))],
            vec![("age", CtValue::Int(2))],
        ]);
        let out = CmdSortBy
            .run(&call(&["age"], true, false), data, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = out else {
            panic!("expected list");
        };
        assert_eq!(field_int(&items[0], "age"), 3);
        assert_eq!(field_int(&items[2], "age"), 1);
    }

    #[test]
    fn test_sort_by_nested_path() {
        let data = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![(
                    "meta".into(),
                    CtValue::Record(vec![("score".into(), CtValue::Int(9))]),
                )]),
                CtValue::Record(vec![(
                    "meta".into(),
                    CtValue::Record(vec![("score".into(), CtValue::Int(1))]),
                )]),
            ]),
            CtPipelineMetadata::default(),
        );
        let out = CmdSortBy
            .run(&call(&["meta.score"], false, false), data, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = out else {
            panic!("expected list");
        };
        let CtValue::Record(r0) = &items[0] else {
            panic!("record");
        };
        let Some((_, CtValue::Record(inner))) = r0.iter().find(|(k, _)| k == "meta") else {
            panic!("meta");
        };
        let Some((_, CtValue::Int(score))) = inner.iter().find(|(k, _)| k == "score") else {
            panic!("score");
        };
        assert_eq!(*score, 1);
    }

    #[test]
    fn test_sort_by_nulls_last() {
        let data = input(vec![
            vec![("age", CtValue::Nothing)],
            vec![("age", CtValue::Int(2))],
            vec![("age", CtValue::Int(1))],
        ]);
        let out = CmdSortBy
            .run(&call(&["age"], false, true), data, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = out else {
            panic!("expected list");
        };
        assert_eq!(field_int(&items[0], "age"), 1);
        assert_eq!(field_int(&items[1], "age"), 2);
        let CtValue::Record(fields) = &items[2] else {
            panic!("record");
        };
        let Some((_, CtValue::Nothing)) = fields.iter().find(|(k, _)| k == "age") else {
            panic!("nothing");
        };
    }

    #[test]
    fn test_sort_by_non_record_input_fails() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![CtValue::Int(1), CtValue::Int(2)]),
            CtPipelineMetadata::default(),
        );
        let err = CmdSortBy
            .run(&call(&["age"], false, false), input, &ctx())
            .unwrap_err();
        assert!(err.to_string().contains("all list items must be Record"));
    }

    #[test]
    fn test_sort_by_desc_with_nulls_last_keeps_null_at_end() {
        let data = input(vec![
            vec![("age", CtValue::Nothing)],
            vec![("age", CtValue::Int(1))],
            vec![("age", CtValue::Int(3))],
            vec![("age", CtValue::Int(2))],
        ]);
        let out = CmdSortBy
            .run(&call(&["age"], true, true), data, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = out else {
            panic!("expected list");
        };
        assert_eq!(field_int(&items[0], "age"), 3);
        assert_eq!(field_int(&items[1], "age"), 2);
        assert_eq!(field_int(&items[2], "age"), 1);
        let CtValue::Record(fields) = &items[3] else {
            panic!("record");
        };
        let Some((_, CtValue::Nothing)) = fields.iter().find(|(k, _)| k == "age") else {
            panic!("nothing");
        };
    }
}
