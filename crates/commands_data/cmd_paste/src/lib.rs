use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdPaste;

struct PasteIntent {
    argv: Vec<OsString>,
}

struct PasteCore;

impl PasteIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("paste"));

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

impl PasteCore {
    fn run_core(
        intent: &PasteIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "paste",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &["-d", "--delimiters"]),
            || Ok(ct_paste::paste_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("paste: {err}")),
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
    let mut stderr = format!("paste: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'paste --help' for more information.\n");
    }
    stderr
}

fn mode_name(mode: ct_paste::PasteMode) -> &'static str {
    match mode {
        ct_paste::PasteMode::Parallel => "parallel",
        ct_paste::PasteMode::Serial => "serial",
    }
}

fn row_to_value(mode: ct_paste::PasteMode, row: &ct_paste::PasteRow) -> CtValue {
    CtValue::Record(vec![
        ("mode".into(), CtValue::String(mode_name(mode).into())),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits in i64")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
    ])
}

fn semantic_to_value(semantic: &ct_paste::PasteSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic.mode, row))
            .collect(),
    )
}

impl DataCommand for CmdPaste {
    fn signature(&self) -> DataSignature {
        DataSignature::new("paste", "structured pasted line rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible paste arguments",
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
        let intent = PasteIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = PasteCore::run_core(&intent, input)?;
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
                source: Some("paste".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PasteIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-s".into()), None),
                BoundArg::new(CtValue::String("left.txt".into()), None),
                BoundArg::new(CtValue::String("right.txt".into()), None),
            ],
            ..DataCall::named("paste")
        };

        let intent = PasteIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("paste"),
                OsString::from("-s"),
                OsString::from("left.txt"),
                OsString::from("right.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_paste::PasteSemantic {
            mode: ct_paste::PasteMode::Parallel,
            rows: vec![
                ct_paste::PasteRow {
                    row_index: 1,
                    line: "a1\tb1".into(),
                },
                ct_paste::PasteRow {
                    row_index: 2,
                    line: "a2\tb2".into(),
                },
            ],
            classic_text: "a1\tb1\na2\tb2\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("mode".into(), CtValue::String("parallel".into())),
                    ("row_index".into(), CtValue::Int(1)),
                    ("line".into(), CtValue::String("a1\tb1".into())),
                ]),
                CtValue::Record(vec![
                    ("mode".into(), CtValue::String("parallel".into())),
                    ("row_index".into(), CtValue::Int(2)),
                    ("line".into(), CtValue::String("a2\tb2".into())),
                ]),
            ])
        );
    }
}
