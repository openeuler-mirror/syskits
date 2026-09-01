use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdLs;

struct LsIntent {
    argv: Vec<OsString>,
}

struct LsCore;

impl LsIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("ls"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("ls: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }

    fn human_readable(&self) -> bool {
        self.argv
            .iter()
            .skip(1)
            .any(|arg| matches_human_readable_flag(&arg.to_string_lossy()))
    }
}

fn matches_human_readable_flag(arg: &str) -> bool {
    arg == "--human-readable"
        || arg == "--block-size=human-readable"
        || (arg.starts_with('-') && !arg.starts_with("--") && arg[1..].chars().any(|ch| ch == 'h'))
}

impl LsCore {
    fn run_core(intent: &LsIntent) -> (CtValue, String, String, i32) {
        match ct_ls::ls_native_semantic(intent.argv.iter().cloned()) {
            Ok(semantic) => (
                semantic_to_value(&semantic, intent.human_readable()),
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
    let mut stderr = format!("ls: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'ls --help' for more information.\n");
    }
    stderr
}

fn opt_u64_to_value(value: Option<u64>) -> CtValue {
    match value.and_then(|value| i64::try_from(value).ok()) {
        Some(value) => CtValue::Int(value),
        None => CtValue::Nothing,
    }
}

fn size_to_value(row: &ct_ls::LsSemanticRow, human_readable: bool) -> CtValue {
    if human_readable {
        match row.size {
            Some(size) => CtValue::Size(size),
            None => CtValue::Nothing,
        }
    } else {
        opt_u64_to_value(row.size)
    }
}

fn opt_string_to_value(value: &Option<String>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.clone()),
        None => CtValue::Nothing,
    }
}

fn row_to_value(
    semantic: &ct_ls::LsSemantic,
    row: &ct_ls::LsSemanticRow,
    human_readable: bool,
) -> CtValue {
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
        ("mode".into(), opt_string_to_value(&row.mode)),
        ("inode".into(), opt_string_to_value(&row.inode)),
        ("user".into(), opt_string_to_value(&row.user)),
        ("group".into(), opt_string_to_value(&row.group)),
        ("file_type".into(), CtValue::String(row.file_type.clone())),
        ("size".into(), size_to_value(row, human_readable)),
        (
            "last_modified".into(),
            opt_string_to_value(&row.last_modified),
        ),
        ("is_dir".into(), CtValue::Bool(row.is_dir)),
        ("is_file".into(), CtValue::Bool(row.is_file)),
        ("is_symlink".into(), CtValue::Bool(row.is_symlink)),
        ("command_line".into(), CtValue::Bool(row.command_line)),
    ])
}

fn semantic_to_value(semantic: &ct_ls::LsSemantic, human_readable: bool) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row, human_readable))
            .collect(),
    )
}

fn display_columns_for_format(display_format: Option<&str>) -> CtValue {
    let columns = if display_format == Some("long") {
        vec![
            "mode",
            "inode",
            "user",
            "group",
            "name",
            "file_type",
            "size",
            "last_modified",
        ]
    } else {
        vec!["name", "file_type", "size", "last_modified"]
    };
    CtValue::List(
        columns
            .into_iter()
            .map(|column| CtValue::String(column.into()))
            .collect(),
    )
}

fn display_columns_for_value(value: &CtValue) -> CtValue {
    let display_format = match value {
        CtValue::List(items) => items.iter().find_map(|item| {
            let CtValue::Record(fields) = item else {
                return None;
            };
            fields
                .iter()
                .find_map(|(key, value)| match (key.as_str(), value) {
                    ("display_format", CtValue::String(format)) => Some(format.as_str()),
                    _ => None,
                })
        }),
        _ => None,
    };
    display_columns_for_format(display_format)
}

