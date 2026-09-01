use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdSort;

struct SortIntent {
    argv: Vec<OsString>,
}

struct SortCore;

impl SortIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("sort"));

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

impl SortCore {
    fn run_core(
        intent: &SortIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "sort",
            input,
            argv_uses_stdin(
                &intent.argv,
                &[
                    "-k",
                    "-o",
                    "-S",
                    "-T",
                    "-t",
                    "--key",
                    "--output",
                    "--buffer-size",
                    "--temporary-directory",
                    "--field-separator",
                    "--parallel",
                    "--batch-size",
                    "--compress-program",
                ],
            ),
            || Ok(ct_sort::sort_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("sort: {err}")),
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
    let mut stderr = format!("sort: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'sort --help' for more information.\n");
    }
    stderr
}

fn opt_string_to_value(value: Option<&str>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.into()),
        None => CtValue::Nothing,
    }
}

fn row_to_value(semantic: &ct_sort::SortSemantic, row: &ct_sort::SortRow) -> CtValue {
    CtValue::Record(vec![
        ("mode".into(), CtValue::String(semantic.mode.clone())),
        ("merge".into(), CtValue::Bool(semantic.merge)),
        ("check".into(), CtValue::Bool(semantic.check)),
        ("debug".into(), CtValue::Bool(semantic.debug)),
        ("reverse".into(), CtValue::Bool(semantic.reverse)),
        ("stable".into(), CtValue::Bool(semantic.stable)),
        ("unique".into(), CtValue::Bool(semantic.unique)),
        (
            "zero_terminated".into(),
            CtValue::Bool(semantic.zero_terminated),
        ),
        ("ignore_case".into(), CtValue::Bool(semantic.ignore_case)),
        (
            "ignore_leading_blanks".into(),
            CtValue::Bool(semantic.ignore_leading_blanks),
        ),
        (
            "dictionary_order".into(),
            CtValue::Bool(semantic.dictionary_order),
        ),
        (
            "ignore_nonprinting".into(),
            CtValue::Bool(semantic.ignore_nonprinting),
        ),
        (
            "key_count".into(),
            CtValue::Int(i64::try_from(semantic.key_count).expect("key count fits")),
        ),
        (
            "source_count".into(),
            CtValue::Int(i64::try_from(semantic.source_count).expect("source count fits")),
        ),
        (
            "separator".into(),
            CtValue::String(semantic.separator.clone()),
        ),
        (
            "output_file".into(),
            opt_string_to_value(semantic.output_file.as_deref()),
        ),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
    ])
}

fn semantic_to_value(semantic: &ct_sort::SortSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

fn display_columns() -> CtValue {
    CtValue::List(vec![CtValue::String("line".into())])
}

impl DataCommand for CmdSort {
    fn signature(&self) -> DataSignature {
        DataSignature::new("sort", "structured sort output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible sort arguments",
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
        let intent = SortIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = SortCore::run_core(&intent, input)?;
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
            source: Some("sort".into()),
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
    use super::{SortIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-r".into()), None),
                BoundArg::new(CtValue::String("input.txt".into()), None),
            ],
            ..DataCall::named("sort")
        };

        let intent = SortIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("sort"),
                OsString::from("-r"),
                OsString::from("input.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_sort::SortSemantic {
            mode: "default".into(),
            merge: false,
            check: false,
            debug: false,
            reverse: false,
            stable: false,
            unique: false,
            zero_terminated: false,
            ignore_case: false,
            ignore_leading_blanks: false,
            dictionary_order: false,
            ignore_nonprinting: false,
            key_count: 0,
            source_count: 1,
            separator: "\n".into(),
            output_file: None,
            rows: vec![
                ct_sort::SortRow {
                    row_index: 1,
                    line: "alpha".into(),
                },
                ct_sort::SortRow {
                    row_index: 2,
                    line: "beta".into(),
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
                    ("mode".into(), CtValue::String("default".into())),
                    ("merge".into(), CtValue::Bool(false)),
                    ("check".into(), CtValue::Bool(false)),
                    ("debug".into(), CtValue::Bool(false)),
                    ("reverse".into(), CtValue::Bool(false)),
                    ("stable".into(), CtValue::Bool(false)),
                    ("unique".into(), CtValue::Bool(false)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("ignore_case".into(), CtValue::Bool(false)),
                    ("ignore_leading_blanks".into(), CtValue::Bool(false)),
                    ("dictionary_order".into(), CtValue::Bool(false)),
                    ("ignore_nonprinting".into(), CtValue::Bool(false)),
                    ("key_count".into(), CtValue::Int(0)),
                    ("source_count".into(), CtValue::Int(1)),
                    ("separator".into(), CtValue::String("\n".into())),
                    ("output_file".into(), CtValue::Nothing),
                    ("row_index".into(), CtValue::Int(1)),
                    ("line".into(), CtValue::String("alpha".into())),
                ]),
                CtValue::Record(vec![
                    ("mode".into(), CtValue::String("default".into())),
                    ("merge".into(), CtValue::Bool(false)),
                    ("check".into(), CtValue::Bool(false)),
                    ("debug".into(), CtValue::Bool(false)),
                    ("reverse".into(), CtValue::Bool(false)),
                    ("stable".into(), CtValue::Bool(false)),
                    ("unique".into(), CtValue::Bool(false)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("ignore_case".into(), CtValue::Bool(false)),
                    ("ignore_leading_blanks".into(), CtValue::Bool(false)),
                    ("dictionary_order".into(), CtValue::Bool(false)),
                    ("ignore_nonprinting".into(), CtValue::Bool(false)),
                    ("key_count".into(), CtValue::Int(0)),
                    ("source_count".into(), CtValue::Int(1)),
                    ("separator".into(), CtValue::String("\n".into())),
                    ("output_file".into(), CtValue::Nothing),
                    ("row_index".into(), CtValue::Int(2)),
                    ("line".into(), CtValue::String("beta".into())),
                ]),
            ])
        );
    }

    #[test]
    fn display_columns_focus_on_sorted_line() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![CtValue::String("line".into())])
        );
    }
}
