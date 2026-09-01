use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdVdir;

struct VdirIntent {
    argv: Vec<OsString>,
}

struct VdirCore;

impl VdirIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("vdir"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("vdir: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl VdirCore {
    fn run_core(intent: &VdirIntent) -> (CtValue, String, String, i32) {
        match ct_vdir::vdir_native_semantic(intent.argv.iter().cloned()) {
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
    let mut stderr = format!("vdir: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'vdir --help' for more information.\n");
    }
    stderr
}

fn opt_u64_to_value(value: Option<u64>) -> CtValue {
    match value.and_then(|value| i64::try_from(value).ok()) {
        Some(value) => CtValue::Int(value),
        None => CtValue::Nothing,
    }
}

fn opt_i64_to_value(value: Option<i64>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(value),
        None => CtValue::Nothing,
    }
}

fn opt_string_to_value(value: Option<&str>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.into()),
        None => CtValue::Nothing,
    }
}

fn row_to_value(semantic: &ct_vdir::VdirSemantic, row: &ct_vdir::VdirSemanticRow) -> CtValue {
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
        (
            "permissions".into(),
            CtValue::String(row.permissions.clone()),
        ),
        ("hard_links".into(), opt_u64_to_value(row.hard_links)),
        ("owner".into(), opt_string_to_value(row.owner.as_deref())),
        ("group".into(), opt_string_to_value(row.group.as_deref())),
        ("blocks".into(), opt_u64_to_value(row.blocks)),
        (
            "modified_unix_seconds".into(),
            opt_i64_to_value(row.modified_unix_seconds),
        ),
        (
            "link_target".into(),
            opt_string_to_value(row.link_target.as_deref()),
        ),
        ("is_dir".into(), CtValue::Bool(row.is_dir)),
        ("is_file".into(), CtValue::Bool(row.is_file)),
        ("is_symlink".into(), CtValue::Bool(row.is_symlink)),
        ("command_line".into(), CtValue::Bool(row.command_line)),
    ])
}

fn semantic_to_value(semantic: &ct_vdir::VdirSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

fn display_columns() -> CtValue {
    CtValue::List(
        [
            "permissions",
            "hard_links",
            "owner",
            "group",
            "name",
            "file_type",
            "size",
        ]
        .into_iter()
        .map(|name| CtValue::String(name.into()))
        .collect(),
    )
}

impl DataCommand for CmdVdir {
    fn signature(&self) -> DataSignature {
        DataSignature::new("vdir", "structured verbose directory listing rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible vdir arguments",
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
        let intent = VdirIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = VdirCore::run_core(&intent);
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
            source: Some("vdir".into()),
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
    use super::{VdirIntent, display_columns, semantic_to_value};
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
            ..DataCall::named("vdir")
        };

        let intent = VdirIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("vdir"),
                OsString::from("-1"),
                OsString::from(".")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_vdir::VdirSemantic {
            command: "vdir".into(),
            display_format: "long".into(),
            include_hidden: false,
            almost_all: false,
            directory_mode: false,
            recursive: false,
            paths: vec![".".into()],
            rows: vec![ct_vdir::VdirSemanticRow {
                row_index: 1,
                source_path: ".".into(),
                path: "./alpha.txt".into(),
                name: "alpha.txt".into(),
                file_type: "file".into(),
                size: Some(6),
                permissions: "-rw-r--r--".into(),
                hard_links: Some(1),
                owner: Some("alice".into()),
                group: Some("staff".into()),
                blocks: Some(8),
                modified_unix_seconds: Some(1_700_000_000),
                link_target: None,
                is_dir: false,
                is_file: true,
                is_symlink: false,
                command_line: false,
            }],
            classic_text: "-rw-r--r-- 1 alice staff 6 ... alpha.txt\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("command".into(), CtValue::String("vdir".into())),
                ("display_format".into(), CtValue::String("long".into())),
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
                ("permissions".into(), CtValue::String("-rw-r--r--".into())),
                ("hard_links".into(), CtValue::Int(1)),
                ("owner".into(), CtValue::String("alice".into())),
                ("group".into(), CtValue::String("staff".into())),
                ("blocks".into(), CtValue::Int(8)),
                ("modified_unix_seconds".into(), CtValue::Int(1_700_000_000)),
                ("link_target".into(), CtValue::Nothing),
                ("is_dir".into(), CtValue::Bool(false)),
                ("is_file".into(), CtValue::Bool(true)),
                ("is_symlink".into(), CtValue::Bool(false)),
                ("command_line".into(), CtValue::Bool(false)),
            ])])
        );
    }

    #[test]
    fn display_columns_focus_on_verbose_listing_fields() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![
                CtValue::String("permissions".into()),
                CtValue::String("hard_links".into()),
                CtValue::String("owner".into()),
                CtValue::String("group".into()),
                CtValue::String("name".into()),
                CtValue::String("file_type".into()),
                CtValue::String("size".into()),
            ])
        );
    }
}