impl DataCommand for CmdLs {
    fn signature(&self) -> DataSignature {
        DataSignature::new("ls", "structured directory listing rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible ls arguments",
                CtType::Any,
            ))
            .input(CtType::Nothing)
            .output(CtType::List)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = LsIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = LsCore::run_core(&intent);
        let display_columns = display_columns_for_value(&value);
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
            source: Some("ls".into()),
            ..Default::default()
        };
        if let Ok(mut custom) = metadata.custom.lock() {
            custom.insert("display.columns".into(), display_columns);
        }
        Ok(CtPipelineData::Value(value, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::{LsIntent, display_columns_for_format, semantic_to_value};
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
            ..DataCall::named("ls")
        };

        let intent = LsIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("ls"),
                OsString::from("-1"),
                OsString::from(".")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_ls::LsSemantic {
            command: "ls".into(),
            display_format: "one-line".into(),
            include_hidden: false,
            almost_all: false,
            directory_mode: false,
            recursive: false,
            paths: vec![".".into()],
            rows: vec![ct_ls::LsSemanticRow {
                row_index: 1,
                source_path: ".".into(),
                path: "./alpha.txt".into(),
                name: "alpha.txt".into(),
                mode: Some("-rw-r--r--".into()),
                inode: Some("123".into()),
                user: Some("root".into()),
                group: Some("root".into()),
                file_type: "file".into(),
                size: Some(6),
                size_display: Some("6".into()),
                last_modified: Some("Jan  1 00:00".into()),
                modified_unix_seconds: Some(1_704_067_200),
                is_dir: false,
                is_file: true,
                is_symlink: false,
                command_line: false,
            }],
            classic_text: "alpha.txt\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        }, false);

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("command".into(), CtValue::String("ls".into())),
                ("display_format".into(), CtValue::String("one-line".into())),
                ("include_hidden".into(), CtValue::Bool(false)),
                ("almost_all".into(), CtValue::Bool(false)),
                ("directory_mode".into(), CtValue::Bool(false)),
                ("recursive".into(), CtValue::Bool(false)),
                ("row_index".into(), CtValue::Int(1)),
                ("source_path".into(), CtValue::String(".".into())),
                ("path".into(), CtValue::String("./alpha.txt".into())),
                ("name".into(), CtValue::String("alpha.txt".into())),
                ("mode".into(), CtValue::String("-rw-r--r--".into())),
                ("inode".into(), CtValue::String("123".into())),
                ("user".into(), CtValue::String("root".into())),
                ("group".into(), CtValue::String("root".into())),
                ("file_type".into(), CtValue::String("file".into())),
                ("size".into(), CtValue::Int(6)),
                ("last_modified".into(), CtValue::String("Jan  1 00:00".into())),
                ("is_dir".into(), CtValue::Bool(false)),
                ("is_file".into(), CtValue::Bool(true)),
                ("is_symlink".into(), CtValue::Bool(false)),
                ("command_line".into(), CtValue::Bool(false)),
            ])])
        );
    }

    #[test]
    fn semantic_to_value_uses_size_type_for_human_readable_mode() {
        let value = semantic_to_value(&ct_ls::LsSemantic {
            command: "ls".into(),
            display_format: "one-line".into(),
            include_hidden: false,
            almost_all: false,
            directory_mode: false,
            recursive: false,
            paths: vec![".".into()],
            rows: vec![ct_ls::LsSemanticRow {
                row_index: 1,
                source_path: ".".into(),
                path: "./alpha.txt".into(),
                name: "alpha.txt".into(),
                mode: None,
                inode: None,
                user: None,
                group: None,
                file_type: "file".into(),
                size: Some(2048),
                size_display: Some("2.0K".into()),
                last_modified: Some("Jan  1 00:00".into()),
                modified_unix_seconds: Some(1_704_067_200),
                is_dir: false,
                is_file: true,
                is_symlink: false,
                command_line: false,
            }],
            classic_text: "alpha.txt\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        }, true);

        let CtValue::List(rows) = value else {
            panic!("expected list");
        };
        let CtValue::Record(fields) = &rows[0] else {
            panic!("expected record");
        };
        assert!(matches!(
            fields.iter().find(|(k, _)| k == "size").map(|(_, v)| v),
            Some(CtValue::Size(2048))
        ));
    }

    #[test]
    fn human_readable_detects_h_flag() {
        let intent = LsIntent {
            argv: vec![OsString::from("ls"), OsString::from("-lh")],
        };

        assert!(intent.human_readable());
    }

    #[test]
    fn default_display_columns_are_compact_ls_columns() {
        assert_eq!(
            display_columns_for_format(None),
            CtValue::List(vec![
                CtValue::String("name".into()),
                CtValue::String("file_type".into()),
                CtValue::String("size".into()),
                CtValue::String("last_modified".into()),
            ])
        );
    }

    #[test]
    fn long_display_columns_include_long_listing_fields() {
        assert_eq!(
            display_columns_for_format(Some("long")),
            CtValue::List(vec![
                CtValue::String("mode".into()),
                CtValue::String("inode".into()),
                CtValue::String("user".into()),
                CtValue::String("group".into()),
                CtValue::String("name".into()),
                CtValue::String("file_type".into()),
                CtValue::String("size".into()),
                CtValue::String("last_modified".into()),
            ])
        );
    }
}
