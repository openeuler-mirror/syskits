use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdHead;

struct HeadIntent {
    argv: Vec<OsString>,
}

struct HeadCore;

impl HeadIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("head"));

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

impl HeadCore {
    fn run_core(
        intent: &HeadIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "head",
            input,
            argv_uses_stdin(&intent.argv, &["-n", "-c", "--lines", "--bytes"]),
            || Ok(ct_head::head_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("head: {err}")),
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
    let mut saw_file = false;
    let mut end_options = false;
    let mut index = 1;

    while index < argv.len() {
        let arg = argv[index].to_string_lossy();
        if arg == "-" {
            return true;
        }
        if end_options {
            saw_file = true;
            break;
        }
        if arg == "--" {
            end_options = true;
            index += 1;
            continue;
        }
        if arg.starts_with("--") {
            let flag = arg.split('=').next().unwrap_or_default();
            if value_flags.contains(&flag) && !arg.contains('=') {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            if value_flags.contains(&arg.as_ref()) {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        saw_file = true;
        break;
    }

    !saw_file
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("head: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'head --help' for more information.\n");
    }
    stderr
}

fn row_to_value(semantic: &ct_head::HeadSemantic, row: &ct_head::HeadRow) -> CtValue {
    CtValue::Record(vec![
        ("mode".into(), CtValue::String(semantic.mode.clone())),
        (
            "count".into(),
            CtValue::Int(i64::try_from(semantic.count).expect("count fits")),
        ),
        (
            "zero_terminated".into(),
            CtValue::Bool(semantic.zero_terminated),
        ),
        ("quiet".into(), CtValue::Bool(semantic.quiet)),
        ("verbose".into(), CtValue::Bool(semantic.verbose)),
        (
            "source_name".into(),
            CtValue::String(row.source_name.clone()),
        ),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
        (
            "byte_length".into(),
            CtValue::Int(i64::try_from(row.byte_length).expect("byte length fits")),
        ),
        ("terminated".into(), CtValue::Bool(row.terminated)),
    ])
}

fn semantic_to_value(semantic: &ct_head::HeadSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdHead {
    fn signature(&self) -> DataSignature {
        DataSignature::new("head", "structured head output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible head arguments",
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
        let intent = HeadIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = HeadCore::run_core(&intent, input)?;
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
                source: Some("head".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{HeadIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-n".into()), None),
                BoundArg::new(CtValue::String("2".into()), None),
                BoundArg::new(CtValue::String("file.txt".into()), None),
            ],
            ..DataCall::named("head")
        };

        let intent = HeadIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("head"),
                OsString::from("-n"),
                OsString::from("2"),
                OsString::from("file.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_head::HeadSemantic {
            mode: "first_lines".into(),
            count: 2,
            zero_terminated: false,
            quiet: false,
            verbose: false,
            rows: vec![
                ct_head::HeadRow {
                    source_name: "file.txt".into(),
                    row_index: 1,
                    line: "alpha".into(),
                    byte_length: 5,
                    terminated: true,
                },
                ct_head::HeadRow {
                    source_name: "file.txt".into(),
                    row_index: 2,
                    line: "beta".into(),
                    byte_length: 4,
                    terminated: true,
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
                    ("mode".into(), CtValue::String("first_lines".into())),
                    ("count".into(), CtValue::Int(2)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("quiet".into(), CtValue::Bool(false)),
                    ("verbose".into(), CtValue::Bool(false)),
                    ("source_name".into(), CtValue::String("file.txt".into())),
                    ("row_index".into(), CtValue::Int(1)),
                    ("line".into(), CtValue::String("alpha".into())),
                    ("byte_length".into(), CtValue::Int(5)),
                    ("terminated".into(), CtValue::Bool(true)),
                ]),
                CtValue::Record(vec![
                    ("mode".into(), CtValue::String("first_lines".into())),
                    ("count".into(), CtValue::Int(2)),
                    ("zero_terminated".into(), CtValue::Bool(false)),
                    ("quiet".into(), CtValue::Bool(false)),
                    ("verbose".into(), CtValue::Bool(false)),
                    ("source_name".into(), CtValue::String("file.txt".into())),
                    ("row_index".into(), CtValue::Int(2)),
                    ("line".into(), CtValue::String("beta".into())),
                    ("byte_length".into(), CtValue::Int(4)),
                    ("terminated".into(), CtValue::Bool(true)),
                ]),
            ])
        );
    }
}
