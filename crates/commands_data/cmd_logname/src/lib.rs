use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdLogname;

struct LognameIntent {
    argv: Vec<OsString>,
}

struct LognameCore;

impl LognameIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("logname"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple(
                    "logname: argument must be string",
                ));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl LognameCore {
    fn run_core(
        intent: &LognameIntent,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_logname::logname_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn row_to_value(row: &ct_logname::LognameSemanticRow) -> CtValue {
    CtValue::Record(vec![
        ("login_name".into(), CtValue::String(row.login_name.clone())),
        ("available".into(), CtValue::Bool(row.available)),
        ("source".into(), CtValue::String(row.source.clone())),
    ])
}

fn semantic_to_value(semantic: &ct_logname::LognameSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

impl DataCommand for CmdLogname {
    fn signature(&self) -> DataSignature {
        DataSignature::new("logname", "structured current login-name rows")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = LognameIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = LognameCore::run_core(&intent)?;
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
                source: Some("logname".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{LognameIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(
                CtValue::String("--invalid-argument".into()),
                None,
            )],
            ..DataCall::named("logname")
        };

        let intent = LognameIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("logname"),
                OsString::from("--invalid-argument"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_logname::LognameSemantic {
            rows: vec![ct_logname::LognameSemanticRow {
                login_name: "root".into(),
                available: true,
                source: "posix:getlogin".into(),
            }],
            classic_text: "root\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("login_name".into(), CtValue::String("root".into())),
                ("available".into(), CtValue::Bool(true)),
                ("source".into(), CtValue::String("posix:getlogin".into())),
            ])])
        );
    }
}
