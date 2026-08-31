use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdTsort;

struct TsortIntent {
    argv: Vec<OsString>,
}

struct TsortCore;

impl TsortIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("tsort"));

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

impl TsortCore {
    fn run_core(
        intent: &TsortIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "tsort",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &[]),
            || Ok(ct_tsort::tsort_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("tsort: {err}")),
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
    let mut stderr = format!("tsort: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'tsort --help' for more information.\n");
    }
    stderr
}

fn row_to_value(semantic: &ct_tsort::TsortSemantic, row: &ct_tsort::TsortRow) -> CtValue {
    CtValue::Record(vec![
        ("had_cycle".into(), CtValue::Bool(semantic.had_cycle)),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("node".into(), CtValue::String(row.node.clone())),
    ])
}

fn semantic_to_value(semantic: &ct_tsort::TsortSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdTsort {
    fn signature(&self) -> DataSignature {
        DataSignature::new("tsort", "structured topological ordering rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible tsort arguments",
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
        let intent = TsortIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = TsortCore::run_core(&intent, input)?;
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
                source: Some("tsort".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{TsortIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("graph.txt".into()), None)],
            ..DataCall::named("tsort")
        };

        let intent = TsortIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("tsort"), OsString::from("graph.txt")]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_tsort::TsortSemantic {
            had_cycle: true,
            rows: vec![
                ct_tsort::TsortRow {
                    row_index: 1,
                    node: "a".into(),
                },
                ct_tsort::TsortRow {
                    row_index: 2,
                    node: "b".into(),
                },
            ],
            classic_text: "a\nb\n".into(),
            stderr_text: "tsort: cycle\n".into(),
            exit_code: 1,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("had_cycle".into(), CtValue::Bool(true)),
                    ("row_index".into(), CtValue::Int(1)),
                    ("node".into(), CtValue::String("a".into())),
                ]),
                CtValue::Record(vec![
                    ("had_cycle".into(), CtValue::Bool(true)),
                    ("row_index".into(), CtValue::Int(2)),
                    ("node".into(), CtValue::String("b".into())),
                ]),
            ])
        );
    }
}
