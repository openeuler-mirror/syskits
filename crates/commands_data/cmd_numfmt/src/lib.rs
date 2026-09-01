use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdNumfmt;

struct NumfmtIntent {
    argv: Vec<OsString>,
}

struct NumfmtCore;

impl NumfmtIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("numfmt"));

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

impl NumfmtCore {
    fn run_core(
        intent: &NumfmtIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "numfmt",
            input,
            ctengine::argv_uses_stdin(
                &intent.argv,
                &[
                    "-d",
                    "--delimiter",
                    "--field",
                    "--format",
                    "--from",
                    "--from-unit",
                    "--to",
                    "--to-unit",
                    "--padding",
                    "--header",
                    "--round",
                    "--suffix",
                    "--invalid",
                ],
            ),
            || {
                Ok(ct_numfmt::numfmt_native_semantic(
                    intent.argv.iter().cloned(),
                ))
            },
            |err| CtDiagnosticError::simple(format!("numfmt: {err}")),
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
    let mut stderr = format!("numfmt: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'numfmt --help' for more information.\n");
    }
    stderr
}

fn semantic_to_value(semantic: &ct_numfmt::NumfmtSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn display_columns() -> CtValue {
    CtValue::List(
        ["input", "output", "status", "error"]
            .into_iter()
            .map(|column| CtValue::String(column.into()))
            .collect(),
    )
}

fn row_to_value(row: &ct_numfmt::NumfmtRow) -> CtValue {
    CtValue::Record(vec![
        (
            "index".into(),
            CtValue::Int(i64::try_from(row.index).expect("index fits in i64")),
        ),
        ("source".into(), CtValue::String(row.source.clone())),
        ("input".into(), CtValue::String(row.input.clone())),
        ("output".into(), CtValue::String(row.output.clone())),
        ("status".into(), CtValue::String(row.status.clone())),
        (
            "error".into(),
            match &row.error {
                Some(error) => CtValue::String(error.clone()),
                None => CtValue::Nothing,
            },
        ),
        (
            "transform_from".into(),
            CtValue::String(row.transform_from.clone()),
        ),
        (
            "transform_to".into(),
            CtValue::String(row.transform_to.clone()),
        ),
        (
            "invalid_mode".into(),
            CtValue::String(row.invalid_mode.clone()),
        ),
        ("zero_terminated".into(), CtValue::Bool(row.zero_terminated)),
    ])
}

impl DataCommand for CmdNumfmt {
    fn signature(&self) -> DataSignature {
        DataSignature::new("numfmt", "structured number formatting output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible numfmt arguments",
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
        let intent = NumfmtIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = NumfmtCore::run_core(&intent, input)?;
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
            source: Some("numfmt".into()),
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
    use super::{NumfmtIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("--to=si".into()), None),
                BoundArg::new(CtValue::String("1000".into()), None),
            ],
            ..DataCall::named("numfmt")
        };

        let intent = NumfmtIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("numfmt"),
                OsString::from("--to=si"),
                OsString::from("1000"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_numfmt::NumfmtSemantic {
            rows: vec![ct_numfmt::NumfmtRow {
                index: 1,
                source: "arg".into(),
                input: "1000".into(),
                output: "1.0K".into(),
                status: "formatted".into(),
                error: None,
                transform_from: "none".into(),
                transform_to: "si".into(),
                invalid_mode: "abort".into(),
                zero_terminated: false,
            }],
            classic_text: "1.0K\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("index".into(), CtValue::Int(1)),
                ("source".into(), CtValue::String("arg".into())),
                ("input".into(), CtValue::String("1000".into())),
                ("output".into(), CtValue::String("1.0K".into())),
                ("status".into(), CtValue::String("formatted".into())),
                ("error".into(), CtValue::Nothing),
                ("transform_from".into(), CtValue::String("none".into())),
                ("transform_to".into(), CtValue::String("si".into())),
                ("invalid_mode".into(), CtValue::String("abort".into())),
                ("zero_terminated".into(), CtValue::Bool(false)),
            ])])
        );
    }

    #[test]
    fn display_columns_focus_on_numfmt_result() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![
                CtValue::String("input".into()),
                CtValue::String("output".into()),
                CtValue::String("status".into()),
                CtValue::String("error".into()),
            ])
        );
    }
}
