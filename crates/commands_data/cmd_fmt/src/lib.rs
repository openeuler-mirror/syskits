use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdFmt;

struct FmtIntent {
    argv: Vec<OsString>,
}

struct FmtCore;

impl FmtIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("fmt"));

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

impl FmtCore {
    fn run_core(
        intent: &FmtIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "fmt",
            input,
            ctengine::argv_uses_stdin(
                &intent.argv,
                &["-w", "--width", "-g", "--goal", "-p", "--prefix"],
            ),
            || Ok(ct_fmt::fmt_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("fmt: {err}")),
        )?;
        Ok(match result {
            Ok(semantic) => (
                CtValue::String(semantic.output.clone()),
                semantic.output,
                semantic.stderr_text,
                semantic.exit_code,
            ),
            Err(err) => (
                CtValue::String(String::new()),
                String::new(),
                render_error_text(err.as_ref()),
                err.code(),
            ),
        })
    }
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("fmt: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'fmt --help' for more information.\n");
    }
    stderr
}

impl DataCommand for CmdFmt {
    fn signature(&self) -> DataSignature {
        DataSignature::new("fmt", "structured text formatting output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible fmt arguments",
                CtType::Any,
            ))
            .input(CtType::Any)
            .output(CtType::String)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = FmtIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = FmtCore::run_core(&intent, input)?;
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
                source: Some("fmt".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::FmtIntent;
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-w".into()), None),
                BoundArg::new(CtValue::String("20".into()), None),
            ],
            ..DataCall::named("fmt")
        };

        let intent = FmtIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("fmt"),
                OsString::from("-w"),
                OsString::from("20"),
            ]
        );
    }
}
