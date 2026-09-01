use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdCut;

struct CutIntent {
    argv: Vec<OsString>,
}

struct CutCore;

impl CutIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("cut"));

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

impl CutCore {
    fn run_core(
        intent: &CutIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "cut",
            input,
            ctengine::argv_uses_stdin(
                &intent.argv,
                &[
                    "-b",
                    "--bytes",
                    "-c",
                    "--characters",
                    "-f",
                    "--fields",
                    "-d",
                    "--delimiter",
                    "--output-delimiter",
                ],
            ),
            || Ok(ct_cut::cut_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("cut: {err}")),
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
    let mut stderr = format!("cut: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'cut --help' for more information.\n");
    }
    stderr
}

fn opt_string_to_value(value: Option<&str>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.into()),
        None => CtValue::Nothing,
    }
}

fn row_to_value(semantic: &ct_cut::CutSemantic, row: &ct_cut::CutRow) -> CtValue {
    CtValue::Record(vec![
        ("mode".into(), CtValue::String(semantic.mode.clone())),
        (
            "range_specs".into(),
            CtValue::List(
                semantic
                    .range_specs
                    .iter()
                    .cloned()
                    .map(CtValue::String)
                    .collect(),
            ),
        ),
        (
            "delimiter".into(),
            opt_string_to_value(semantic.delimiter.as_deref()),
        ),
        (
            "output_delimiter".into(),
            opt_string_to_value(semantic.output_delimiter.as_deref()),
        ),
        (
            "only_delimited".into(),
            CtValue::Bool(semantic.only_delimited),
        ),
        (
            "zero_terminated".into(),
            CtValue::Bool(semantic.zero_terminated),
        ),
        (
            "no_split_multibyte".into(),
            CtValue::Bool(semantic.no_split_multibyte),
        ),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
        (
            "byte_length".into(),
            CtValue::Int(i64::try_from(row.byte_length).expect("byte length fits")),
        ),
    ])
}

fn semantic_to_value(semantic: &ct_cut::CutSemantic) -> CtValue {
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
        ["row_index", "line", "mode", "range_specs"]
            .into_iter()
            .map(|column| CtValue::String(column.into()))
            .collect(),
    )
}

impl DataCommand for CmdCut {
    fn signature(&self) -> DataSignature {
        DataSignature::new("cut", "structured cut output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible cut arguments",
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
        let intent = CutIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = CutCore::run_core(&intent, input)?;
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
            source: Some("cut".into()),
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
    use super::{CutIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-d".into()), None),
                BoundArg::new(CtValue::String(":".into()), None),
                BoundArg::new(CtValue::String("-f".into()), None),
                BoundArg::new(CtValue::String("2".into()), None),
                BoundArg::new(CtValue::String("input.txt".into()), None),
            ],
            ..DataCall::named("cut")
        };

        let intent = CutIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("cut"),
                OsString::from("-d"),
                OsString::from(":"),
                OsString::from("-f"),
                OsString::from("2"),
                OsString::from("input.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_cut::CutSemantic {
            mode: "fields".into(),
            range_specs: vec!["2".into()],
            delimiter: Some(":".into()),
            output_delimiter: None,
            only_delimited: false,
            zero_terminated: false,
            no_split_multibyte: false,
            rows: vec![
                ct_cut::CutRow {
                    row_index: 1,
                    line: "one".into(),
                    byte_length: 3,
                },
                ct_cut::CutRow {
                    row_index: 2,
                    line: String::new(),
                    byte_length: 0,
                },
            ],
            classic_text: "one\n\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("mode".into(), CtValue::String("fields".into())),
                    (
                        "range_specs".into(),
                        CtValue::List(vec![CtValue::String("2".into())]),
                    ),
                    ("delimiter".into(), CtValue::String(":".into())),
                    ("output_delimiter".into(), CtValue::Nothing),
                    ("only_delimited".into(), CtValue::Bool(false)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("no_split_multibyte".into(), CtValue::Bool(false)),
                    ("row_index".into(), CtValue::Int(1)),
                    ("line".into(), CtValue::String("one".into())),
                    ("byte_length".into(), CtValue::Int(3)),
                ]),
                CtValue::Record(vec![
                    ("mode".into(), CtValue::String("fields".into())),
                    (
                        "range_specs".into(),
                        CtValue::List(vec![CtValue::String("2".into())]),
                    ),
                    ("delimiter".into(), CtValue::String(":".into())),
                    ("output_delimiter".into(), CtValue::Nothing),
                    ("only_delimited".into(), CtValue::Bool(false)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("no_split_multibyte".into(), CtValue::Bool(false)),
                    ("row_index".into(), CtValue::Int(2)),
                    ("line".into(), CtValue::String(String::new())),
                    ("byte_length".into(), CtValue::Int(0)),
                ]),
            ])
        );
    }

    #[test]
    fn display_columns_focus_on_cut_result() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![
                CtValue::String("row_index".into()),
                CtValue::String("line".into()),
                CtValue::String("mode".into()),
                CtValue::String("range_specs".into()),
            ])
        );
    }
}
