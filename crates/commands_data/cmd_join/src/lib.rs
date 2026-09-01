use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdJoin;

struct JoinIntent {
    argv: Vec<OsString>,
}

struct JoinCore;

impl JoinIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("join"));

        for arg in &call.positionals {
            argv.push(OsString::from(value_to_arg(&arg.value)));
        }

        Ok(Self { argv })
    }
}

fn value_to_arg(value: &CtValue) -> String {
    match value {
        CtValue::String(s) => s.clone(),
        other => other.to_text(),
    }
}

impl JoinCore {
    fn run_core(
        intent: &JoinIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "join",
            input,
            ctengine::argv_has_stdin_operand(
                &intent.argv,
                &["-1", "-2", "-j", "-o", "-t", "-a", "-v", "-e"],
            ),
            || Ok(ct_join::join_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("join: {err}")),
        )?;
        Ok(match result {
            Ok(semantic) => (
                semantic_to_value(&semantic),
                semantic.classic_text,
                semantic.stderr_text,
                semantic.exit_code,
            ),
            Err(err) => (
                CtValue::List(Vec::new()),
                String::new(),
                render_error_text(err.as_ref()),
                err.code(),
            ),
        })
    }
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("join: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'join --help' for more information.\n");
    }
    stderr
}

fn opt_string_to_value(value: Option<&str>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.into()),
        None => CtValue::Nothing,
    }
}

fn string_list_to_value(values: &[String]) -> CtValue {
    CtValue::List(values.iter().cloned().map(CtValue::String).collect())
}

fn row_to_value(semantic: &ct_join::JoinSemantic, row: &ct_join::JoinRow) -> CtValue {
    CtValue::Record(vec![
        (
            "key1".into(),
            CtValue::Int(i64::try_from(semantic.key1).expect("key1 fits")),
        ),
        (
            "key2".into(),
            CtValue::Int(i64::try_from(semantic.key2).expect("key2 fits")),
        ),
        ("print_joined".into(), CtValue::Bool(semantic.print_joined)),
        (
            "print_unpaired1".into(),
            CtValue::Bool(semantic.print_unpaired1),
        ),
        (
            "print_unpaired2".into(),
            CtValue::Bool(semantic.print_unpaired2),
        ),
        ("ignore_case".into(), CtValue::Bool(semantic.ignore_case)),
        (
            "zero_terminated".into(),
            CtValue::Bool(semantic.zero_terminated),
        ),
        ("headers".into(), CtValue::Bool(semantic.headers)),
        ("autoformat".into(), CtValue::Bool(semantic.autoformat)),
        (
            "check_order".into(),
            CtValue::String(semantic.check_order.clone()),
        ),
        (
            "separator_mode".into(),
            CtValue::String(semantic.separator_mode.clone()),
        ),
        (
            "separator_text".into(),
            CtValue::String(semantic.separator_text.clone()),
        ),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("row_kind".into(), CtValue::String(row.row_kind.clone())),
        (
            "join_key".into(),
            opt_string_to_value(row.join_key.as_deref()),
        ),
        (
            "source_file_num".into(),
            match row.source_file_num {
                Some(value) => CtValue::Int(i64::try_from(value).expect("source file num fits")),
                None => CtValue::Nothing,
            },
        ),
        (
            "file1_fields".into(),
            string_list_to_value(&row.file1_fields),
        ),
        (
            "file2_fields".into(),
            string_list_to_value(&row.file2_fields),
        ),
        (
            "output_line".into(),
            CtValue::String(row.output_line.clone()),
        ),
    ])
}

