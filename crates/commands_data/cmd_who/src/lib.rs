use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdWho;

struct WhoIntent {
    argv: Vec<OsString>,
}

struct WhoCore;

impl WhoIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("who"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("who: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl WhoCore {
    fn run_core(intent: &WhoIntent) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_who::who_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn optional_string(value: &Option<String>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.clone()),
        None => CtValue::Nothing,
    }
}

fn optional_i64(value: Option<i64>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(value),
        None => CtValue::Nothing,
    }
}

fn semantic_to_value(semantic: &ct_who::WhoSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_who::WhoRow) -> CtValue {
    CtValue::Record(vec![
        ("kind".into(), CtValue::String(row.kind.clone())),
        ("user".into(), optional_string(&row.user)),
        ("mesg".into(), optional_string(&row.mesg)),
        ("line".into(), optional_string(&row.line)),
        ("time".into(), optional_string(&row.time)),
        ("idle".into(), optional_string(&row.idle)),
        ("pid".into(), optional_i64(row.pid)),
        ("host".into(), optional_string(&row.host)),
        ("comment".into(), optional_string(&row.comment)),
        ("exit".into(), optional_string(&row.exit)),
        (
            "user_names".into(),
            CtValue::List(
                row.user_names
                    .iter()
                    .cloned()
                    .map(CtValue::String)
                    .collect(),
            ),
        ),
        (
            "user_count".into(),
            match row.user_count {
                Some(value) => CtValue::Int(i64::try_from(value).expect("count fits in i64")),
                None => CtValue::Nothing,
            },
        ),
    ])
}

impl DataCommand for CmdWho {
    fn signature(&self) -> DataSignature {
        DataSignature::new("who", "structured utmp session listing")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible who arguments",
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
        let intent = WhoIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = WhoCore::run_core(&intent)?;
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
                source: Some("who".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{WhoIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-q".into()), None),
                BoundArg::new(CtValue::String("/tmp/fixture".into()), None),
            ],
            ..DataCall::named("who")
        };

        let intent = WhoIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("who"),
                OsString::from("-q"),
                OsString::from("/tmp/fixture"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_who::WhoSemantic {
            view_kind: "default".into(),
            source_file: "/tmp/fixture".into(),
            rows: vec![ct_who::WhoRow {
                kind: "user".into(),
                user: Some("alice".into()),
                mesg: Some("+".into()),
                line: Some("pts/1".into()),
                time: Some("Apr 7 14:23".into()),
                idle: Some("  .  ".into()),
                pid: Some(42),
                host: Some("host-a".into()),
                comment: Some("(host-a)".into()),
                exit: None,
                user_names: Vec::new(),
                user_count: None,
            }],
            classic_text: String::new(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("kind".into(), CtValue::String("user".into())),
                ("user".into(), CtValue::String("alice".into())),
                ("mesg".into(), CtValue::String("+".into())),
                ("line".into(), CtValue::String("pts/1".into())),
                ("time".into(), CtValue::String("Apr 7 14:23".into())),
                ("idle".into(), CtValue::String("  .  ".into())),
                ("pid".into(), CtValue::Int(42)),
                ("host".into(), CtValue::String("host-a".into())),
                ("comment".into(), CtValue::String("(host-a)".into())),
                ("exit".into(), CtValue::Nothing),
                ("user_names".into(), CtValue::List(vec![])),
                ("user_count".into(), CtValue::Nothing),
            ])])
        );
    }
}
