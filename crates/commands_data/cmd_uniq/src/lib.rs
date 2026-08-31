use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdUniq;

struct UniqIntent {
    argv: Vec<OsString>,
}

struct UniqCore;

impl UniqIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("uniq"));

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

impl UniqCore {
    fn run_core(
        intent: &UniqIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "uniq",
            input,
            ctengine::argv_uses_stdin(
                &intent.argv,
                &[
                    "-f",
                    "--skip-fields",
                    "-s",
                    "--skip-chars",
                    "-w",
                    "--check-chars",
                ],
            ),
            || Ok(ct_uniq::uniq_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("uniq: {err}")),
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
    let mut stderr = format!("uniq: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'uniq --help' for more information.\n");
    }
    stderr
}

fn opt_usize_to_value(value: Option<usize>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(i64::try_from(value).expect("usize fits in i64")),
        None => CtValue::Nothing,
    }
}

fn row_to_value(semantic: &ct_uniq::UniqSemantic, row: &ct_uniq::UniqRow) -> CtValue {
    CtValue::Record(vec![
        (
            "delimiter_mode".into(),
            CtValue::String(semantic.delimiter_mode.clone()),
        ),
        ("show_counts".into(), CtValue::Bool(semantic.show_counts)),
        (
            "repeated_only".into(),
            CtValue::Bool(semantic.repeated_only),
        ),
        ("unique_only".into(), CtValue::Bool(semantic.unique_only)),
        ("all_repeated".into(), CtValue::Bool(semantic.all_repeated)),
        ("ignore_case".into(), CtValue::Bool(semantic.ignore_case)),
        (
            "zero_terminated".into(),
            CtValue::Bool(semantic.zero_terminated),
        ),
        (
            "skip_fields".into(),
            opt_usize_to_value(semantic.skip_fields),
        ),
        ("skip_chars".into(), opt_usize_to_value(semantic.skip_chars)),
        (
            "check_chars".into(),
            opt_usize_to_value(semantic.check_chars),
        ),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        (
            "group_index".into(),
            CtValue::Int(i64::try_from(row.group_index).expect("group index fits")),
        ),
        (
            "occurrence_index".into(),
            CtValue::Int(i64::try_from(row.occurrence_index).expect("occurrence index fits")),
        ),
        (
            "count".into(),
            CtValue::Int(i64::try_from(row.count).expect("count fits")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
        ("is_repeated".into(), CtValue::Bool(row.is_repeated)),
        ("is_unique".into(), CtValue::Bool(row.is_unique)),
    ])
}

fn semantic_to_value(semantic: &ct_uniq::UniqSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdUniq {
    fn signature(&self) -> DataSignature {
        DataSignature::new("uniq", "structured uniq output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible uniq arguments",
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
        let intent = UniqIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = UniqCore::run_core(&intent, input)?;
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
                source: Some("uniq".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{UniqIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-c".into()), None),
                BoundArg::new(CtValue::String("input.txt".into()), None),
            ],
            ..DataCall::named("uniq")
        };

        let intent = UniqIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("uniq"),
                OsString::from("-c"),
                OsString::from("input.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_uniq::UniqSemantic {
            delimiter_mode: "none".into(),
            show_counts: false,
            repeated_only: false,
            unique_only: false,
            all_repeated: false,
            ignore_case: false,
            zero_terminated: false,
            skip_fields: None,
            skip_chars: None,
            check_chars: Some(3),
            rows: vec![
                ct_uniq::UniqRow {
                    row_index: 1,
                    group_index: 1,
                    occurrence_index: 1,
                    count: 2,
                    line: "alpha".into(),
                    is_repeated: true,
                    is_unique: false,
                },
                ct_uniq::UniqRow {
                    row_index: 2,
                    group_index: 2,
                    occurrence_index: 1,
                    count: 1,
                    line: "beta".into(),
                    is_repeated: false,
                    is_unique: true,
                },
            ],
            classic_text: "alpha\nbeta\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("delimiter_mode".into(), CtValue::String("none".into())),
                    ("show_counts".into(), CtValue::Bool(false)),
                    ("repeated_only".into(), CtValue::Bool(false)),
                    ("unique_only".into(), CtValue::Bool(false)),
                    ("all_repeated".into(), CtValue::Bool(false)),
                    ("ignore_case".into(), CtValue::Bool(false)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("skip_fields".into(), CtValue::Nothing),
                    ("skip_chars".into(), CtValue::Nothing),
                    ("check_chars".into(), CtValue::Int(3)),
                    ("row_index".into(), CtValue::Int(1)),
                    ("group_index".into(), CtValue::Int(1)),
                    ("occurrence_index".into(), CtValue::Int(1)),
                    ("count".into(), CtValue::Int(2)),
                    ("line".into(), CtValue::String("alpha".into())),
                    ("is_repeated".into(), CtValue::Bool(true)),
                    ("is_unique".into(), CtValue::Bool(false)),
                ]),
                CtValue::Record(vec![
                    ("delimiter_mode".into(), CtValue::String("none".into())),
                    ("show_counts".into(), CtValue::Bool(false)),
                    ("repeated_only".into(), CtValue::Bool(false)),
                    ("unique_only".into(), CtValue::Bool(false)),
                    ("all_repeated".into(), CtValue::Bool(false)),
                    ("ignore_case".into(), CtValue::Bool(false)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("skip_fields".into(), CtValue::Nothing),
                    ("skip_chars".into(), CtValue::Nothing),
                    ("check_chars".into(), CtValue::Int(3)),
                    ("row_index".into(), CtValue::Int(2)),
                    ("group_index".into(), CtValue::Int(2)),
                    ("occurrence_index".into(), CtValue::Int(1)),
                    ("count".into(), CtValue::Int(1)),
                    ("line".into(), CtValue::String("beta".into())),
                    ("is_repeated".into(), CtValue::Bool(false)),
                    ("is_unique".into(), CtValue::Bool(true)),
                ]),
            ])
        );
    }
}
