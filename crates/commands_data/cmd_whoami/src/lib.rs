use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};

#[derive(Default)]
pub struct CmdWhoami;

struct WhoamiIntent;
struct WhoamiCore;

impl WhoamiCore {
    fn run_core(_intent: WhoamiIntent) -> Result<CtValue, CtDiagnosticError> {
        let username =
            ct_whoami::whoami_exec().map_err(|e| CtDiagnosticError::simple(e.to_string()))?;
        Ok(CtValue::String(username.to_string_lossy().into_owned()))
    }
}

impl DataCommand for CmdWhoami {
    fn signature(&self) -> DataSignature {
        DataSignature::new("whoami", "structured current username")
            .input(CtType::Nothing)
            .output(CtType::String)
    }

    fn run(
        &self,
        _call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let value = WhoamiCore::run_core(WhoamiIntent)?;
        let classic_text = match &value {
            CtValue::String(username) => Some(username.clone()),
            _ => None,
        };
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text,
                classic_bytes: None,
                classic_append_newline: true,
                stderr_text: None,
                exit_code: 0,
                source: Some("whoami".into()),
                ..Default::default()
            },
        ))
    }
}
