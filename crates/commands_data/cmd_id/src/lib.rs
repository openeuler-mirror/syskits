use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdId;

struct IdIntent {
    argv: Vec<OsString>,
}

struct IdCore;

impl IdIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("id"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("id: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl IdCore {
    fn run_core(intent: &IdIntent) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_id::id_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn semantic_to_value(semantic: &ct_id::IdSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn entity_to_value(entity: &ct_id::IdEntity) -> CtValue {
    CtValue::Record(vec![
        ("id".into(), CtValue::Int(i64::from(entity.id))),
        ("label".into(), CtValue::String(entity.label.clone())),
    ])
}

fn row_to_value(row: &ct_id::IdRow) -> CtValue {
    let mut fields = vec![("mode".into(), CtValue::String(row.mode.clone()))];

    if let Some(subject) = &row.subject {
        fields.push(("subject".into(), CtValue::String(subject.clone())));
    }
    if let Some(uid) = &row.uid {
        fields.push(("uid".into(), entity_to_value(uid)));
    }
    if let Some(gid) = &row.gid {
        fields.push(("gid".into(), entity_to_value(gid)));
    }
    if let Some(euid) = &row.euid {
        fields.push(("euid".into(), entity_to_value(euid)));
    }
    if let Some(egid) = &row.egid {
        fields.push(("egid".into(), entity_to_value(egid)));
    }
    if !row.groups.is_empty() {
        fields.push((
            "groups".into(),
            CtValue::List(row.groups.iter().map(entity_to_value).collect()),
        ));
    }
    if let Some(context) = &row.context {
        fields.push(("context".into(), CtValue::String(context.clone())));
    }

    CtValue::Record(fields)
}

impl DataCommand for CmdId {
    fn signature(&self) -> DataSignature {
        DataSignature::new("id", "structured identity output")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = IdIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = IdCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: Some(stderr_text),
                exit_code,
                source: Some("id".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{IdIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("-u".into()), None)],
            ..DataCall::named("id")
        };

        let intent = IdIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("id"), OsString::from("-u")]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_id::IdSemantic {
            rows: vec![ct_id::IdRow {
                mode: "default".into(),
                subject: None,
                uid: Some(ct_id::IdEntity {
                    id: 0,
                    label: "root".into(),
                }),
                gid: Some(ct_id::IdEntity {
                    id: 0,
                    label: "root".into(),
                }),
                euid: None,
                egid: None,
                groups: vec![ct_id::IdEntity {
                    id: 0,
                    label: "root".into(),
                }],
                context: None,
            }],
            classic_text: "uid=0(root) gid=0(root) groups=0(root)\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("mode".into(), CtValue::String("default".into())),
                (
                    "uid".into(),
                    CtValue::Record(vec![
                        ("id".into(), CtValue::Int(0)),
                        ("label".into(), CtValue::String("root".into())),
                    ]),
                ),
                (
                    "gid".into(),
                    CtValue::Record(vec![
                        ("id".into(), CtValue::Int(0)),
                        ("label".into(), CtValue::String("root".into())),
                    ]),
                ),
                (
                    "groups".into(),
                    CtValue::List(vec![CtValue::Record(vec![
                        ("id".into(), CtValue::Int(0)),
                        ("label".into(), CtValue::String("root".into())),
                    ])]),
                ),
            ])])
        );
    }
}
