use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdSum;

struct SumIntent {
    argv: Vec<OsString>,
}

struct SumCore;

impl SumIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("sum"));

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

impl SumCore {
    fn run_core(
        intent: &SumIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "sum",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &[]),
            || Ok(ct_sum::sum_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("sum: {err}")),
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
    let mut stderr = format!("sum: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'sum --help' for more information.\n");
    }
    stderr
}

fn semantic_to_value(semantic: &ct_sum::SumSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_sum::SumRow) -> CtValue {
    CtValue::Record(vec![
        ("algorithm".into(), CtValue::String(row.algorithm.clone())),
        ("checksum".into(), CtValue::String(row.checksum.to_string())),
        (
            "blocks".into(),
            CtValue::Int(i64::try_from(row.blocks).expect("blocks fit in i64")),
        ),
        (
            "file".into(),
            match &row.file {
                Some(file) => CtValue::String(file.clone()),
                None => CtValue::Nothing,
            },
        ),
    ])
}

impl DataCommand for CmdSum {
    fn signature(&self) -> DataSignature {
        DataSignature::new("sum", "structured checksum output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible sum arguments",
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
        let intent = SumIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = SumCore::run_core(&intent, input)?;
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
                source: Some("sum".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{SumIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("--sysv".into()), None),
                BoundArg::new(CtValue::String("a.txt".into()), None),
            ],
            ..DataCall::named("sum")
        };

        let intent = SumIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("sum"),
                OsString::from("--sysv"),
                OsString::from("a.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_sum::SumSemantic {
            rows: vec![ct_sum::SumRow {
                algorithm: "bsd".into(),
                checksum: 16556,
                blocks: 1,
                file: Some("sample.txt".into()),
            }],
            classic_text: "16556     1 sample.txt\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("algorithm".into(), CtValue::String("bsd".into())),
                ("checksum".into(), CtValue::String("16556".into())),
                ("blocks".into(), CtValue::Int(1)),
                ("file".into(), CtValue::String("sample.txt".into())),
            ])])
        );
    }
}
