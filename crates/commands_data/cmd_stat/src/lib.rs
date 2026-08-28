use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdStat;

struct StatIntent {
    argv: Vec<OsString>,
    classic_append_newline: bool,
}

struct StatCore;

impl StatIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("stat"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("stat: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        let classic_append_newline = !call.positionals.iter().any(|arg| match &arg.value {
            CtValue::String(arg) => arg == "--printf" || arg.starts_with("--printf="),
            _ => false,
        });

        Ok(Self {
            argv,
            classic_append_newline,
        })
    }
}

impl StatCore {
    fn run_core(intent: &StatIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_stat::stat_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((semantic_to_value(&semantic), semantic.classic_text))
    }
}

fn semantic_to_value(semantic: &ct_stat::StatSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(row, &semantic.selected_fields))
            .collect(),
    )
}

fn row_to_value(
    row: &ct_stat::StatSemanticRow,
    selected_fields: &[ct_stat::StatSemanticField],
) -> CtValue {
    let mut fields = Vec::with_capacity(selected_fields.len() + 2);
    fields.push((
        "row_kind".into(),
        CtValue::String(row_kind_name(row.row_kind).to_string()),
    ));
    fields.push((
        "is_filesystem".into(),
        CtValue::Bool(row.row_kind == ct_stat::StatRowKind::Filesystem),
    ));

    for field in selected_fields {
        fields.push((field_name(*field).into(), field_value(row, *field)));
    }

    CtValue::Record(fields)
}

fn row_kind_name(kind: ct_stat::StatRowKind) -> &'static str {
    match kind {
        ct_stat::StatRowKind::File => "file",
        ct_stat::StatRowKind::Filesystem => "filesystem",
    }
}

fn field_name(field: ct_stat::StatSemanticField) -> &'static str {
    match field {
        ct_stat::StatSemanticField::Name => "name",
        ct_stat::StatSemanticField::QuotedName => "quoted_name",
        ct_stat::StatSemanticField::Size => "size",
        ct_stat::StatSemanticField::Blocks => "blocks",
        ct_stat::StatSemanticField::BlockSizeReported => "reported_block_size",
        ct_stat::StatSemanticField::IoBlock => "io_block",
        ct_stat::StatSemanticField::RawModeHex => "raw_mode_hex",
        ct_stat::StatSemanticField::FileType => "file_type",
        ct_stat::StatSemanticField::Uid => "uid",
        ct_stat::StatSemanticField::User => "user",
        ct_stat::StatSemanticField::Gid => "gid",
        ct_stat::StatSemanticField::Group => "group",
        ct_stat::StatSemanticField::Device => "device",
        ct_stat::StatSemanticField::DeviceHex => "device_hex",
        ct_stat::StatSemanticField::DeviceMajor => "device_major",
        ct_stat::StatSemanticField::DeviceMinor => "device_minor",
        ct_stat::StatSemanticField::DeviceMajorHex => "device_major_hex",
        ct_stat::StatSemanticField::DeviceMinorHex => "device_minor_hex",
        ct_stat::StatSemanticField::DeviceType => "device_type",
        ct_stat::StatSemanticField::DeviceTypeHex => "device_type_hex",
        ct_stat::StatSemanticField::DeviceTypeMajor => "device_type_major",
        ct_stat::StatSemanticField::DeviceTypeMinor => "device_type_minor",
        ct_stat::StatSemanticField::DeviceTypeMajorHex => "device_type_major_hex",
        ct_stat::StatSemanticField::DeviceTypeMinorHex => "device_type_minor_hex",
        ct_stat::StatSemanticField::Inode => "inode",
        ct_stat::StatSemanticField::Links => "links",
        ct_stat::StatSemanticField::MountPoint => "mount_point",
        ct_stat::StatSemanticField::AccessRightsOctal => "access_rights_octal",
        ct_stat::StatSemanticField::AccessRightsHuman => "access_rights_human",
        ct_stat::StatSemanticField::Context => "context",
        ct_stat::StatSemanticField::AccessTime => "access_time",
        ct_stat::StatSemanticField::AccessEpoch => "access_epoch",
        ct_stat::StatSemanticField::ModifyTime => "modify_time",
        ct_stat::StatSemanticField::ModifyEpoch => "modify_epoch",
        ct_stat::StatSemanticField::ChangeTime => "change_time",
        ct_stat::StatSemanticField::ChangeEpoch => "change_epoch",
        ct_stat::StatSemanticField::BirthTime => "birth_time",
        ct_stat::StatSemanticField::BirthEpoch => "birth_epoch",
        ct_stat::StatSemanticField::FilesystemIdHex => "filesystem_id_hex",
        ct_stat::StatSemanticField::NameMax => "name_max",
        ct_stat::StatSemanticField::FilesystemTypeHex => "filesystem_type_hex",
        ct_stat::StatSemanticField::FilesystemType => "filesystem_type",
        ct_stat::StatSemanticField::BlockSize => "block_size",
        ct_stat::StatSemanticField::FundamentalBlockSize => "fundamental_block_size",
        ct_stat::StatSemanticField::TotalBlocks => "total_blocks",
        ct_stat::StatSemanticField::FreeBlocks => "free_blocks",
        ct_stat::StatSemanticField::AvailableBlocks => "available_blocks",
        ct_stat::StatSemanticField::TotalFileNodes => "total_file_nodes",
        ct_stat::StatSemanticField::FreeFileNodes => "free_file_nodes",
        ct_stat::StatSemanticField::Formatted => "formatted",
    }
}

