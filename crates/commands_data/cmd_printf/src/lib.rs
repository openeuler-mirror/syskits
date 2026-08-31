use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdPrintf;

struct PrintfIntent {
    argv: Vec<OsString>,
}

struct PrintfCore;

impl PrintfIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("printf"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("printf: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl PrintfCore {
    fn run_core(
        intent: &PrintfIntent,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_printf::printf_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn row_to_value(row: &ct_printf::PrintfSemanticRow) -> CtValue {
    CtValue::Record(vec![
        (
            "line_index".into(),
            CtValue::Int(i64::try_from(row.line_index).expect("line index fits")),
        ),
        ("text".into(), CtValue::String(row.text.clone())),
        (
            "byte_len".into(),
            CtValue::Int(i64::try_from(row.byte_len).expect("byte len fits")),
        ),
        ("terminated".into(), CtValue::Bool(row.terminated)),
        (
            "format_string".into(),
            CtValue::String(row.format_string.clone()),
        ),
    ])
}

fn semantic_to_value(semantic: &ct_printf::PrintfSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

impl DataCommand for CmdPrintf {
    fn signature(&self) -> DataSignature {
        DataSignature::new("printf", "structured printf visible-output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible printf arguments",
                CtType::Any,
            ))
            .input(CtType::Nothing)
            .output(CtType::List)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = PrintfIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = PrintfCore::run_core(&intent)?;
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
                source: Some("printf".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PrintfIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("%s".into()), None),
                BoundArg::new(CtValue::String("alpha".into()), None),
            ],
            ..DataCall::named("printf")
        };

        let intent = PrintfIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("printf"),
                OsString::from("%s"),
                OsString::from("alpha"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_printf::PrintfSemantic {
            rows: vec![ct_printf::PrintfSemanticRow {
                line_index: 1,
                text: "alpha".into(),
                byte_len: 5,
                terminated: false,
                format_string: "%s".into(),
            }],
            classic_text: "alpha".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("line_index".into(), CtValue::Int(1)),
                ("text".into(), CtValue::String("alpha".into())),
                ("byte_len".into(), CtValue::Int(5)),
                ("terminated".into(), CtValue::Bool(false)),
                ("format_string".into(), CtValue::String("%s".into())),
            ])])
        );
    }
}
