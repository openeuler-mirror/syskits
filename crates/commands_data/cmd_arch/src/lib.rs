use clap::error::ErrorKind;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdArch;

struct ArchIntent {
    argv: Vec<OsString>,
}

struct ArchCore;
struct ArchOutput {
    value: CtValue,
    classic_text: String,
    classic_append_newline: bool,
    stderr_text: Option<String>,
    exit_code: i32,
}

impl ArchIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("arch"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("arch: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl ArchCore {
    fn run_core(intent: ArchIntent) -> Result<ArchOutput, CtDiagnosticError> {
        if let Err(err) = ct_arch::ct_app().try_get_matches_from(intent.argv.iter().cloned()) {
            let rendered = err.render().to_string();
            let is_display = matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            return Ok(if is_display {
                ArchOutput {
                    value: CtValue::Nothing,
                    classic_text: rendered,
                    classic_append_newline: false,
                    stderr_text: None,
                    exit_code: 0,
                }
            } else {
                ArchOutput {
                    value: CtValue::Nothing,
                    classic_text: String::new(),
                    classic_append_newline: false,
                    stderr_text: Some(format!(
                        "{rendered}Try 'arch --help' for more information.\n"
                    )),
                    exit_code: 1,
                }
            });
        }

        ct_arch::arch_main(intent.argv.into_iter())
            .map_err(|e| CtDiagnosticError::simple(e.to_string()).with_code(e.code()))
            .map(|machine| ArchOutput {
                value: arch_value(machine.clone()),
                classic_text: machine,
                classic_append_newline: true,
                stderr_text: None,
                exit_code: 0,
            })
    }
}

fn arch_value(machine: String) -> CtValue {
    CtValue::Record(vec![("machine".into(), CtValue::String(machine))])
}

impl DataCommand for CmdArch {
    fn signature(&self) -> DataSignature {
        DataSignature::new("arch", "structured machine architecture")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible arch arguments",
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
        let intent = ArchIntent::from_call(call)?;
        let output = ArchCore::run_core(intent)?;
        Ok(CtPipelineData::Value(
            output.value,
            CtPipelineMetadata {
                classic_text: Some(output.classic_text),
                classic_bytes: None,
                classic_append_newline: output.classic_append_newline,
                stderr_text: output.stderr_text,
                exit_code: output.exit_code,
                source: Some("arch".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ArchIntent, arch_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("--help".into()), None)],
            ..DataCall::named("arch")
        };

        let intent = ArchIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("arch"), OsString::from("--help")]
        );
    }

    #[test]
    fn arch_value_uses_machine_field() {
        assert_eq!(
            arch_value("aarch64".into()),
            CtValue::Record(vec![("machine".into(), CtValue::String("aarch64".into()))])
        );
    }
}
