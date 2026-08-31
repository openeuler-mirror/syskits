use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdShuf;

struct ShufIntent {
    argv: Vec<OsString>,
}

struct ShufCore;

impl ShufIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("shuf"));

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

impl ShufCore {
    fn run_core(
        intent: &ShufIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "shuf",
            input,
            shuf_uses_stdin(&intent.argv),
            || Ok(ct_shuf::shuf_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("shuf: {err}")),
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

fn shuf_uses_stdin(argv: &[OsString]) -> bool {
    if argv.iter().skip(1).any(|arg| {
        let arg = arg.to_string_lossy();
        arg == "-e"
            || arg == "--echo"
            || arg == "-i"
            || arg == "--input-range"
            || arg.starts_with("--input-range=")
    }) {
        return false;
    }

    ctengine::argv_uses_stdin(
        argv,
        &["-n", "--head-count", "-o", "--output", "--random-source"],
    )
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("shuf: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'shuf --help' for more information.\n");
    }
    stderr
}

fn opt_string_to_value(value: Option<&str>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.into()),
        None => CtValue::Nothing,
    }
}

fn opt_usize_to_value(value: Option<usize>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(i64::try_from(value).expect("usize fits in i64")),
        None => CtValue::Nothing,
    }
}

fn row_to_value(semantic: &ct_shuf::ShufSemantic, row: &ct_shuf::ShufRow) -> CtValue {
    CtValue::Record(vec![
        (
            "input_kind".into(),
            CtValue::String(semantic.input_kind.clone()),
        ),
        ("head_count".into(), opt_usize_to_value(semantic.head_count)),
        ("repeat".into(), CtValue::Bool(semantic.repeat)),
        (
            "zero_terminated".into(),
            CtValue::Bool(semantic.zero_terminated),
        ),
        (
            "separator_text".into(),
            CtValue::String(semantic.separator_text.clone()),
        ),
        (
            "output_file".into(),
            opt_string_to_value(semantic.output_file.as_deref()),
        ),
        (
            "random_source".into(),
            opt_string_to_value(semantic.random_source.as_deref()),
        ),
        (
            "input_file".into(),
            opt_string_to_value(semantic.input_file.as_deref()),
        ),
        (
            "range_start".into(),
            opt_usize_to_value(semantic.range_start),
        ),
        ("range_end".into(), opt_usize_to_value(semantic.range_end)),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("item_kind".into(), CtValue::String(row.item_kind.clone())),
        (
            "output_text".into(),
            CtValue::String(row.output_text.clone()),
        ),
        ("line".into(), opt_string_to_value(row.line.as_deref())),
        ("number".into(), opt_usize_to_value(row.number)),
    ])
}

fn semantic_to_value(semantic: &ct_shuf::ShufSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdShuf {
    fn signature(&self) -> DataSignature {
        DataSignature::new("shuf", "structured shuf output rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible shuf arguments",
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
        let intent = ShufIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = ShufCore::run_core(&intent, input)?;
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
                source: Some("shuf".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ShufIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-n".into()), None),
                BoundArg::new(CtValue::String("3".into()), None),
                BoundArg::new(CtValue::String("input.txt".into()), None),
            ],
            ..DataCall::named("shuf")
        };

        let intent = ShufIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("shuf"),
                OsString::from("-n"),
                OsString::from("3"),
                OsString::from("input.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_shuf::ShufSemantic {
            input_kind: "file".into(),
            head_count: Some(3),
            repeat: false,
            zero_terminated: false,
            separator_text: "\n".into(),
            output_file: None,
            random_source: Some("random.bin".into()),
            input_file: Some("input.txt".into()),
            range_start: None,
            range_end: None,
            rows: vec![ct_shuf::ShufRow {
                row_index: 1,
                item_kind: "line".into(),
                output_text: "alpha".into(),
                line: Some("alpha".into()),
                number: None,
            }],
            classic_text: "alpha\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("input_kind".into(), CtValue::String("file".into())),
                ("head_count".into(), CtValue::Int(3)),
                ("repeat".into(), CtValue::Bool(false)),
                ("zero_terminated".into(), CtValue::Bool(false)),
                ("separator_text".into(), CtValue::String("\n".into())),
                ("output_file".into(), CtValue::Nothing),
                ("random_source".into(), CtValue::String("random.bin".into()),),
                ("input_file".into(), CtValue::String("input.txt".into())),
                ("range_start".into(), CtValue::Nothing),
                ("range_end".into(), CtValue::Nothing),
                ("row_index".into(), CtValue::Int(1)),
                ("item_kind".into(), CtValue::String("line".into())),
                ("output_text".into(), CtValue::String("alpha".into())),
                ("line".into(), CtValue::String("alpha".into())),
                ("number".into(), CtValue::Nothing),
            ])])
        );
    }
}
