use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdPathchk;

struct PathchkIntent {
    argv: Vec<OsString>,
}

struct PathchkCore;

impl PathchkIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("pathchk"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple(
                    "pathchk: argument must be string",
                ));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl PathchkCore {
    fn run_core(
        intent: &PathchkIntent,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_pathchk::pathchk_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            String::new(),
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn semantic_to_value(semantic: &ct_pathchk::PathchkSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_pathchk::PathchkRow) -> CtValue {
    CtValue::Record(vec![
        ("path".into(), CtValue::String(row.path.clone())),
        ("ok".into(), CtValue::Bool(row.ok)),
        (
            "diagnostic_kind".into(),
            match &row.diagnostic_kind {
                Some(value) => CtValue::String(value.clone()),
                None => CtValue::Nothing,
            },
        ),
        (
            "message".into(),
            match &row.message {
                Some(value) => CtValue::String(value.clone()),
                None => CtValue::Nothing,
            },
        ),
    ])
}

impl DataCommand for CmdPathchk {
    fn signature(&self) -> DataSignature {
        DataSignature::new("pathchk", "structured path validation diagnostics")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = PathchkIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = PathchkCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: Some(stderr_text),
                exit_code,
                source: Some("pathchk".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PathchkIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-p".into()), None),
                BoundArg::new(CtValue::String("a".into()), None),
            ],
            ..DataCall::named("pathchk")
        };

        let intent = PathchkIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("pathchk"),
                OsString::from("-p"),
                OsString::from("a")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_pathchk::PathchkSemantic {
            rows: vec![ct_pathchk::PathchkRow {
                path: "bad".into(),
                ok: false,
                diagnostic_kind: Some("non_portable_character".into()),
                message: Some("pathchk: non-portable character '#' in file name 'bad#'".into()),
            }],
            stderr_text: "pathchk: non-portable character '#' in file name 'bad#'\n".into(),
            exit_code: 1,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("path".into(), CtValue::String("bad".into())),
                ("ok".into(), CtValue::Bool(false)),
                (
                    "diagnostic_kind".into(),
                    CtValue::String("non_portable_character".into())
                ),
                (
                    "message".into(),
                    CtValue::String(
                        "pathchk: non-portable character '#' in file name 'bad#'".into()
                    )
                ),
            ])])
        );
    }
}
