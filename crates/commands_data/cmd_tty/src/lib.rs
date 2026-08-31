use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdTty;

struct TtyIntent {
    argv: Vec<OsString>,
}

struct TtyCore;

impl TtyIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("tty"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("tty: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl TtyCore {
    fn run_core(intent: &TtyIntent) -> Result<(CtValue, String, i32, bool), CtDiagnosticError> {
        let semantic = ct_tty::tty_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.exit_code,
            semantic.silent,
        ))
    }
}

fn semantic_to_value(semantic: &ct_tty::TtySemantic) -> CtValue {
    CtValue::Record(vec![
        ("is_tty".into(), CtValue::Bool(semantic.is_tty)),
        (
            "tty_name".into(),
            match &semantic.tty_name {
                Some(value) => CtValue::String(value.clone()),
                None => CtValue::Nothing,
            },
        ),
        ("silent".into(), CtValue::Bool(semantic.silent)),
    ])
}

impl DataCommand for CmdTty {
    fn signature(&self) -> DataSignature {
        DataSignature::new("tty", "structured terminal state output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible tty arguments",
                CtType::Any,
            ))
            .input(CtType::Nothing)
            .output(CtType::Record)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = TtyIntent::from_call(call)?;
        let (value, classic_text, exit_code, silent) = TtyCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: !silent,
                stderr_text: None,
                exit_code,
                source: Some("tty".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{TtyIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("-s".into()), None)],
            ..DataCall::named("tty")
        };

        let intent = TtyIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("tty"), OsString::from("-s")]
        );
    }

    #[test]
    fn semantic_to_value_renders_record() {
        let value = semantic_to_value(&ct_tty::TtySemantic {
            is_tty: false,
            tty_name: None,
            silent: false,
            classic_text: "not a tty".into(),
            exit_code: 1,
        });

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("is_tty".into(), CtValue::Bool(false)),
                ("tty_name".into(), CtValue::Nothing),
                ("silent".into(), CtValue::Bool(false)),
            ])
        );
    }
}
