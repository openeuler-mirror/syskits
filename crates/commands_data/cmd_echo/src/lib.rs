use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdEcho;

struct EchoIntent {
    argv: Vec<OsString>,
}

struct EchoCore;

impl EchoIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("echo"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("echo: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl EchoCore {
    fn run_core(intent: &EchoIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_echo::echo_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        let classic_text = semantic.classic_text.clone();
        Ok((semantic_to_value(&semantic), classic_text))
    }
}

fn semantic_to_value(semantic: &ct_echo::EchoSemantic) -> CtValue {
    CtValue::Record(vec![
        (
            "inputs".into(),
            CtValue::List(
                semantic
                    .inputs
                    .iter()
                    .cloned()
                    .map(CtValue::String)
                    .collect(),
            ),
        ),
        ("text".into(), CtValue::String(semantic.text.clone())),
        (
            "trailing_newline".into(),
            CtValue::Bool(semantic.trailing_newline),
        ),
        (
            "escape_mode".into(),
            CtValue::String(match semantic.escape_mode {
                ct_echo::EchoEscapeMode::Literal => "literal".into(),
                ct_echo::EchoEscapeMode::Interpreted => "interpreted".into(),
            }),
        ),
        (
            "terminated_early".into(),
            CtValue::Bool(semantic.terminated_early),
        ),
    ])
}

fn display_columns() -> CtValue {
    CtValue::List(
        ["text", "trailing_newline"]
            .into_iter()
            .map(|column| CtValue::String(column.into()))
            .collect(),
    )
}

impl DataCommand for CmdEcho {
    fn signature(&self) -> DataSignature {
        DataSignature::new("echo", "structured echo output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible echo arguments",
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
        let intent = EchoIntent::from_call(call)?;
        let (value, classic_text) = EchoCore::run_core(&intent)?;
        let metadata = CtPipelineMetadata {
            classic_text: Some(classic_text),
            classic_bytes: None,
            classic_append_newline: false,
            stderr_text: None,
            exit_code: 0,
            source: Some("echo".into()),
            ..Default::default()
        };
        if let Ok(mut custom) = metadata.custom.lock() {
            custom.insert("display.columns".into(), display_columns());
        }
        Ok(CtPipelineData::Value(value, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::{EchoIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-n".into()), None),
                BoundArg::new(CtValue::String("hello".into()), None),
            ],
            ..DataCall::named("echo")
        };

        let intent = EchoIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("echo"),
                OsString::from("-n"),
                OsString::from("hello"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_record() {
        let value = semantic_to_value(&ct_echo::EchoSemantic {
            inputs: vec!["hello".into(), "world".into()],
            text: "hello world".into(),
            trailing_newline: true,
            escape_mode: ct_echo::EchoEscapeMode::Literal,
            terminated_early: false,
            classic_text: "hello world\n".into(),
        });

        assert_eq!(
            value,
            CtValue::Record(vec![
                (
                    "inputs".into(),
                    CtValue::List(vec![
                        CtValue::String("hello".into()),
                        CtValue::String("world".into()),
                    ]),
                ),
                ("text".into(), CtValue::String("hello world".into())),
                ("trailing_newline".into(), CtValue::Bool(true)),
                ("escape_mode".into(), CtValue::String("literal".into())),
                ("terminated_early".into(), CtValue::Bool(false)),
            ])
        );
    }

    #[test]
    fn display_columns_focus_on_echo_result() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![
                CtValue::String("text".into()),
                CtValue::String("trailing_newline".into()),
            ])
        );
    }
}
