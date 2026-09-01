use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdTee;

struct TeeIntent {
    argv: Vec<OsString>,
}

struct TeeCore;

impl TeeIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("tee"));

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

impl TeeCore {
    fn run_core(
        intent: &TeeIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "tee",
            input,
            true,
            || Ok(ct_tee::tee_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("tee: {err}")),
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
    let mut stderr = format!("tee: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'tee --help' for more information.\n");
    }
    stderr
}

fn first_target_value(target_files: &[String]) -> CtValue {
    match target_files.first() {
        Some(target) => CtValue::String(target.clone()),
        None => CtValue::Nothing,
    }
}

fn target_files_value(target_files: &[String]) -> CtValue {
    CtValue::List(
        target_files
            .iter()
            .map(|target| CtValue::String(target.clone()))
            .collect(),
    )
}

fn row_to_value(semantic: &ct_tee::TeeSemantic, row: &ct_tee::TeeRow) -> CtValue {
    CtValue::Record(vec![
        ("append".into(), CtValue::Bool(semantic.append)),
        (
            "ignore_interrupts".into(),
            CtValue::Bool(semantic.ignore_interrupts),
        ),
        (
            "output_error_mode".into(),
            CtValue::String(semantic.output_error_mode.clone()),
        ),
        (
            "target_count".into(),
            CtValue::Int(i64::try_from(semantic.target_files.len()).expect("target count fits")),
        ),
        (
            "target_file".into(),
            first_target_value(&semantic.target_files),
        ),
        (
            "target_files".into(),
            target_files_value(&semantic.target_files),
        ),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
        (
            "byte_len".into(),
            CtValue::Int(i64::try_from(row.byte_len).expect("byte len fits")),
        ),
        ("terminated".into(), CtValue::Bool(row.terminated)),
    ])
}

fn semantic_to_value(semantic: &ct_tee::TeeSemantic) -> CtValue {
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
        ["append", "target_file", "line", "byte_len"]
            .into_iter()
            .map(|name| CtValue::String(name.into()))
            .collect(),
    )
}

impl DataCommand for CmdTee {
    fn signature(&self) -> DataSignature {
        DataSignature::new("tee", "structured tee output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible tee arguments",
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
        let intent = TeeIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = TeeCore::run_core(&intent, input)?;
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
            source: Some("tee".into()),
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
    use super::{TeeIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-a".into()), None),
                BoundArg::new(CtValue::String("file.txt".into()), None),
            ],
            ..DataCall::named("tee")
        };

        let intent = TeeIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("tee"),
                OsString::from("-a"),
                OsString::from("file.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_tee::TeeSemantic {
            append: false,
            ignore_interrupts: false,
            output_error_mode: "warn_nopipe".into(),
            target_files: vec!["out.txt".into()],
            rows: vec![
                ct_tee::TeeRow {
                    row_index: 1,
                    line: "alpha\n".into(),
                    byte_len: 6,
                    terminated: true,
                },
                ct_tee::TeeRow {
                    row_index: 2,
                    line: "beta".into(),
                    byte_len: 4,
                    terminated: false,
                },
            ],
            classic_text: "alpha\nbeta".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("append".into(), CtValue::Bool(false)),
                    ("ignore_interrupts".into(), CtValue::Bool(false)),
                    (
                        "output_error_mode".into(),
                        CtValue::String("warn_nopipe".into()),
                    ),
                    ("target_count".into(), CtValue::Int(1)),
                    ("target_file".into(), CtValue::String("out.txt".into())),
                    (
                        "target_files".into(),
                        CtValue::List(vec![CtValue::String("out.txt".into())]),
                    ),
                    ("row_index".into(), CtValue::Int(1)),
                    ("line".into(), CtValue::String("alpha\n".into())),
                    ("byte_len".into(), CtValue::Int(6)),
                    ("terminated".into(), CtValue::Bool(true)),
                ]),
                CtValue::Record(vec![
                    ("append".into(), CtValue::Bool(false)),
                    ("ignore_interrupts".into(), CtValue::Bool(false)),
                    (
                        "output_error_mode".into(),
                        CtValue::String("warn_nopipe".into()),
                    ),
                    ("target_count".into(), CtValue::Int(1)),
                    ("target_file".into(), CtValue::String("out.txt".into())),
                    (
                        "target_files".into(),
                        CtValue::List(vec![CtValue::String("out.txt".into())]),
                    ),
                    ("row_index".into(), CtValue::Int(2)),
                    ("line".into(), CtValue::String("beta".into())),
                    ("byte_len".into(), CtValue::Int(4)),
                    ("terminated".into(), CtValue::Bool(false)),
                ]),
            ])
        );
    }

    #[test]
    fn display_columns_focus_on_written_output() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![
                CtValue::String("append".into()),
                CtValue::String("target_file".into()),
                CtValue::String("line".into()),
                CtValue::String("byte_len".into()),
            ])
        );
    }
}
