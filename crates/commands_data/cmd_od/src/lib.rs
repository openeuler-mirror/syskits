use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdOd;

struct OdIntent {
    argv: Vec<OsString>,
}

struct OdCore;

impl OdIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("od"));

        for arg in &call.positionals {
            argv.push(OsString::from(value_to_arg(&arg.value)));
        }

        Ok(Self { argv })
    }
}

fn value_to_arg(value: &CtValue) -> String {
    match value {
        CtValue::String(s) => s.clone(),
        other => other.to_text(),
    }
}

impl OdCore {
    fn run_core(
        intent: &OdIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "od",
            input,
            ctengine::argv_uses_stdin(
                &intent.argv,
                &[
                    "-A",
                    "--address-radix",
                    "-j",
                    "--skip-bytes",
                    "-N",
                    "--read-bytes",
                    "-t",
                    "--format",
                    "-w",
                    "--width",
                    "-S",
                    "--strings",
                    "--endian",
                ],
            ),
            || Ok(ct_od::od_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("od: {err}")),
        )?;
        Ok(match result {
            Ok(semantic) => (
                semantic_to_value(&semantic),
                semantic.classic_text,
                semantic.stderr_text,
                semantic.exit_code,
            ),
            Err(err) => (
                CtValue::List(Vec::new()),
                String::new(),
                render_error_text(err.as_ref()),
                err.code(),
            ),
        })
    }
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("od: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'od --help' for more information.\n");
    }
    stderr
}

fn optional_i64(value: Option<u64>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(i64::try_from(value).expect("value fits in i64")),
        None => CtValue::Nothing,
    }
}

fn views_to_value(views: &[ct_od::OdView]) -> CtValue {
    CtValue::List(
        views
            .iter()
            .map(|view| {
                CtValue::Record(vec![
                    ("spec".into(), CtValue::String(view.spec.clone())),
                    ("text".into(), CtValue::String(view.text.clone())),
                ])
            })
            .collect(),
    )
}

fn semantic_to_value(semantic: &ct_od::OdSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_od::OdRow) -> CtValue {
    CtValue::Record(vec![
        ("row_kind".into(), CtValue::String(row.row_kind.clone())),
        (
            "offset".into(),
            CtValue::Int(i64::try_from(row.offset).expect("offset fits in i64")),
        ),
        ("label".into(), optional_i64(row.label)),
        (
            "byte_len".into(),
            CtValue::Int(i64::try_from(row.byte_len).expect("len fits in i64")),
        ),
        (
            "bytes".into(),
            CtValue::List(
                row.bytes
                    .iter()
                    .map(|b| CtValue::Int(i64::from(*b)))
                    .collect(),
            ),
        ),
        ("views".into(), views_to_value(&row.views)),
    ])
}

impl DataCommand for CmdOd {
    fn signature(&self) -> DataSignature {
        DataSignature::new("od", "structured octal dump rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible od arguments",
                CtType::Any,
            ))
            .input(CtType::Any)
            .output(CtType::List)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = OdIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = OdCore::run_core(&intent, input)?;
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
                source: Some("od".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{OdIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-t".into()), None),
                BoundArg::new(CtValue::String("x1".into()), None),
                BoundArg::new(CtValue::String("sample.bin".into()), None),
            ],
            ..DataCall::named("od")
        };

        let intent = OdIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("od"),
                OsString::from("-t"),
                OsString::from("x1"),
                OsString::from("sample.bin"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_od::OdSemantic {
            rows: vec![ct_od::OdRow {
                row_kind: "data".into(),
                offset: 0,
                label: None,
                byte_len: 3,
                bytes: vec![65, 66, 67],
                views: vec![ct_od::OdView {
                    spec: "int_0".into(),
                    text: "41 42 43".into(),
                }],
            }],
            classic_text: String::new(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("row_kind".into(), CtValue::String("data".into())),
                ("offset".into(), CtValue::Int(0)),
                ("label".into(), CtValue::Nothing),
                ("byte_len".into(), CtValue::Int(3)),
                (
                    "bytes".into(),
                    CtValue::List(vec![CtValue::Int(65), CtValue::Int(66), CtValue::Int(67)])
                ),
                (
                    "views".into(),
                    CtValue::List(vec![CtValue::Record(vec![
                        ("spec".into(), CtValue::String("int_0".into())),
                        ("text".into(), CtValue::String("41 42 43".into())),
                    ])])
                ),
            ])])
        );
    }
}
