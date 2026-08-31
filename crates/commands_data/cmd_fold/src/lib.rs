use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdFold;

struct FoldIntent {
    argv: Vec<OsString>,
}

struct FoldCore;

impl FoldIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("fold"));

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

impl FoldCore {
    fn run_core(
        intent: &FoldIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "fold",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &["-w", "--width"]),
            || Ok(ct_fold::fold_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("fold: {err}")),
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
    let mut stderr = format!("fold: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'fold --help' for more information.\n");
    }
    stderr
}

fn mode_name(mode: ct_fold::FoldMode) -> &'static str {
    match mode {
        ct_fold::FoldMode::Columns => "columns",
        ct_fold::FoldMode::Bytes => "bytes",
        ct_fold::FoldMode::Characters => "characters",
    }
}

fn row_to_value(semantic: &ct_fold::FoldSemantic, row: &ct_fold::FoldRow) -> CtValue {
    CtValue::Record(vec![
        (
            "mode".into(),
            CtValue::String(mode_name(semantic.mode).into()),
        ),
        (
            "width".into(),
            CtValue::Int(i64::try_from(semantic.width).expect("width fits in i64")),
        ),
        ("break_spaces".into(), CtValue::Bool(semantic.break_spaces)),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits in i64")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
    ])
}

fn semantic_to_value(semantic: &ct_fold::FoldSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdFold {
    fn signature(&self) -> DataSignature {
        DataSignature::new("fold", "structured folded line rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible fold arguments",
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
        let intent = FoldIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = FoldCore::run_core(&intent, input)?;
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
                source: Some("fold".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{FoldIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-w".into()), None),
                BoundArg::new(CtValue::String("4".into()), None),
                BoundArg::new(CtValue::String("file.txt".into()), None),
            ],
            ..DataCall::named("fold")
        };

        let intent = FoldIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("fold"),
                OsString::from("-w"),
                OsString::from("4"),
                OsString::from("file.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_fold::FoldSemantic {
            mode: ct_fold::FoldMode::Columns,
            width: 4,
            break_spaces: false,
            rows: vec![
                ct_fold::FoldRow {
                    row_index: 1,
                    line: "abcd".into(),
                },
                ct_fold::FoldRow {
                    row_index: 2,
                    line: "efgh".into(),
                },
            ],
            classic_text: "abcd\nefgh\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("mode".into(), CtValue::String("columns".into())),
                    ("width".into(), CtValue::Int(4)),
                    ("break_spaces".into(), CtValue::Bool(false)),
                    ("row_index".into(), CtValue::Int(1)),
                    ("line".into(), CtValue::String("abcd".into())),
                ]),
                CtValue::Record(vec![
                    ("mode".into(), CtValue::String("columns".into())),
                    ("width".into(), CtValue::Int(4)),
                    ("break_spaces".into(), CtValue::Bool(false)),
                    ("row_index".into(), CtValue::Int(2)),
                    ("line".into(), CtValue::String("efgh".into())),
                ]),
            ])
        );
    }
}