fn semantic_to_value(semantic: &ct_join::JoinSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

fn display_columns() -> CtValue {
    CtValue::List(
        [
            "row_index",
            "row_kind",
            "join_key",
            "file1_fields",
            "file2_fields",
            "output_line",
        ]
        .into_iter()
        .map(|column| CtValue::String(column.into()))
        .collect(),
    )
}

impl DataCommand for CmdJoin {
    fn signature(&self) -> DataSignature {
        DataSignature::new("join", "structured join output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible join arguments",
                CtType::Any,
            ))
            .input(CtType::Any)
            .output(CtType::List)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = JoinIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = JoinCore::run_core(&intent, input)?;
        let metadata = CtPipelineMetadata {
            classic_text: Some(classic_text),
            classic_bytes: None,
            classic_append_newline: false,
            stderr_text: if stderr_text.is_empty() {
                None
            } else {
                Some(stderr_text)
            },
            exit_code,
            source: Some("join".into()),
            ..Default::default()
        };
        if let Ok(mut custom) = metadata.custom.lock() {
            custom.insert("display.columns".into(), display_columns());
        }
        Ok(CtPipelineData::Value(value, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::{JoinIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-a".into()), None),
                BoundArg::new(CtValue::String("1".into()), None),
                BoundArg::new(CtValue::String("left.txt".into()), None),
                BoundArg::new(CtValue::String("right.txt".into()), None),
            ],
            ..DataCall::named("join")
        };

        let intent = JoinIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("join"),
                OsString::from("-a"),
                OsString::from("1"),
                OsString::from("left.txt"),
                OsString::from("right.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_join::JoinSemantic {
            key1: 1,
            key2: 1,
            print_joined: true,
            print_unpaired1: false,
            print_unpaired2: false,
            ignore_case: false,
            zero_terminated: false,
            headers: false,
            autoformat: false,
            check_order: "default".into(),
            separator_mode: "whitespaces".into(),
            separator_text: " ".into(),
            rows: vec![
                ct_join::JoinRow {
                    row_index: 1,
                    row_kind: "joined".into(),
                    join_key: Some("1".into()),
                    source_file_num: None,
                    file1_fields: vec!["1".into(), "alpha".into()],
                    file2_fields: vec!["1".into(), "uno".into()],
                    output_line: "1 alpha uno".into(),
                },
                ct_join::JoinRow {
                    row_index: 2,
                    row_kind: "unpaired".into(),
                    join_key: Some("3".into()),
                    source_file_num: Some(1),
                    file1_fields: vec!["3".into(), "gamma".into()],
                    file2_fields: Vec::new(),
                    output_line: "3 gamma".into(),
                },
            ],
            classic_text: "1 alpha uno\n3 gamma\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("key1".into(), CtValue::Int(1)),
                    ("key2".into(), CtValue::Int(1)),
                    ("print_joined".into(), CtValue::Bool(true)),
                    ("print_unpaired1".into(), CtValue::Bool(false)),
                    ("print_unpaired2".into(), CtValue::Bool(false)),
                    ("ignore_case".into(), CtValue::Bool(false)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("headers".into(), CtValue::Bool(false)),
                    ("autoformat".into(), CtValue::Bool(false)),
                    ("check_order".into(), CtValue::String("default".into())),
                    (
                        "separator_mode".into(),
                        CtValue::String("whitespaces".into()),
                    ),
                    ("separator_text".into(), CtValue::String(" ".into())),
                    ("row_index".into(), CtValue::Int(1)),
                    ("row_kind".into(), CtValue::String("joined".into())),
                    ("join_key".into(), CtValue::String("1".into())),
                    ("source_file_num".into(), CtValue::Nothing),
                    (
                        "file1_fields".into(),
                        CtValue::List(vec![
                            CtValue::String("1".into()),
                            CtValue::String("alpha".into()),
                        ]),
                    ),
                    (
                        "file2_fields".into(),
                        CtValue::List(vec![
                            CtValue::String("1".into()),
                            CtValue::String("uno".into()),
                        ]),
                    ),
                    ("output_line".into(), CtValue::String("1 alpha uno".into())),
                ]),
                CtValue::Record(vec![
                    ("key1".into(), CtValue::Int(1)),
                    ("key2".into(), CtValue::Int(1)),
                    ("print_joined".into(), CtValue::Bool(true)),
                    ("print_unpaired1".into(), CtValue::Bool(false)),
                    ("print_unpaired2".into(), CtValue::Bool(false)),
                    ("ignore_case".into(), CtValue::Bool(false)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("headers".into(), CtValue::Bool(false)),
                    ("autoformat".into(), CtValue::Bool(false)),
                    ("check_order".into(), CtValue::String("default".into())),
                    (
                        "separator_mode".into(),
                        CtValue::String("whitespaces".into()),
                    ),
                    ("separator_text".into(), CtValue::String(" ".into())),
                    ("row_index".into(), CtValue::Int(2)),
                    ("row_kind".into(), CtValue::String("unpaired".into())),
                    ("join_key".into(), CtValue::String("3".into())),
                    ("source_file_num".into(), CtValue::Int(1)),
                    (
                        "file1_fields".into(),
                        CtValue::List(vec![
                            CtValue::String("3".into()),
                            CtValue::String("gamma".into()),
                        ]),
                    ),
                    ("file2_fields".into(), CtValue::List(vec![])),
                    ("output_line".into(), CtValue::String("3 gamma".into())),
                ]),
            ])
        );
    }

    #[test]
    fn display_columns_focus_on_join_rows() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![
                CtValue::String("row_index".into()),
                CtValue::String("row_kind".into()),
                CtValue::String("join_key".into()),
                CtValue::String("file1_fields".into()),
                CtValue::String("file2_fields".into()),
                CtValue::String("output_line".into()),
            ])
        );
    }
}
