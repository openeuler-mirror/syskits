use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdExpr;

struct ExprIntent {
    argv: Vec<OsString>,
}

struct ExprCore;

impl ExprIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("expr"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("expr: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl ExprCore {
    fn run_core(intent: &ExprIntent) -> Result<(CtValue, String, i32), CtDiagnosticError> {
        let semantic = ct_expr::expr_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        let exit_code = if semantic.truthy { 0 } else { 1 };
        Ok((
            semantic_to_value(&semantic),
            format!("{}\n", semantic.text),
            exit_code,
        ))
    }
}

fn semantic_to_value(semantic: &ct_expr::ExprSemantic) -> CtValue {
    CtValue::Record(vec![
        ("text".into(), CtValue::String(semantic.text.clone())),
        ("truthy".into(), CtValue::Bool(semantic.truthy)),
    ])
}

impl DataCommand for CmdExpr {
    fn signature(&self) -> DataSignature {
        DataSignature::new("expr", "structured expression result")
            .input(CtType::Nothing)
            .output(CtType::Record)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = ExprIntent::from_call(call)?;
        let (value, classic_text, exit_code) = ExprCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: None,
                exit_code,
                source: Some("expr".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ExprIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("5".into()), None),
                BoundArg::new(CtValue::String("+".into()), None),
                BoundArg::new(CtValue::String("3".into()), None),
            ],
            ..DataCall::named("expr")
        };

        let intent = ExprIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("expr"),
                OsString::from("5"),
                OsString::from("+"),
                OsString::from("3"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_record() {
        let value = semantic_to_value(&ct_expr::ExprSemantic {
            text: "8".into(),
            truthy: true,
        });

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("text".into(), CtValue::String("8".into())),
                ("truthy".into(), CtValue::Bool(true)),
            ])
        );
    }
}
