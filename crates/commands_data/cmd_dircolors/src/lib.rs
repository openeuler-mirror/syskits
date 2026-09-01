use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdDircolors;

struct DircolorsIntent {
    argv: Vec<OsString>,
}

struct DircolorsCore;

impl DircolorsIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("dircolors"));

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

impl DircolorsCore {
    fn run_core(
        intent: &DircolorsIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "dircolors",
            input,
            ctengine::argv_has_stdin_operand(&intent.argv, &[]),
            || {
                Ok(ct_dircolors::dircolors_native_semantic(
                    intent.argv.iter().cloned(),
                ))
            },
            |err| CtDiagnosticError::simple(format!("dircolors: {err}")),
        )?;
        Ok(match result {
            Ok(semantic) => {
                let classic_text = if semantic.output.is_empty() {
                    String::new()
                } else {
                    format!("{}\n", semantic.output)
                };
                (semantic_to_value(&semantic), classic_text, String::new(), 0)
            }
            Err(err) => (
                CtValue::Record(vec![]),
                String::new(),
                render_error_text(err.as_ref()),
                err.code(),
            ),
        })
    }
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("dircolors: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'dircolors --help' for more information.\n");
    }
    stderr
}

fn semantic_to_value(semantic: &ct_dircolors::DircolorsSemantic) -> CtValue {
    CtValue::Record(vec![
        (
            "output_kind".into(),
            CtValue::String(semantic.output_kind.clone()),
        ),
        ("output".into(), CtValue::String(semantic.output.clone())),
    ])
}

impl DataCommand for CmdDircolors {
    fn signature(&self) -> DataSignature {
        DataSignature::new("dircolors", "structured dircolors output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible dircolors arguments",
                CtType::Any,
            ))
            .input(CtType::Any)
            .output(CtType::Record)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = DircolorsIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) =
            DircolorsCore::run_core(&intent, input)?;
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
                source: Some("dircolors".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{DircolorsIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("-p".into()), None)],
            ..DataCall::named("dircolors")
        };

        let intent = DircolorsIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("dircolors"), OsString::from("-p")]
        );
    }

    #[test]
    fn semantic_to_value_renders_record() {
        let value = semantic_to_value(&ct_dircolors::DircolorsSemantic {
            output_kind: "shell".into(),
            output: "LS_COLORS='di=01;34:';\nexport LS_COLORS".into(),
        });

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("output_kind".into(), CtValue::String("shell".into())),
                (
                    "output".into(),
                    CtValue::String("LS_COLORS='di=01;34:';\nexport LS_COLORS".into())
                ),
            ])
        );
    }
}
