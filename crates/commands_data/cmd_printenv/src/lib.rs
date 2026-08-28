use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdPrintenv;

struct PrintenvIntent {
    argv: Vec<OsString>,
}

struct PrintenvCore;

impl PrintenvIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("printenv"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple(
                    "printenv: argument must be string",
                ));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl PrintenvCore {
    fn run_core(intent: &PrintenvIntent) -> Result<(CtValue, String, i32), CtDiagnosticError> {
        let semantic = ct_printenv::printenv_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.exit_code,
        ))
    }
}

fn semantic_to_value(semantic: &ct_printenv::PrintenvSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_printenv::PrintenvRow) -> CtValue {
    CtValue::Record(vec![
        ("name".into(), CtValue::String(row.name.clone())),
        ("value".into(), CtValue::String(row.value.clone())),
    ])
}

impl DataCommand for CmdPrintenv {
    fn signature(&self) -> DataSignature {
        DataSignature::new("printenv", "structured environment variable output")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = PrintenvIntent::from_call(call)?;
        let (value, classic_text, exit_code) = PrintenvCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: None,
                exit_code,
                source: Some("printenv".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PrintenvIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-0".into()), None),
                BoundArg::new(CtValue::String("HOME".into()), None),
            ],
            ..DataCall::named("printenv")
        };

        let intent = PrintenvIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("printenv"),
                OsString::from("-0"),
                OsString::from("HOME")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_printenv::PrintenvSemantic {
            rows: vec![ct_printenv::PrintenvRow {
                name: "HOME".into(),
                value: "/root".into(),
            }],
            classic_text: "/root\n".into(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("name".into(), CtValue::String("HOME".into())),
                ("value".into(), CtValue::String("/root".into())),
            ])])
        );
    }
}
