use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdExpand;

struct ExpandIntent {
    argv: Vec<OsString>,
}

struct ExpandCore;

impl ExpandIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("expand"));

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

impl ExpandCore {
    fn run_core(
        intent: &ExpandIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "expand",
            input,
            argv_uses_stdin(&intent.argv, &["-t", "--tabs"]),
            || {
                Ok(ct_expand::expand_native_semantic(
                    intent.argv.iter().cloned(),
                ))
            },
            |err| CtDiagnosticError::simple(format!("expand: {err}")),
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

fn argv_uses_stdin(argv: &[OsString], value_flags: &[&str]) -> bool {
    ctengine::argv_uses_stdin(argv, value_flags)
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("expand: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'expand --help' for more information.\n");
    }
    stderr
}

fn tabstop_mode_name(mode: ct_expand::ExpandTabstopMode) -> &'static str {
    match mode {
        ct_expand::ExpandTabstopMode::None => "none",
        ct_expand::ExpandTabstopMode::Slash => "slash",
        ct_expand::ExpandTabstopMode::Plus => "plus",
    }
}

fn row_to_value(semantic: &ct_expand::ExpandSemantic, row: &ct_expand::ExpandRow) -> CtValue {
    CtValue::Record(vec![
        (
            "tabstop_mode".into(),
            CtValue::String(tabstop_mode_name(semantic.tabstop_mode).into()),
        ),
        (
            "tabstops".into(),
            CtValue::List(
                semantic
                    .tabstops
                    .iter()
                    .map(|tabstop| CtValue::Int(i64::try_from(*tabstop).expect("tabstop fits")))
                    .collect(),
            ),
        ),
        ("initial_only".into(), CtValue::Bool(semantic.initial_only)),
        ("assume_utf8".into(), CtValue::Bool(semantic.assume_utf8)),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
        ("had_tabs".into(), CtValue::Bool(row.had_tabs)),
    ])
}

fn semantic_to_value(semantic: &ct_expand::ExpandSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdExpand {
    fn signature(&self) -> DataSignature {
        DataSignature::new("expand", "structured expanded line rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible expand arguments",
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
        let intent = ExpandIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = ExpandCore::run_core(&intent, input)?;
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
                source: Some("expand".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ExpandIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-t".into()), None),
                BoundArg::new(CtValue::String("4".into()), None),
                BoundArg::new(CtValue::String("file.txt".into()), None),
            ],
            ..DataCall::named("expand")
        };

        let intent = ExpandIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("expand"),
                OsString::from("-t"),
                OsString::from("4"),
                OsString::from("file.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_expand::ExpandSemantic {
            tabstop_mode: ct_expand::ExpandTabstopMode::None,
            tabstops: vec![4],
            initial_only: false,
            assume_utf8: true,
            rows: vec![
                ct_expand::ExpandRow {
                    row_index: 1,
                    line: "a   b".into(),
                    had_tabs: true,
                },
                ct_expand::ExpandRow {
                    row_index: 2,
                    line: "    c".into(),
                    had_tabs: true,
                },
            ],
            classic_text: "a   b\n    c\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("tabstop_mode".into(), CtValue::String("none".into())),
                    ("tabstops".into(), CtValue::List(vec![CtValue::Int(4)]),),
                    ("initial_only".into(), CtValue::Bool(false)),
                    ("assume_utf8".into(), CtValue::Bool(true)),
                    ("row_index".into(), CtValue::Int(1)),
                    ("line".into(), CtValue::String("a   b".into())),
                    ("had_tabs".into(), CtValue::Bool(true)),
                ]),
                CtValue::Record(vec![
                    ("tabstop_mode".into(), CtValue::String("none".into())),
                    ("tabstops".into(), CtValue::List(vec![CtValue::Int(4)]),),
                    ("initial_only".into(), CtValue::Bool(false)),
                    ("assume_utf8".into(), CtValue::Bool(true)),
                    ("row_index".into(), CtValue::Int(2)),
                    ("line".into(), CtValue::String("    c".into())),
                    ("had_tabs".into(), CtValue::Bool(true)),
                ]),
            ])
        );
    }
}
