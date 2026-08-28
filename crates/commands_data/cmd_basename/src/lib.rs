use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdBasename;

struct BasenameIntent {
    argv: Vec<OsString>,
}

struct BasenameCore;

impl BasenameIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("basename"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple(
                    "basename: argument must be string",
                ));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl BasenameCore {
    fn run_core(intent: &BasenameIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_basename::basename_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((semantic_to_value(&semantic), semantic.classic_text))
    }
}

fn semantic_to_value(semantic: &ct_basename::BasenameSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_basename::BasenameRow) -> CtValue {
    CtValue::Record(vec![
        ("input".into(), CtValue::String(row.input.clone())),
        ("basename".into(), CtValue::String(row.basename.clone())),
    ])
}

impl DataCommand for CmdBasename {
    fn signature(&self) -> DataSignature {
        DataSignature::new("basename", "structured basename extraction")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = BasenameIntent::from_call(call)?;
        let (value, classic_text) = BasenameCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: None,
                exit_code: 0,
                source: Some("basename".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{BasenameIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-s".into()), None),
                BoundArg::new(CtValue::String(".txt".into()), None),
                BoundArg::new(CtValue::String("/tmp/a.txt".into()), None),
            ],
            ..DataCall::named("basename")
        };

        let intent = BasenameIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("basename"),
                OsString::from("-s"),
                OsString::from(".txt"),
                OsString::from("/tmp/a.txt")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_basename::BasenameSemantic {
            rows: vec![ct_basename::BasenameRow {
                input: "/tmp/a.txt".into(),
                basename: "a.txt".into(),
            }],
            classic_text: "a.txt\n".into(),
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("input".into(), CtValue::String("/tmp/a.txt".into())),
                ("basename".into(), CtValue::String("a.txt".into())),
            ])])
        );
    }
}
