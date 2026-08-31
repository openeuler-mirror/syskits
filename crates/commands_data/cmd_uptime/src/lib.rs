use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdUptime;

struct UptimeIntent {
    argv: Vec<OsString>,
}

struct UptimeCore;

impl UptimeIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("uptime"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("uptime: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl UptimeCore {
    fn run_core(
        intent: &UptimeIntent,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_uptime::uptime_native_semantic(intent.argv.iter().cloned())
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

fn semantic_to_value(semantic: &ct_uptime::UptimeSemantic) -> CtValue {
    CtValue::Record(vec![
        (
            "view_kind".into(),
            CtValue::String(semantic.view_kind.clone()),
        ),
        (
            "uptime_source_kind".into(),
            CtValue::String(semantic.uptime_source_kind.clone()),
        ),
        (
            "sample_time_unix".into(),
            CtValue::Int(semantic.sample_time_unix),
        ),
        (
            "sample_time_local".into(),
            CtValue::String(semantic.sample_time_local.clone()),
        ),
        (
            "boot_time_unix".into(),
            optional_i64(semantic.boot_time_unix),
        ),
        (
            "boot_time_local".into(),
            optional_string(&semantic.boot_time_local),
        ),
        (
            "uptime_seconds".into(),
            optional_i64(semantic.uptime_seconds),
        ),
        (
            "uptime_pretty".into(),
            optional_string(&semantic.uptime_pretty),
        ),
        (
            "user_count".into(),
            CtValue::Int(i64::try_from(semantic.user_count).expect("user count fits in i64")),
        ),
        (
            "load_averages".into(),
            CtValue::List(
                semantic
                    .load_averages
                    .iter()
                    .map(|value| CtValue::Float(*value))
                    .collect(),
            ),
        ),
    ])
}

impl DataCommand for CmdUptime {
    fn signature(&self) -> DataSignature {
        DataSignature::new("uptime", "structured uptime snapshot")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible uptime arguments",
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
        let intent = UptimeIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = UptimeCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: Some(stderr_text),
                exit_code,
                source: Some("uptime".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{UptimeIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("--since".into()), None),
                BoundArg::new(CtValue::String("/tmp/fixture".into()), None),
            ],
            ..DataCall::named("uptime")
        };

        let intent = UptimeIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("uptime"),
                OsString::from("--since"),
                OsString::from("/tmp/fixture"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_record_fields() {
        let value = semantic_to_value(&ct_uptime::UptimeSemantic {
            view_kind: "default".into(),
            uptime_source_kind: "boot_time".into(),
            sample_time_unix: 1_700_000_100,
            sample_time_local: "2023-11-14 22:15:00".into(),
            boot_time_unix: Some(1_700_000_000),
            boot_time_local: Some("2023-11-14 22:13:20".into()),
            uptime_seconds: Some(100),
            uptime_pretty: Some("up 1 minute".into()),
            user_count: 2,
            load_averages: vec![0.1, 0.2],
            classic_text: String::new(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("view_kind".into(), CtValue::String("default".into())),
                (
                    "uptime_source_kind".into(),
                    CtValue::String("boot_time".into())
                ),
                ("sample_time_unix".into(), CtValue::Int(1_700_000_100)),
                (
                    "sample_time_local".into(),
                    CtValue::String("2023-11-14 22:15:00".into())
                ),
                ("boot_time_unix".into(), CtValue::Int(1_700_000_000)),
                (
                    "boot_time_local".into(),
                    CtValue::String("2023-11-14 22:13:20".into())
                ),
                ("uptime_seconds".into(), CtValue::Int(100)),
                (
                    "uptime_pretty".into(),
                    CtValue::String("up 1 minute".into())
                ),
                ("user_count".into(), CtValue::Int(2)),
                (
                    "load_averages".into(),
                    CtValue::List(vec![CtValue::Float(0.1), CtValue::Float(0.2)])
                ),
            ])
        );
    }
}
