use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdUsers;

struct UsersIntent {
    argv: Vec<OsString>,
}

struct UsersCore;

impl UsersIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("users"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("users: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl UsersCore {
    fn run_core(intent: &UsersIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_users::users_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((semantic_to_value(&semantic), semantic.classic_text))
    }
}

fn semantic_to_value(semantic: &ct_users::UsersSemantic) -> CtValue {
    CtValue::List(semantic.sessions.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_users::UsersSession) -> CtValue {
    CtValue::Record(vec![
        ("user".into(), CtValue::String(row.user.clone())),
        ("tty_device".into(), CtValue::String(row.tty_device.clone())),
        ("host".into(), CtValue::String(row.host.clone())),
    ])
}

impl DataCommand for CmdUsers {
    fn signature(&self) -> DataSignature {
        DataSignature::new("users", "structured logged-in user sessions")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible users arguments",
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
        let intent = UsersIntent::from_call(call)?;
        let (value, classic_text) = UsersCore::run_core(&intent)?;
        let classic_append_newline = !classic_text.is_empty();
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline,
                stderr_text: None,
                exit_code: 0,
                source: Some("users".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{UsersIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(
                CtValue::String("/tmp/users.utmp".into()),
                None,
            )],
            ..DataCall::named("users")
        };

        let intent = UsersIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("users"), OsString::from("/tmp/users.utmp")]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_users::UsersSemantic {
            sessions: vec![ct_users::UsersSession {
                user: "alpha".into(),
                tty_device: "pts/1".into(),
                host: "remote-a".into(),
            }],
            classic_text: "alpha".into(),
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("user".into(), CtValue::String("alpha".into())),
                ("tty_device".into(), CtValue::String("pts/1".into())),
                ("host".into(), CtValue::String("remote-a".into())),
            ])])
        );
    }
}
