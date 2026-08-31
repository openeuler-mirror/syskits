use clap::error::ErrorKind;
use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdSplit;

struct SplitIntent {
    argv: Vec<OsString>,
}

struct SplitCore;

impl SplitIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("split"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("split: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl SplitCore {
    fn run_core(intent: &SplitIntent) -> (CtValue, String, String, i32) {
        if let Err(err) = ct_split::ct_app().try_get_matches_from(intent.argv.iter().cloned())
            && matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            )
        {
            return (
                CtValue::List(Vec::new()),
                err.render().to_string(),
                String::new(),
                0,
            );
        }

        match ct_split::split_native_semantic(intent.argv.iter().cloned()) {
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
        }
    }
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("split: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'split --help' for more information.\n");
    }
    stderr
}

fn opt_string_to_value(value: Option<&str>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.into()),
        None => CtValue::Nothing,
    }
}

fn row_to_value(semantic: &ct_split::SplitSemantic, row: &ct_split::SplitRow) -> CtValue {
    CtValue::Record(vec![
        (
            "strategy".into(),
            CtValue::String(semantic.strategy.clone()),
        ),
        ("prefix".into(), CtValue::String(semantic.prefix.clone())),
        ("input".into(), CtValue::String(semantic.input.clone())),
        (
            "filter".into(),
            opt_string_to_value(semantic.filter.as_deref()),
        ),
        (
            "separator_text".into(),
            CtValue::String(semantic.separator_text.clone()),
        ),
        ("verbose".into(), CtValue::Bool(semantic.verbose)),
        (
            "elide_empty_files".into(),
            CtValue::Bool(semantic.elide_empty_files),
        ),
        ("unbuffered".into(), CtValue::Bool(semantic.unbuffered)),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        (
            "output_kind".into(),
            CtValue::String(row.output_kind.clone()),
        ),
        ("path".into(), opt_string_to_value(row.path.as_deref())),
        (
            "file_name".into(),
            opt_string_to_value(row.file_name.as_deref()),
        ),
        (
            "byte_len".into(),
            CtValue::Int(i64::try_from(row.byte_len).expect("byte len fits")),
        ),
        ("content".into(), CtValue::String(row.content.clone())),
    ])
}

fn semantic_to_value(semantic: &ct_split::SplitSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdSplit {
    fn signature(&self) -> DataSignature {
        DataSignature::new("split", "structured split output rows")
            .input(CtType::Nothing)
            .output(CtType::List)
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible split arguments",
                CtType::Any,
            ))
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = SplitIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = SplitCore::run_core(&intent);
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
                source: Some("split".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{SplitIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-l".into()), None),
                BoundArg::new(CtValue::String("2".into()), None),
                BoundArg::new(CtValue::String("input.txt".into()), None),
                BoundArg::new(CtValue::String("chunk_".into()), None),
            ],
            ..DataCall::named("split")
        };

        let intent = SplitIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("split"),
                OsString::from("-l"),
                OsString::from("2"),
                OsString::from("input.txt"),
                OsString::from("chunk_"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_split::SplitSemantic {
            strategy: "lines".into(),
            prefix: "chunk_".into(),
            input: "input.txt".into(),
            filter: None,
            separator_text: "\n".into(),
            verbose: false,
            elide_empty_files: false,
            unbuffered: false,
            rows: vec![ct_split::SplitRow {
                row_index: 1,
                output_kind: "file".into(),
                path: Some("/tmp/chunk_aa".into()),
                file_name: Some("chunk_aa".into()),
                byte_len: 4,
                content: "1\n2\n".into(),
            }],
            classic_text: String::new(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("strategy".into(), CtValue::String("lines".into())),
                ("prefix".into(), CtValue::String("chunk_".into())),
                ("input".into(), CtValue::String("input.txt".into())),
                ("filter".into(), CtValue::Nothing),
                ("separator_text".into(), CtValue::String("\n".into())),
                ("verbose".into(), CtValue::Bool(false)),
                ("elide_empty_files".into(), CtValue::Bool(false)),
                ("unbuffered".into(), CtValue::Bool(false)),
                ("row_index".into(), CtValue::Int(1)),
                ("output_kind".into(), CtValue::String("file".into())),
                ("path".into(), CtValue::String("/tmp/chunk_aa".into())),
                ("file_name".into(), CtValue::String("chunk_aa".into())),
                ("byte_len".into(), CtValue::Int(4)),
                ("content".into(), CtValue::String("1\n2\n".into())),
            ])])
        );
    }
}
