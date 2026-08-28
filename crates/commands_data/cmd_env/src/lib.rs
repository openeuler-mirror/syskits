use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdEnv;

struct EnvIntent {
    argv: Vec<OsString>,
}

struct EnvCore;

impl EnvIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("env"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("env: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl EnvCore {
    fn run_core(intent: &EnvIntent) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_env::env_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn semantic_to_value(semantic: &ct_env::EnvSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_env::EnvRow) -> CtValue {
    CtValue::Record(vec![
        ("name".into(), CtValue::String(row.name.clone())),
        ("value".into(), CtValue::String(row.value.clone())),
    ])
}

impl DataCommand for CmdEnv {
    fn signature(&self) -> DataSignature {
        DataSignature::new("env", "structured environment snapshot")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = EnvIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = EnvCore::run_core(&intent)?;
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
                source: Some("env".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{EnvIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-i".into()), None),
                BoundArg::new(CtValue::String("FOO=bar".into()), None),
                BoundArg::new(CtValue::String("BAR=baz".into()), None),
            ],
            ..DataCall::named("env")
        };

        let intent = EnvIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("env"),
                OsString::from("-i"),
                OsString::from("FOO=bar"),
                OsString::from("BAR=baz"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_env::EnvSemantic {
            rows: vec![ct_env::EnvRow {
                name: "FOO".into(),
                value: "bar".into(),
            }],
            classic_text: "FOO=bar\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("name".into(), CtValue::String("FOO".into())),
                ("value".into(), CtValue::String("bar".into())),
            ])])
        );
    }
}
