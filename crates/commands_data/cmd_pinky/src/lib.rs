use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdPinky;

struct PinkyIntent {
    argv: Vec<OsString>,
}

struct PinkyCore;

impl PinkyIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("pinky"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("pinky: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl PinkyCore {
    fn run_core(intent: &PinkyIntent) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_pinky::pinky_native_semantic(intent.argv.iter().cloned())
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

fn semantic_to_value(semantic: &ct_pinky::PinkySemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn display_columns() -> CtValue {
    CtValue::List(
        ["user", "full_name", "tty_device", "idle", "login_time", "host"]
            .into_iter()
            .map(|column| CtValue::String(column.into()))
            .collect(),
    )
}

fn row_to_value(row: &ct_pinky::PinkyRow) -> CtValue {
    CtValue::Record(vec![
        ("kind".into(), CtValue::String(row.kind.clone())),
        ("user".into(), CtValue::String(row.user.clone())),
        ("full_name".into(), optional_string(&row.full_name)),
        ("tty_device".into(), optional_string(&row.tty_device)),
        ("mesg".into(), optional_string(&row.mesg)),
        ("idle".into(), optional_string(&row.idle)),
        ("login_time".into(), optional_string(&row.login_time)),
        ("host".into(), optional_string(&row.host)),
        ("home_dir".into(), optional_string(&row.home_dir)),
        ("shell".into(), optional_string(&row.shell)),
        ("project_text".into(), optional_string(&row.project_text)),
        ("plan_text".into(), optional_string(&row.plan_text)),
    ])
}

impl DataCommand for CmdPinky {
    fn signature(&self) -> DataSignature {
        DataSignature::new("pinky", "structured lightweight user information")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible pinky arguments",
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
        let intent = PinkyIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = PinkyCore::run_core(&intent)?;
        let metadata = CtPipelineMetadata {
            classic_text: Some(classic_text),
            classic_bytes: None,
            classic_append_newline: false,
            stderr_text: if stderr_text.is_empty() {
                None
            } else {
                Some(stderr_text)
            },
            exit_code,
            source: Some("pinky".into()),
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
    use super::{PinkyIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-l".into()), None),
                BoundArg::new(CtValue::String("root".into()), None),
            ],
            ..DataCall::named("pinky")
        };

        let intent = PinkyIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("pinky"),
                OsString::from("-l"),
                OsString::from("root"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_pinky::PinkySemantic {
            view_kind: "long".into(),
            rows: vec![ct_pinky::PinkyRow {
                kind: "profile".into(),
                user: "root".into(),
                full_name: Some("Super User".into()),
                tty_device: None,
                mesg: None,
                idle: None,
                login_time: None,
                host: None,
                home_dir: Some("/root".into()),
                shell: Some("/bin/bash".into()),
                project_text: None,
                plan_text: None,
            }],
            classic_text: String::new(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("kind".into(), CtValue::String("profile".into())),
                ("user".into(), CtValue::String("root".into())),
                ("full_name".into(), CtValue::String("Super User".into())),
                ("tty_device".into(), CtValue::Nothing),
                ("mesg".into(), CtValue::Nothing),
                ("idle".into(), CtValue::Nothing),
                ("login_time".into(), CtValue::Nothing),
                ("host".into(), CtValue::Nothing),
                ("home_dir".into(), CtValue::String("/root".into())),
                ("shell".into(), CtValue::String("/bin/bash".into())),
                ("project_text".into(), CtValue::Nothing),
                ("plan_text".into(), CtValue::Nothing),
            ])])
        );
    }

    #[test]
    fn display_columns_focus_on_default_pinky_summary() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![
                CtValue::String("user".into()),
                CtValue::String("full_name".into()),
                CtValue::String("tty_device".into()),
                CtValue::String("idle".into()),
                CtValue::String("login_time".into()),
                CtValue::String("host".into()),
            ])
        );
    }
}
