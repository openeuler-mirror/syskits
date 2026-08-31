use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdMore;

struct MoreIntent {
    argv: Vec<OsString>,
}

struct MoreCore;

impl MoreIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("more"));

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

impl MoreCore {
    fn run_core(
        intent: &MoreIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "more",
            input,
            ctengine::argv_uses_stdin(
                &intent.argv,
                &["--lines", "--number", "--pattern", "--from-line"],
            ),
            || Ok(ct_more::more_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("more: {err}")),
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
    let mut stderr = format!("more: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'more --help' for more information.\n");
    }
    stderr
}

fn optional_string(value: &Option<String>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.clone()),
        None => CtValue::Nothing,
    }
}

fn optional_usize(value: Option<usize>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(i64::try_from(value).expect("line index fits")),
        None => CtValue::Nothing,
    }
}

fn row_to_value(row: &ct_more::MoreSemanticRow) -> CtValue {
    CtValue::Record(vec![
        ("kind".into(), CtValue::String(row.kind.clone())),
        ("file".into(), optional_string(&row.file)),
        ("line_index".into(), optional_usize(row.line_index)),
        ("text".into(), CtValue::String(row.text.clone())),
        ("source".into(), CtValue::String(row.source.clone())),
        ("terminated".into(), CtValue::Bool(row.terminated)),
    ])
}

fn semantic_to_value(semantic: &ct_more::MoreSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

impl DataCommand for CmdMore {
    fn signature(&self) -> DataSignature {
        DataSignature::new("more", "structured more visible-output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible more arguments",
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
        let intent = MoreIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = MoreCore::run_core(&intent, input)?;
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
                source: Some("more".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{MoreIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-n".into()), None),
                BoundArg::new(CtValue::String("2".into()), None),
                BoundArg::new(CtValue::String("sample.txt".into()), None),
            ],
            ..DataCall::named("more")
        };

        let intent = MoreIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("more"),
                OsString::from("-n"),
                OsString::from("2"),
                OsString::from("sample.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_more::MoreSemantic {
            rows: vec![
                ct_more::MoreSemanticRow {
                    kind: "file_header_name".into(),
                    file: Some("sample.txt".into()),
                    line_index: None,
                    text: "sample.txt".into(),
                    source: "stdout".into(),
                    terminated: true,
                },
                ct_more::MoreSemanticRow {
                    kind: "content_line".into(),
                    file: Some("sample.txt".into()),
                    line_index: Some(1),
                    text: "alpha".into(),
                    source: "stdout".into(),
                    terminated: true,
                },
            ],
            classic_text: "::::::::::::::\nsample.txt\n::::::::::::::\nalpha\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("kind".into(), CtValue::String("file_header_name".into())),
                    ("file".into(), CtValue::String("sample.txt".into())),
                    ("line_index".into(), CtValue::Nothing),
                    ("text".into(), CtValue::String("sample.txt".into())),
                    ("source".into(), CtValue::String("stdout".into())),
                    ("terminated".into(), CtValue::Bool(true)),
                ]),
                CtValue::Record(vec![
                    ("kind".into(), CtValue::String("content_line".into())),
                    ("file".into(), CtValue::String("sample.txt".into())),
                    ("line_index".into(), CtValue::Int(1)),
                    ("text".into(), CtValue::String("alpha".into())),
                    ("source".into(), CtValue::String("stdout".into())),
                    ("terminated".into(), CtValue::Bool(true)),
                ]),
            ])
        );
    }
}
