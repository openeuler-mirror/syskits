use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdWhoami;

struct WhoamiIntent {
    argv: Vec<OsString>,
}
struct WhoamiOutput {
    value: CtValue,
    classic_text: String,
    classic_append_newline: bool,
    stderr_text: Option<String>,
    exit_code: i32,
}
struct WhoamiCore;

impl WhoamiIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("whoami"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("whoami: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl WhoamiCore {
    fn run_core(intent: WhoamiIntent) -> Result<WhoamiOutput, CtDiagnosticError> {
        if let Err(err) = ct_whoami::ct_app().try_get_matches_from(intent.argv.iter().cloned()) {
            let rendered = err.render().to_string();
            return Ok(if err.use_stderr() {
                WhoamiOutput {
                    value: CtValue::Nothing,
                    classic_text: String::new(),
                    classic_append_newline: false,
                    stderr_text: Some(rendered),
                    exit_code: err.exit_code(),
                }
            } else {
                WhoamiOutput {
                    value: CtValue::Nothing,
                    classic_text: rendered,
                    classic_append_newline: false,
                    stderr_text: None,
                    exit_code: err.exit_code(),
                }
            });
        }

        let username =
            ct_whoami::whoami_exec().map_err(|e| CtDiagnosticError::simple(e.to_string()))?;
        let username = username.to_string_lossy().into_owned();
        Ok(WhoamiOutput {
            value: username_value(&username),
            classic_text: username,
            classic_append_newline: true,
            stderr_text: None,
            exit_code: 0,
        })
    }
}

fn username_value(username: &str) -> CtValue {
    CtValue::Record(vec![(
        "username".into(),
        CtValue::String(username.to_string()),
    )])
}

impl DataCommand for CmdWhoami {
    fn signature(&self) -> DataSignature {
        DataSignature::new("whoami", "structured current username")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible whoami arguments",
                CtType::Any,
            ))
            .input(CtType::Nothing)
            .output(CtType::Any)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = WhoamiIntent::from_call(call)?;
        let output = WhoamiCore::run_core(intent)?;
        Ok(CtPipelineData::Value(
            output.value,
            CtPipelineMetadata {
                classic_text: Some(output.classic_text),
                classic_bytes: None,
                classic_append_newline: output.classic_append_newline,
                stderr_text: output.stderr_text,
                exit_code: output.exit_code,
                source: Some("whoami".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{WhoamiIntent, username_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("--help".into()), None)],
            ..DataCall::named("whoami")
        };

        let intent = WhoamiIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("whoami"), OsString::from("--help")]
        );
    }

    #[test]
    fn username_value_renders_structured_record() {
        assert_eq!(
            username_value("root"),
            CtValue::Record(vec![("username".into(), CtValue::String("root".into()))])
        );
    }
}
