use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdTail;

struct TailIntent {
    argv: Vec<OsString>,
}

struct TailCore;

impl TailIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("tail"));

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

impl TailCore {
    fn run_core(
        intent: &TailIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "tail",
            input,
            ctengine::argv_uses_stdin(
                &intent.argv,
                &[
                    "-c",
                    "--bytes",
                    "-n",
                    "--lines",
                    "--pid",
                    "-s",
                    "--sleep-interval",
                ],
            ),
            || Ok(ct_tail::tail_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("tail: {err}")),
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
    let mut stderr = format!("tail: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'tail --help' for more information.\n");
    }
    stderr
}

fn opt_u8_to_value(value: Option<u8>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(i64::from(value)),
        None => CtValue::Nothing,
    }
}

fn opt_string_to_value(value: Option<&str>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.into()),
        None => CtValue::Nothing,
    }
}

fn row_to_value(semantic: &ct_tail::TailSemantic, row: &ct_tail::TailRow) -> CtValue {
    CtValue::Record(vec![
        ("mode".into(), CtValue::String(semantic.mode.clone())),
        ("signum".into(), CtValue::String(semantic.signum.clone())),
        (
            "count".into(),
            CtValue::Int(i64::try_from(semantic.count).expect("count fits")),
        ),
        (
            "delimiter_kind".into(),
            CtValue::String(semantic.delimiter_kind.clone()),
        ),
        (
            "delimiter_byte".into(),
            opt_u8_to_value(semantic.delimiter_byte),
        ),
        ("verbose".into(), CtValue::Bool(semantic.verbose)),
        (
            "follow_mode".into(),
            opt_string_to_value(semantic.follow_mode.as_deref()),
        ),
        (
            "source_name".into(),
            CtValue::String(row.source_name.clone()),
        ),
        (
            "source_index".into(),
            CtValue::Int(i64::try_from(row.source_index).expect("source index fits")),
        ),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        (
            "source_row_index".into(),
            CtValue::Int(i64::try_from(row.source_row_index).expect("source row index fits")),
        ),
        ("line".into(), CtValue::String(row.content.clone())),
        (
            "byte_len".into(),
            CtValue::Int(i64::try_from(row.byte_len).expect("byte len fits")),
        ),
        ("terminated".into(), CtValue::Bool(row.terminated)),
    ])
}

fn semantic_to_value(semantic: &ct_tail::TailSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdTail {
    fn signature(&self) -> DataSignature {
        DataSignature::new("tail", "structured tail output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible tail arguments",
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
        let intent = TailIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = TailCore::run_core(&intent, input)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: if stderr_text.is_empty() {
                    None
                } else {
                    Some(stderr_text)
                },
                exit_code,
                source: Some("tail".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{TailIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-n".into()), None),
                BoundArg::new(CtValue::String("2".into()), None),
                BoundArg::new(CtValue::String("file.txt".into()), None),
            ],
            ..DataCall::named("tail")
        };

        let intent = TailIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("tail"),
                OsString::from("-n"),
                OsString::from("2"),
                OsString::from("file.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_tail::TailSemantic {
            mode: "lines".into(),
            signum: "negative".into(),
            count: 2,
            delimiter_byte: Some(b'\n'),
            delimiter_kind: "newline".into(),
            verbose: false,
            follow_mode: None,
            rows: vec![ct_tail::TailRow {
                source_name: "file.txt".into(),
                source_index: 1,
                row_index: 1,
                source_row_index: 1,
                content: "gamma\n".into(),
                byte_len: 6,
                terminated: true,
            }],
            classic_text: "gamma\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("mode".into(), CtValue::String("lines".into())),
                ("signum".into(), CtValue::String("negative".into())),
                ("count".into(), CtValue::Int(2)),
                ("delimiter_kind".into(), CtValue::String("newline".into())),
                ("delimiter_byte".into(), CtValue::Int(i64::from(b'\n'))),
                ("verbose".into(), CtValue::Bool(false)),
                ("follow_mode".into(), CtValue::Nothing),
                ("source_name".into(), CtValue::String("file.txt".into())),
                ("source_index".into(), CtValue::Int(1)),
                ("row_index".into(), CtValue::Int(1)),
                ("source_row_index".into(), CtValue::Int(1)),
                ("line".into(), CtValue::String("gamma\n".into())),
                ("byte_len".into(), CtValue::Int(6)),
                ("terminated".into(), CtValue::Bool(true)),
            ])])
        );
    }
}
