use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdDirname;

struct DirnameIntent {
    argv: Vec<OsString>,
}

struct DirnameCore;

impl DirnameIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("dirname"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple(
                    "dirname: argument must be string",
                ));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl DirnameCore {
    fn run_core(intent: &DirnameIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_dirname::dirname_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((semantic_to_value(&semantic), semantic.classic_text))
    }
}

fn semantic_to_value(semantic: &ct_dirname::DirnameSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_dirname::DirnameRow) -> CtValue {
    CtValue::Record(vec![
        ("input".into(), CtValue::String(row.input.clone())),
        (
            "directory_path".into(),
            CtValue::String(row.directory_path.clone()),
        ),
    ])
}

impl DataCommand for CmdDirname {
    fn signature(&self) -> DataSignature {
        DataSignature::new("dirname", "structured parent directory extraction")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = DirnameIntent::from_call(call)?;
        let (value, classic_text) = DirnameCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: None,
                exit_code: 0,
                source: Some("dirname".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{DirnameIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-z".into()), None),
                BoundArg::new(CtValue::String("/tmp/a".into()), None),
            ],
            ..DataCall::named("dirname")
        };

        let intent = DirnameIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("dirname"),
                OsString::from("-z"),
                OsString::from("/tmp/a")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_dirname::DirnameSemantic {
            rows: vec![ct_dirname::DirnameRow {
                input: "/tmp/a".into(),
                directory_path: "/tmp".into(),
            }],
            classic_text: "/tmp\n".into(),
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("input".into(), CtValue::String("/tmp/a".into())),
                ("directory_path".into(), CtValue::String("/tmp".into())),
            ])])
        );
    }
}
