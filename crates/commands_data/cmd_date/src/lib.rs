use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdDate;

struct DateIntent {
    argv: Vec<OsString>,
}

struct DateCore;

impl DateIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("date"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("date: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl DateCore {
    fn run_core(intent: &DateIntent) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ct_date::date_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn row_to_value(row: &ct_date::DateRow) -> CtValue {
    CtValue::Record(vec![
        (
            "source_kind".into(),
            CtValue::String(row.source_kind.clone()),
        ),
        (
            "format_kind".into(),
            CtValue::String(row.format_kind.clone()),
        ),
        ("formatted".into(), CtValue::String(row.formatted.clone())),
        ("unix_seconds".into(), CtValue::Int(row.unix_seconds)),
        ("unix_nanos".into(), CtValue::Int(i64::from(row.unix_nanos))),
        (
            "timezone_offset".into(),
            CtValue::String(row.timezone_offset.clone()),
        ),
        (
            "timezone_name".into(),
            CtValue::String(row.timezone_name.clone()),
        ),
        ("year".into(), CtValue::Int(i64::from(row.year))),
        ("month".into(), CtValue::Int(i64::from(row.month))),
        ("day".into(), CtValue::Int(i64::from(row.day))),
        ("hour".into(), CtValue::Int(i64::from(row.hour))),
        ("minute".into(), CtValue::Int(i64::from(row.minute))),
        ("second".into(), CtValue::Int(i64::from(row.second))),
        ("nanosecond".into(), CtValue::Int(i64::from(row.nanosecond))),
    ])
}

fn semantic_to_value(semantic: &ct_date::DateSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

impl DataCommand for CmdDate {
    fn signature(&self) -> DataSignature {
        DataSignature::new("date", "structured date and time output")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = DateIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = DateCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: Some(stderr_text),
                exit_code,
                source: Some("date".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{DateIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-u".into()), None),
                BoundArg::new(CtValue::String("-d".into()), None),
            ],
            ..DataCall::named("date")
        };

        let intent = DateIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("date"),
                OsString::from("-u"),
                OsString::from("-d"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_date::DateSemantic {
            rows: vec![ct_date::DateRow {
                source_kind: "custom".into(),
                format_kind: "default".into(),
                formatted: "Fri Aug 15 12:34:56 UTC 2025".into(),
                unix_seconds: 1755261296,
                unix_nanos: 0,
                timezone_offset: "+00:00".into(),
                timezone_name: "UTC".into(),
                year: 2025,
                month: 8,
                day: 15,
                hour: 12,
                minute: 34,
                second: 56,
                nanosecond: 0,
            }],
            classic_text: "Fri Aug 15 12:34:56 UTC 2025\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("source_kind".into(), CtValue::String("custom".into())),
                ("format_kind".into(), CtValue::String("default".into())),
                (
                    "formatted".into(),
                    CtValue::String("Fri Aug 15 12:34:56 UTC 2025".into())
                ),
                ("unix_seconds".into(), CtValue::Int(1755261296)),
                ("unix_nanos".into(), CtValue::Int(0)),
                ("timezone_offset".into(), CtValue::String("+00:00".into())),
                ("timezone_name".into(), CtValue::String("UTC".into())),
                ("year".into(), CtValue::Int(2025)),
                ("month".into(), CtValue::Int(8)),
                ("day".into(), CtValue::Int(15)),
                ("hour".into(), CtValue::Int(12)),
                ("minute".into(), CtValue::Int(34)),
                ("second".into(), CtValue::Int(56)),
                ("nanosecond".into(), CtValue::Int(0)),
            ])])
        );
    }
}