fn field_value(row: &ct_stat::StatSemanticRow, field: ct_stat::StatSemanticField) -> CtValue {
    match row.fields.iter().find(|(existing, _)| *existing == field) {
        Some((_, ct_stat::StatSemanticValue::String(value))) => CtValue::String(value.clone()),
        Some((_, ct_stat::StatSemanticValue::Int(value))) => CtValue::Int(*value),
        None => CtValue::Nothing,
    }
}

impl DataCommand for CmdStat {
    fn signature(&self) -> DataSignature {
        DataSignature::new("stat", "structured file and file system status")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = StatIntent::from_call(call)?;
        let (value, classic_text) = StatCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: intent.classic_append_newline,
                stderr_text: None,
                exit_code: 0,
                source: Some("stat".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{StatIntent, field_name, row_kind_name, row_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-f".into()), None),
                BoundArg::new(CtValue::String("file".into()), None),
            ],
            ..DataCall::named("stat")
        };

        let intent = StatIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("stat"),
                OsString::from("-f"),
                OsString::from("file")
            ]
        );
    }

    #[test]
    fn row_kind_name_uses_public_strings() {
        assert_eq!(row_kind_name(ct_stat::StatRowKind::File), "file");
        assert_eq!(
            row_kind_name(ct_stat::StatRowKind::Filesystem),
            "filesystem"
        );
    }

    #[test]
    fn field_name_matches_contract() {
        assert_eq!(field_name(ct_stat::StatSemanticField::Name), "name");
        assert_eq!(
            field_name(ct_stat::StatSemanticField::FilesystemType),
            "filesystem_type"
        );
    }

    #[test]
    fn row_to_value_projects_selected_fields() {
        let value = row_to_value(
            &ct_stat::StatSemanticRow {
                row_kind: ct_stat::StatRowKind::File,
                fields: vec![
                    (
                        ct_stat::StatSemanticField::Name,
                        ct_stat::StatSemanticValue::String("sample".into()),
                    ),
                    (
                        ct_stat::StatSemanticField::Size,
                        ct_stat::StatSemanticValue::Int(12),
                    ),
                ],
            },
            &[
                ct_stat::StatSemanticField::Name,
                ct_stat::StatSemanticField::Size,
            ],
        );

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("row_kind".into(), CtValue::String("file".into())),
                ("is_filesystem".into(), CtValue::Bool(false)),
                ("name".into(), CtValue::String("sample".into())),
                ("size".into(), CtValue::Int(12)),
            ])
        );
    }
}
