use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdCat;

struct CatIntent {
    argv: Vec<OsString>,
}

struct CatCore;

impl CatIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("cat"));

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

impl CatCore {
    fn run_core(
        intent: &CatIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "cat",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &[]),
            || Ok(ct_cat::cat_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("cat: {err}")),
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
    let mut stderr = format!("cat: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'cat --help' for more information.\n");
    }
    stderr
}

fn row_to_value(semantic: &ct_cat::CatSemantic, row: &ct_cat::CatRow) -> CtValue {
    CtValue::Record(vec![
        (
            "number_mode".into(),
            CtValue::String(semantic.number_mode.clone()),
        ),
        (
            "squeeze_blank".into(),
            CtValue::Bool(semantic.squeeze_blank),
        ),
        ("show_tabs".into(), CtValue::Bool(semantic.show_tabs)),
        ("show_ends".into(), CtValue::Bool(semantic.show_ends)),
        (
            "show_non_print".into(),
            CtValue::Bool(semantic.show_non_print),
        ),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
        ("is_blank".into(), CtValue::Bool(row.is_blank)),
    ])
}

fn semantic_to_value(semantic: &ct_cat::CatSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdCat {
    fn signature(&self) -> DataSignature {
        DataSignature::new("cat", "structured cat output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible cat arguments",
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
        let intent = CatIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = CatCore::run_core(&intent, input)?;
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
                source: Some("cat".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{CatIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-n".into()), None),
                BoundArg::new(CtValue::String("file.txt".into()), None),
            ],
            ..DataCall::named("cat")
        };

        let intent = CatIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("cat"),
                OsString::from("-n"),
                OsString::from("file.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_cat::CatSemantic {
            number_mode: "none".into(),
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
            rows: vec![
                ct_cat::CatRow {
                    row_index: 1,
                    line: "alpha".into(),
                    is_blank: false,
                },
                ct_cat::CatRow {
                    row_index: 2,
                    line: String::new(),
                    is_blank: true,
                },
            ],
            classic_text: "alpha\n\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("number_mode".into(), CtValue::String("none".into())),
                    ("squeeze_blank".into(), CtValue::Bool(false)),
                    ("show_tabs".into(), CtValue::Bool(false)),
                    ("show_ends".into(), CtValue::Bool(false)),
                    ("show_non_print".into(), CtValue::Bool(false)),
                    ("row_index".into(), CtValue::Int(1)),
                    ("line".into(), CtValue::String("alpha".into())),
                    ("is_blank".into(), CtValue::Bool(false)),
                ]),
                CtValue::Record(vec![
                    ("number_mode".into(), CtValue::String("none".into())),
                    ("squeeze_blank".into(), CtValue::Bool(false)),
                    ("show_tabs".into(), CtValue::Bool(false)),
                    ("show_ends".into(), CtValue::Bool(false)),
                    ("show_non_print".into(), CtValue::Bool(false)),
                    ("row_index".into(), CtValue::Int(2)),
                    ("line".into(), CtValue::String(String::new())),
                    ("is_blank".into(), CtValue::Bool(true)),
                ]),
            ])
        );
    }
}
