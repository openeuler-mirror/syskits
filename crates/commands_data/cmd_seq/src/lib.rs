use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdSeq;

struct SeqIntent {
    argv: Vec<OsString>,
}

struct SeqCore;

impl SeqIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("seq"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("seq: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl SeqCore {
    fn run_core(intent: &SeqIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_seq::seq_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((semantic_to_value(&semantic), semantic.classic_text))
    }
}

fn semantic_to_value(semantic: &ct_seq::SeqSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_seq::SeqRow) -> CtValue {
    CtValue::Record(vec![
        (
            "index".into(),
            CtValue::Int(i64::try_from(row.index).expect("index fits in i64")),
        ),
        ("value".into(), CtValue::String(row.value.clone())),
    ])
}

impl DataCommand for CmdSeq {
    fn signature(&self) -> DataSignature {
        DataSignature::new("seq", "structured numeric sequence output")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = SeqIntent::from_call(call)?;
        let (value, classic_text) = SeqCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: None,
                exit_code: 0,
                source: Some("seq".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{SeqIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-s".into()), None),
                BoundArg::new(CtValue::String(",".into()), None),
                BoundArg::new(CtValue::String("1".into()), None),
                BoundArg::new(CtValue::String("3".into()), None),
            ],
            ..DataCall::named("seq")
        };

        let intent = SeqIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("seq"),
                OsString::from("-s"),
                OsString::from(","),
                OsString::from("1"),
                OsString::from("3"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_seq::SeqSemantic {
            rows: vec![
                ct_seq::SeqRow {
                    index: 0,
                    value: "1".into(),
                },
                ct_seq::SeqRow {
                    index: 1,
                    value: "2".into(),
                },
            ],
            classic_text: "1\n2\n".into(),
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("index".into(), CtValue::Int(0)),
                    ("value".into(), CtValue::String("1".into())),
                ]),
                CtValue::Record(vec![
                    ("index".into(), CtValue::Int(1)),
                    ("value".into(), CtValue::String("2".into())),
                ]),
            ])
        );
    }
}
