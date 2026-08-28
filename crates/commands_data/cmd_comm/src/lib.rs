use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdComm;

struct CommIntent {
    argv: Vec<OsString>,
}

struct CommCore;

impl CommIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("comm"));

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

impl CommCore {
    fn run_core(
        intent: &CommIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "comm",
            input,
            ctengine::argv_has_stdin_operand(&intent.argv, &["--output-delimiter"]),
            || Ok(ct_comm::comm_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("comm: {err}")),
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
    let mut stderr = format!("comm: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'comm --help' for more information.\n");
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
        Some(value) => CtValue::Int(i64::try_from(value).expect("count fits in i64")),
        None => CtValue::Nothing,
    }
}

fn row_to_value(row: &ct_comm::CommRow) -> CtValue {
    CtValue::Record(vec![
        ("kind".into(), CtValue::String(row.kind.clone())),
        ("line".into(), optional_string(&row.line)),
        ("from_file1".into(), CtValue::Bool(row.from_file1)),
        ("from_file2".into(), CtValue::Bool(row.from_file2)),
        (
            "count_file1_only".into(),
            optional_usize(row.count_file1_only),
        ),
        (
            "count_file2_only".into(),
            optional_usize(row.count_file2_only),
        ),
        ("count_both".into(), optional_usize(row.count_both)),
    ])
}

fn semantic_to_value(semantic: &ct_comm::CommSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

impl DataCommand for CmdComm {
    fn signature(&self) -> DataSignature {
        DataSignature::new("comm", "structured comparison rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible comm arguments",
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
        let intent = CommIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = CommCore::run_core(&intent, input)?;
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
                source: Some("comm".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{CommIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("left.txt".into()), None),
                BoundArg::new(CtValue::String("right.txt".into()), None),
            ],
            ..DataCall::named("comm")
        };

        let intent = CommIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("comm"),
                OsString::from("left.txt"),
                OsString::from("right.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_comm::CommSemantic {
            rows: vec![
                ct_comm::CommRow {
                    kind: "left".into(),
                    line: Some("a".into()),
                    from_file1: true,
                    from_file2: false,
                    count_file1_only: None,
                    count_file2_only: None,
                    count_both: None,
                },
                ct_comm::CommRow {
                    kind: "total".into(),
                    line: None,
                    from_file1: false,
                    from_file2: false,
                    count_file1_only: Some(1),
                    count_file2_only: Some(2),
                    count_both: Some(3),
                },
            ],
            classic_text: "a\n1\t2\t3\ttotal\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("kind".into(), CtValue::String("left".into())),
                    ("line".into(), CtValue::String("a".into())),
                    ("from_file1".into(), CtValue::Bool(true)),
                    ("from_file2".into(), CtValue::Bool(false)),
                    ("count_file1_only".into(), CtValue::Nothing),
                    ("count_file2_only".into(), CtValue::Nothing),
                    ("count_both".into(), CtValue::Nothing),
                ]),
                CtValue::Record(vec![
                    ("kind".into(), CtValue::String("total".into())),
                    ("line".into(), CtValue::Nothing),
                    ("from_file1".into(), CtValue::Bool(false)),
                    ("from_file2".into(), CtValue::Bool(false)),
                    ("count_file1_only".into(), CtValue::Int(1)),
                    ("count_file2_only".into(), CtValue::Int(2)),
                    ("count_both".into(), CtValue::Int(3)),
                ]),
            ])
        );
    }
}
