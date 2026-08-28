use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdWc;

struct WcIntent {
    argv: Vec<OsString>,
}

struct WcCore;

impl WcIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("wc"));

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

impl WcCore {
    fn run_core(
        intent: &WcIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "wc",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &["--files0-from"]),
            || Ok(ct_wc::wc_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("wc: {err}")),
        )?;
        Ok(match result {
            Ok(semantic) => (
                semantic_to_value(&semantic),
                semantic.classic_text,
                String::new(),
                0,
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

fn semantic_to_value(semantic: &ct_wc::WcSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_wc::WcRow) -> CtValue {
    let mut fields = Vec::new();

    fields.push((
        "row_kind".into(),
        CtValue::String(row_kind_name(row.row_kind).to_string()),
    ));
    fields.push((
        "is_total".into(),
        CtValue::Bool(row.row_kind == ct_wc::WcRowKind::Total),
    ));

    if let Some(input) = &row.input {
        fields.push(("input".into(), CtValue::String(input.clone())));
    }
    if let Some(lines) = row.lines {
        fields.push(("lines".into(), int_value(lines)));
    }
    if let Some(words) = row.words {
        fields.push(("words".into(), int_value(words)));
    }
    if let Some(chars) = row.chars {
        fields.push(("chars".into(), int_value(chars)));
    }
    if let Some(bytes) = row.bytes {
        fields.push(("bytes".into(), int_value(bytes)));
    }
    if let Some(max_line_length) = row.max_line_length {
        fields.push(("max_line_length".into(), int_value(max_line_length)));
    }

    CtValue::Record(fields)
}

fn row_kind_name(kind: ct_wc::WcRowKind) -> &'static str {
    match kind {
        ct_wc::WcRowKind::Path => "path",
        ct_wc::WcRowKind::Stdin => "stdin",
        ct_wc::WcRowKind::Total => "total",
    }
}

fn int_value(value: usize) -> CtValue {
    CtValue::Int(i64::try_from(value).expect("wc counters fit in i64"))
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("wc: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'wc --help' for more information.\n");
    }
    stderr
}

impl DataCommand for CmdWc {
    fn signature(&self) -> DataSignature {
        DataSignature::new("wc", "structured line, word, and byte counts")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible wc arguments",
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
        let intent = WcIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = WcCore::run_core(&intent, input)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: exit_code == 0,
                stderr_text: if stderr_text.is_empty() {
                    None
                } else {
                    Some(stderr_text)
                },
                exit_code,
                source: Some("wc".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{WcIntent, row_kind_name, row_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-l".into()), None),
                BoundArg::new(CtValue::String("file".into()), None),
            ],
            ..DataCall::named("wc")
        };

        let intent = WcIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("wc"),
                OsString::from("-l"),
                OsString::from("file")
            ]
        );
    }

    #[test]
    fn row_kind_name_uses_stable_strings() {
        assert_eq!(row_kind_name(ct_wc::WcRowKind::Path), "path");
        assert_eq!(row_kind_name(ct_wc::WcRowKind::Stdin), "stdin");
        assert_eq!(row_kind_name(ct_wc::WcRowKind::Total), "total");
    }

    #[test]
    fn row_to_value_renders_selected_counts() {
        let value = row_to_value(&ct_wc::WcRow {
            row_kind: ct_wc::WcRowKind::Path,
            input: Some("sample.txt".into()),
            lines: Some(2),
            words: Some(4),
            chars: None,
            bytes: Some(23),
            max_line_length: None,
        });

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("row_kind".into(), CtValue::String("path".into())),
                ("is_total".into(), CtValue::Bool(false)),
                ("input".into(), CtValue::String("sample.txt".into())),
                ("lines".into(), CtValue::Int(2)),
                ("words".into(), CtValue::Int(4)),
                ("bytes".into(), CtValue::Int(23)),
            ])
        );
    }
}
