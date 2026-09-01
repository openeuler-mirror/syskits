use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdHashsum;

struct HashsumIntent {
    argv: Vec<OsString>,
}

struct HashsumCore;

impl HashsumIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("hashsum"));

        for arg in &call.positionals {
            argv.push(OsString::from(value_to_arg(&arg.value)));
        }

        Ok(Self { argv })
    }

    fn is_check_mode(&self) -> bool {
        let mut options_done = false;
        for arg in self.argv.iter().skip(1) {
            let arg = arg.to_string_lossy();
            if options_done {
                continue;
            }

            if arg == "--" {
                options_done = true;
                continue;
            }

            if arg == "--check" || arg.starts_with("--check=") {
                return true;
            }

            if arg.starts_with('-')
                && !arg.starts_with("--")
                && arg[1..].chars().any(|ch| ch == 'c')
            {
                return true;
            }
        }

        false
    }
}

fn value_to_arg(value: &CtValue) -> String {
    match value {
        CtValue::String(s) => s.clone(),
        other => other.to_text(),
    }
}

impl HashsumCore {
    fn run_core(
        intent: &HashsumIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "hashsum",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &["-l", "--length", "--bits"]),
            || {
                Ok(ct_hashsum::hashsum_native_semantic(
                    intent.argv.iter().cloned(),
                ))
            },
            |err| CtDiagnosticError::simple(format!("hashsum: {err}")),
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
    let mut stderr = format!("hashsum: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'hashsum --help' for more information.\n");
    }
    stderr
}

fn optional_string(value: &Option<String>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.clone()),
        None => CtValue::Nothing,
    }
}

fn semantic_to_value(semantic: &ct_hashsum::HashsumSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_hashsum::HashsumRow) -> CtValue {
    CtValue::Record(vec![
        ("kind".into(), CtValue::String(row.kind.clone())),
        ("algorithm".into(), CtValue::String(row.algorithm.clone())),
        ("file".into(), CtValue::String(row.file.clone())),
        ("digest_hex".into(), CtValue::String(row.digest_hex.clone())),
        (
            "output_bits".into(),
            CtValue::Int(i64::try_from(row.output_bits).expect("bits fit in i64")),
        ),
        (
            "binary_marker".into(),
            CtValue::String(row.binary_marker.clone()),
        ),
        (
            "output_style".into(),
            CtValue::String(row.output_style.clone()),
        ),
        ("source".into(), CtValue::String(row.source.clone())),
        (
            "filename_was_escaped".into(),
            CtValue::Bool(row.filename_was_escaped),
        ),
        (
            "rendered_filename".into(),
            optional_string(&row.rendered_filename),
        ),
        ("manifest_file".into(), optional_string(&row.manifest_file)),
        (
            "expected_digest".into(),
            optional_string(&row.expected_digest),
        ),
        ("actual_digest".into(), optional_string(&row.actual_digest)),
        ("status".into(), optional_string(&row.status)),
        (
            "matched".into(),
            match row.matched {
                Some(value) => CtValue::Bool(value),
                None => CtValue::Nothing,
            },
        ),
        ("input_format".into(), optional_string(&row.input_format)),
        ("ignored_missing".into(), CtValue::Bool(row.ignored_missing)),
        (
            "binary_check".into(),
            match row.binary_check {
                Some(value) => CtValue::Bool(value),
                None => CtValue::Nothing,
            },
        ),
    ])
}

fn display_columns(is_check_mode: bool) -> CtValue {
    let columns: &[&str] = if is_check_mode {
        &["file", "status", "matched", "algorithm"]
    } else {
        &["file", "digest_hex", "algorithm"]
    };

    CtValue::List(
        columns
            .iter()
            .map(|column| CtValue::String((*column).into()))
            .collect(),
    )
}

impl DataCommand for CmdHashsum {
    fn signature(&self) -> DataSignature {
        DataSignature::new("hashsum", "structured digest output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible hashsum arguments",
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
        let intent = HashsumIntent::from_call(call)?;
        let is_check_mode = intent.is_check_mode();
        let (value, classic_text, stderr_text, exit_code) = HashsumCore::run_core(&intent, input)?;
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
            source: Some("hashsum".into()),
            ..Default::default()
        };
        if let Ok(mut custom) = metadata.custom.lock() {
            custom.insert("display.columns".into(), display_columns(is_check_mode));
        }
        Ok(CtPipelineData::Value(value, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::{HashsumIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("--sha256".into()), None),
                BoundArg::new(CtValue::String("sample.txt".into()), None),
            ],
            ..DataCall::named("hashsum")
        };

        let intent = HashsumIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("hashsum"),
                OsString::from("--sha256"),
                OsString::from("sample.txt"),
            ]
        );
        assert!(!intent.is_check_mode());
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_hashsum::HashsumSemantic {
            rows: vec![ct_hashsum::HashsumRow {
                kind: "digest".into(),
                algorithm: "SHA256".into(),
                file: "sample.txt".into(),
                digest_hex: "abc123".into(),
                output_bits: 256,
                binary_marker: " ".into(),
                output_style: "gnu".into(),
                source: "file".into(),
                filename_was_escaped: false,
                rendered_filename: Some("sample.txt".into()),
                manifest_file: None,
                expected_digest: None,
                actual_digest: None,
                status: None,
                matched: None,
                input_format: None,
                ignored_missing: false,
                binary_check: None,
            }],
            classic_text: String::new(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("kind".into(), CtValue::String("digest".into())),
                ("algorithm".into(), CtValue::String("SHA256".into())),
                ("file".into(), CtValue::String("sample.txt".into())),
                ("digest_hex".into(), CtValue::String("abc123".into())),
                ("output_bits".into(), CtValue::Int(256)),
                ("binary_marker".into(), CtValue::String(" ".into())),
                ("output_style".into(), CtValue::String("gnu".into())),
                ("source".into(), CtValue::String("file".into())),
                ("filename_was_escaped".into(), CtValue::Bool(false)),
                (
                    "rendered_filename".into(),
                    CtValue::String("sample.txt".into())
                ),
                ("manifest_file".into(), CtValue::Nothing),
                ("expected_digest".into(), CtValue::Nothing),
                ("actual_digest".into(), CtValue::Nothing),
                ("status".into(), CtValue::Nothing),
                ("matched".into(), CtValue::Nothing),
                ("input_format".into(), CtValue::Nothing),
                ("ignored_missing".into(), CtValue::Bool(false)),
                ("binary_check".into(), CtValue::Nothing),
            ])])
        );
    }

    #[test]
    fn is_check_mode_detects_check_flag() {
        let intent = HashsumIntent {
            argv: vec![
                OsString::from("hashsum"),
                OsString::from("-cq"),
                OsString::from("manifest.txt"),
            ],
        };

        assert!(intent.is_check_mode());
    }

    #[test]
    fn is_check_mode_ignores_dash_c_after_option_terminator() {
        let intent = HashsumIntent {
            argv: vec![
                OsString::from("hashsum"),
                OsString::from("--"),
                OsString::from("-c"),
            ],
        };

        assert!(!intent.is_check_mode());
    }

    #[test]
    fn display_columns_focus_on_key_hashsum_fields() {
        assert_eq!(
            display_columns(false),
            CtValue::List(vec![
                CtValue::String("file".into()),
                CtValue::String("digest_hex".into()),
                CtValue::String("algorithm".into()),
            ])
        );
        assert_eq!(
            display_columns(true),
            CtValue::List(vec![
                CtValue::String("file".into()),
                CtValue::String("status".into()),
                CtValue::String("matched".into()),
                CtValue::String("algorithm".into()),
            ])
        );
    }
}
