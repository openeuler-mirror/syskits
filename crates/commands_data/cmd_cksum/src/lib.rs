use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdCksum;

struct CksumIntent {
    argv: Vec<OsString>,
}

struct CksumCore;

impl CksumIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("cksum"));

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

impl CksumCore {
    fn run_core(
        intent: &CksumIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "cksum",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &["-a", "--algorithm", "-l", "--length"]),
            || Ok(ct_cksum::cksum_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("cksum: {err}")),
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
    let mut stderr = format!("cksum: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'cksum --help' for more information.\n");
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
    match value.and_then(|value| i64::try_from(value).ok()) {
        Some(value) => CtValue::Int(value),
        None => CtValue::Nothing,
    }
}

fn optional_bool(value: Option<bool>) -> CtValue {
    match value {
        Some(value) => CtValue::Bool(value),
        None => CtValue::Nothing,
    }
}

fn semantic_to_value(semantic: &ct_cksum::CksumSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_cksum::CksumRow) -> CtValue {
    CtValue::Record(vec![
        ("kind".into(), CtValue::String(row.kind.clone())),
        ("algorithm".into(), CtValue::String(row.algorithm.clone())),
        ("input".into(), CtValue::String(row.input.clone())),
        ("file".into(), optional_string(&row.file)),
        ("manifest_file".into(), optional_string(&row.manifest_file)),
        ("checksum".into(), CtValue::String(row.checksum.clone())),
        (
            "actual_checksum".into(),
            optional_string(&row.actual_checksum),
        ),
        ("bytes".into(), optional_usize(row.bytes)),
        ("reported_size".into(), optional_usize(row.reported_size)),
        ("size_kind".into(), optional_string(&row.size_kind)),
        ("block_size".into(), optional_usize(row.block_size)),
        (
            "output_format".into(),
            CtValue::String(row.output_format.clone()),
        ),
        ("tagged".into(), CtValue::Bool(row.tagged)),
        ("binary".into(), CtValue::Bool(row.binary)),
        ("status".into(), optional_string(&row.status)),
        ("matched".into(), optional_bool(row.matched)),
    ])
}

fn display_columns() -> CtValue {
    CtValue::List(
        ["file", "checksum", "bytes", "status", "matched"]
            .into_iter()
            .map(|column| CtValue::String(column.into()))
            .collect(),
    )
}

impl DataCommand for CmdCksum {
    fn signature(&self) -> DataSignature {
        DataSignature::new("cksum", "structured checksum output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible cksum arguments",
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
        let intent = CksumIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = CksumCore::run_core(&intent, input)?;
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
            source: Some("cksum".into()),
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
    use super::{CksumIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("--algorithm=sha256".into()), None),
                BoundArg::new(CtValue::String("sample.txt".into()), None),
            ],
            ..DataCall::named("cksum")
        };

        let intent = CksumIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("cksum"),
                OsString::from("--algorithm=sha256"),
                OsString::from("sample.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_cksum::CksumSemantic {
            rows: vec![ct_cksum::CksumRow {
                kind: "compute".into(),
                algorithm: "crc".into(),
                input: "file".into(),
                file: Some("sample.txt".into()),
                manifest_file: None,
                checksum: "1219131554".into(),
                actual_checksum: None,
                bytes: Some(3),
                reported_size: Some(3),
                size_kind: Some("bytes".into()),
                block_size: None,
                output_format: "hexadecimal".into(),
                tagged: true,
                binary: false,
                status: None,
                matched: None,
            }],
            classic_text: "1219131554 3 sample.txt\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("kind".into(), CtValue::String("compute".into())),
                ("algorithm".into(), CtValue::String("crc".into())),
                ("input".into(), CtValue::String("file".into())),
                ("file".into(), CtValue::String("sample.txt".into())),
                ("manifest_file".into(), CtValue::Nothing),
                ("checksum".into(), CtValue::String("1219131554".into())),
                ("actual_checksum".into(), CtValue::Nothing),
                ("bytes".into(), CtValue::Int(3)),
                ("reported_size".into(), CtValue::Int(3)),
                ("size_kind".into(), CtValue::String("bytes".into())),
                ("block_size".into(), CtValue::Nothing),
                (
                    "output_format".into(),
                    CtValue::String("hexadecimal".into())
                ),
                ("tagged".into(), CtValue::Bool(true)),
                ("binary".into(), CtValue::Bool(false)),
                ("status".into(), CtValue::Nothing),
                ("matched".into(), CtValue::Nothing),
            ])])
        );
    }

    #[test]
    fn display_columns_focus_on_checksum_result() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![
                CtValue::String("file".into()),
                CtValue::String("checksum".into()),
                CtValue::String("bytes".into()),
                CtValue::String("status".into()),
                CtValue::String("matched".into()),
            ])
        );
    }
}
