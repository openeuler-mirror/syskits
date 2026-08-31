use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdTac;

struct TacIntent {
    argv: Vec<OsString>,
}

struct TacCore;

impl TacIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("tac"));

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

impl TacCore {
    fn run_core(
        intent: &TacIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "tac",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &["-s", "--separator"]),
            || Ok(ct_tac::tac_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("tac: {err}")),
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
    let mut stderr = format!("tac: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'tac --help' for more information.\n");
    }
    stderr
}

fn row_to_value(semantic: &ct_tac::TacSemantic, row: &ct_tac::TacRow) -> CtValue {
    CtValue::Record(vec![
        (
            "separator_kind".into(),
            CtValue::String(semantic.separator_kind.clone()),
        ),
        (
            "separator_text".into(),
            CtValue::String(semantic.separator_text.clone()),
        ),
        ("before".into(), CtValue::Bool(semantic.before)),
        (
            "source_name".into(),
            CtValue::String(row.source_name.clone()),
        ),
        (
            "file_index".into(),
            CtValue::Int(i64::try_from(row.file_index).expect("file index fits")),
        ),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("chunk".into(), CtValue::String(row.chunk.clone())),
        (
            "byte_len".into(),
            CtValue::Int(i64::try_from(row.byte_len).expect("byte len fits")),
        ),
    ])
}

fn semantic_to_value(semantic: &ct_tac::TacSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdTac {
    fn signature(&self) -> DataSignature {
        DataSignature::new("tac", "structured tac output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible tac arguments",
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
        let intent = TacIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = TacCore::run_core(&intent, input)?;
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
                source: Some("tac".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{TacIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-s".into()), None),
                BoundArg::new(CtValue::String(":".into()), None),
                BoundArg::new(CtValue::String("file.txt".into()), None),
            ],
            ..DataCall::named("tac")
        };

        let intent = TacIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("tac"),
                OsString::from("-s"),
                OsString::from(":"),
                OsString::from("file.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_tac::TacSemantic {
            separator_kind: "string".into(),
            separator_text: "\n".into(),
            before: false,
            rows: vec![ct_tac::TacRow {
                source_name: "file.txt".into(),
                file_index: 1,
                row_index: 1,
                chunk: "gamma\n".into(),
                byte_len: 6,
            }],
            classic_text: "gamma\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("separator_kind".into(), CtValue::String("string".into())),
                ("separator_text".into(), CtValue::String("\n".into())),
                ("before".into(), CtValue::Bool(false)),
                ("source_name".into(), CtValue::String("file.txt".into())),
                ("file_index".into(), CtValue::Int(1)),
                ("row_index".into(), CtValue::Int(1)),
                ("chunk".into(), CtValue::String("gamma\n".into())),
                ("byte_len".into(), CtValue::Int(6)),
            ])])
        );
    }
}
