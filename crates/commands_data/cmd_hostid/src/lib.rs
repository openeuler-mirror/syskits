use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdHostid;

struct HostidIntent {
    argv: Vec<OsString>,
}

struct HostidCore;

impl HostidIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("hostid"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("hostid: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl HostidCore {
    fn run_core(intent: &HostidIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_hostid::hostid_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((semantic_to_value(&semantic), semantic.hostid))
    }
}

fn semantic_to_value(semantic: &ct_hostid::HostidSemantic) -> CtValue {
    CtValue::Record(vec![(
        "hostid".into(),
        CtValue::String(semantic.hostid.clone()),
    )])
}

impl DataCommand for CmdHostid {
    fn signature(&self) -> DataSignature {
        DataSignature::new("hostid", "structured host identifier")
            .input(CtType::Nothing)
            .output(CtType::Record)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = HostidIntent::from_call(call)?;
        let (value, classic_text) = HostidCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: true,
                stderr_text: None,
                exit_code: 0,
                source: Some("hostid".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{HostidIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("--help".into()), None)],
            ..DataCall::named("hostid")
        };

        let intent = HostidIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("hostid"), OsString::from("--help")]
        );
    }

    #[test]
    fn semantic_to_value_renders_record() {
        let value = semantic_to_value(&ct_hostid::HostidSemantic {
            hostid: "007f0101".into(),
        });

        assert_eq!(
            value,
            CtValue::Record(vec![("hostid".into(), CtValue::String("007f0101".into()))])
        );
    }
}
