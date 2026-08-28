use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdDir;

struct DirIntent {
    argv: Vec<OsString>,
}

struct DirCore;

impl DirIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("dir"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("dir: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl DirCore {
    fn run_core(intent: &DirIntent) -> (CtValue, String, String, i32) {
        match ct_dir::dir_native_semantic(intent.argv.iter().cloned()) {
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
    let mut stderr = format!("dir: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'dir --help' for more information.\n");
    }
    stderr
}

fn opt_u64_to_value(value: Option<u64>) -> CtValue {
    match value.and_then(|value| i64::try_from(value).ok()) {
        Some(value) => CtValue::Int(value),
        None => CtValue::Nothing,
    }
}

fn row_to_value(semantic: &ct_dir::DirSemantic, row: &ct_dir::DirSemanticRow) -> CtValue {
    CtValue::Record(vec![
        ("command".into(), CtValue::String(semantic.command.clone())),
        (
            "display_format".into(),
            CtValue::String(semantic.display_format.clone()),
        ),
        (
            "include_hidden".into(),
            CtValue::Bool(semantic.include_hidden),
        ),
        ("almost_all".into(), CtValue::Bool(semantic.almost_all)),
        (
            "directory_mode".into(),
            CtValue::Bool(semantic.directory_mode),
        ),
        ("recursive".into(), CtValue::Bool(semantic.recursive)),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        (
            "source_path".into(),
            CtValue::String(row.source_path.clone()),
        ),
        ("path".into(), CtValue::String(row.path.clone())),
        ("name".into(), CtValue::String(row.name.clone())),
        ("file_type".into(), CtValue::String(row.file_type.clone())),
        ("size".into(), opt_u64_to_value(row.size)),
        ("is_dir".into(), CtValue::Bool(row.is_dir)),
        ("is_file".into(), CtValue::Bool(row.is_file)),
        ("is_symlink".into(), CtValue::Bool(row.is_symlink)),
        ("command_line".into(), CtValue::Bool(row.command_line)),
    ])
}

fn semantic_to_value(semantic: &ct_dir::DirSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdDir {
    fn signature(&self) -> DataSignature {
        DataSignature::new("dir", "structured directory listing rows")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = DirIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = DirCore::run_core(&intent);
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
                source: Some("dir".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{DirIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-1".into()), None),
                BoundArg::new(CtValue::String(".".into()), None),
            ],
            ..DataCall::named("dir")
        };

        let intent = DirIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("dir"),
                OsString::from("-1"),
                OsString::from(".")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_dir::DirSemantic {
            command: "dir".into(),
            display_format: "columns".into(),
            include_hidden: false,
            almost_all: false,
            directory_mode: false,
            recursive: false,
            paths: vec![".".into()],
            rows: vec![ct_dir::DirSemanticRow {
                row_index: 1,
                source_path: ".".into(),
                path: "./alpha.txt".into(),
                name: "alpha.txt".into(),
                file_type: "file".into(),
                size: Some(6),
                is_dir: false,
                is_file: true,
                is_symlink: false,
                command_line: false,
            }],
            classic_text: "alpha.txt\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("command".into(), CtValue::String("dir".into())),
                ("display_format".into(), CtValue::String("columns".into())),
                ("include_hidden".into(), CtValue::Bool(false)),
                ("almost_all".into(), CtValue::Bool(false)),
                ("directory_mode".into(), CtValue::Bool(false)),
                ("recursive".into(), CtValue::Bool(false)),
                ("row_index".into(), CtValue::Int(1)),
                ("source_path".into(), CtValue::String(".".into())),
                ("path".into(), CtValue::String("./alpha.txt".into())),
                ("name".into(), CtValue::String("alpha.txt".into())),
                ("file_type".into(), CtValue::String("file".into())),
                ("size".into(), CtValue::Int(6)),
                ("is_dir".into(), CtValue::Bool(false)),
                ("is_file".into(), CtValue::Bool(true)),
                ("is_symlink".into(), CtValue::Bool(false)),
                ("command_line".into(), CtValue::Bool(false)),
            ])])
        );
    }
}
