use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdGroups;

struct GroupsIntent {
    argv: Vec<OsString>,
}

struct GroupsCore;

impl GroupsIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("groups"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("groups: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl GroupsCore {
    fn run_core(
        intent: &GroupsIntent,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_groups::groups_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn semantic_to_value(semantic: &ct_groups::GroupsSemantic) -> CtValue {
    CtValue::List(semantic.entries.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_groups::GroupsEntry) -> CtValue {
    CtValue::Record(vec![
        (
            "user".into(),
            match &row.user {
                Some(user) => CtValue::String(user.clone()),
                None => CtValue::Nothing,
            },
        ),
        (
            "groups".into(),
            CtValue::List(row.groups.iter().cloned().map(CtValue::String).collect()),
        ),
    ])
}

impl DataCommand for CmdGroups {
    fn signature(&self) -> DataSignature {
        DataSignature::new("groups", "structured group membership output")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = GroupsIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = GroupsCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: Some(stderr_text),
                exit_code,
                source: Some("groups".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{GroupsIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("root".into()), None)],
            ..DataCall::named("groups")
        };

        let intent = GroupsIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("groups"), OsString::from("root")]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_groups::GroupsSemantic {
            entries: vec![ct_groups::GroupsEntry {
                user: Some("root".into()),
                groups: vec!["root".into(), "wheel".into()],
            }],
            classic_text: "root : root wheel\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("user".into(), CtValue::String("root".into())),
                (
                    "groups".into(),
                    CtValue::List(vec![
                        CtValue::String("root".into()),
                        CtValue::String("wheel".into()),
                    ]),
                ),
            ])])
        );
    }
}
