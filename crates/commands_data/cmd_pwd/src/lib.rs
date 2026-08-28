use ct_pwd::PwdMode;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};

#[derive(Default)]
pub struct CmdPwd;

#[derive(Debug)]
struct PwdIntent {
    mode: PwdMode,
}

struct PwdCore;

impl PwdCore {
    fn run_core(intent: PwdIntent) -> Result<CtValue, CtDiagnosticError> {
        let path = ct_pwd::resolve_pwd_path(intent.mode)
            .map_err(|e| CtDiagnosticError::simple(format!("pwd: {e}")))?;
        Ok(CtValue::Record(vec![(
            "path".into(),
            CtValue::String(path.display().to_string()),
        )]))
    }
}

fn pwd_classic_text(value: &CtValue) -> Option<String> {
    let CtValue::Record(fields) = value else {
        return None;
    };

    fields.iter().find_map(|(name, value)| {
        if name == "path" {
            match value {
                CtValue::String(path) => Some(path.clone()),
                _ => None,
            }
        } else {
            None
        }
    })
}

impl PwdIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut logical = false;
        let mut physical = false;

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("pwd: argument must be string"));
            };

            match arg.as_str() {
                "-L" | "--logical" => logical = true,
                "-P" | "--physical" => physical = true,
                other if other.starts_with('-') => {
                    return Err(CtDiagnosticError::simple(format!(
                        "pwd: unsupported option `{other}`"
                    ))
                    .with_code(1));
                }
                _ => {}
            }
        }

        let mode = if physical {
            PwdMode::Physical
        } else if logical || std::env::var("POSIXLY_CORRECT").is_ok() {
            PwdMode::Logical
        } else {
            PwdMode::Physical
        };

        Ok(Self { mode })
    }
}

impl DataCommand for CmdPwd {
    fn signature(&self) -> DataSignature {
        DataSignature::new("pwd", "structured current working directory")
            .input(CtType::Nothing)
            .output(CtType::Record)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = PwdIntent::from_call(call)?;
        let value = PwdCore::run_core(intent)?;
        let classic_text = pwd_classic_text(&value);
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text,
                classic_bytes: None,
                classic_append_newline: true,
                stderr_text: None,
                exit_code: 0,
                source: Some("pwd".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PwdIntent, pwd_classic_text};
    use ct_pwd::PwdMode;
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};

    #[test]
    fn from_call_prefers_physical_when_both_flags_are_present() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("--logical".into()), None),
                BoundArg::new(CtValue::String("--physical".into()), None),
            ],
            ..DataCall::named("pwd")
        };

        let intent = PwdIntent::from_call(&call).expect("intent");

        assert_eq!(intent.mode, PwdMode::Physical);
    }

    #[test]
    fn from_call_rejects_unknown_options() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("--unknown".into()), None)],
            ..DataCall::named("pwd")
        };

        let err = PwdIntent::from_call(&call).expect_err("unknown option should fail");

        assert_eq!(err.code, 1);
        assert!(err.to_string().contains("unsupported option"));
    }

    #[test]
    fn pwd_classic_text_reads_path_field() {
        let value = CtValue::Record(vec![("path".into(), CtValue::String("/tmp".into()))]);

        assert_eq!(pwd_classic_text(&value), Some("/tmp".into()));
    }
}
