use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdDf;

struct DfIntent {
    argv: Vec<OsString>,
}

struct DfCore;

impl DfIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("df"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("df: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl DfCore {
    fn run_core(intent: &DfIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_df::df_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((semantic_to_value(&semantic), semantic.classic_text))
    }
}

fn semantic_to_value(semantic: &ct_df::DfSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(row, &semantic.selected_fields))
            .collect(),
    )
}

fn row_to_value(row: &ct_df::DfSemanticRow, selected_fields: &[ct_df::DfSemanticField]) -> CtValue {
    let mut fields = Vec::with_capacity(selected_fields.len() + 2);
    fields.push((
        "row_kind".into(),
        CtValue::String(row_kind_name(row.row_kind).to_string()),
    ));
    fields.push((
        "is_total".into(),
        CtValue::Bool(row.row_kind == ct_df::DfRowKind::Total),
    ));

    for field in selected_fields {
        fields.push((field_name(*field).into(), field_value(row, *field)));
    }

    CtValue::Record(fields)
}

fn row_kind_name(kind: ct_df::DfRowKind) -> &'static str {
    match kind {
        ct_df::DfRowKind::Filesystem => "filesystem",
        ct_df::DfRowKind::Total => "total",
    }
}

fn field_name(field: ct_df::DfSemanticField) -> &'static str {
    match field {
        ct_df::DfSemanticField::Source => "source",
        ct_df::DfSemanticField::Fstype => "fstype",
        ct_df::DfSemanticField::Itotal => "itotal",
        ct_df::DfSemanticField::Iused => "iused",
        ct_df::DfSemanticField::Iavail => "iavail",
        ct_df::DfSemanticField::Ipcent => "ipcent",
        ct_df::DfSemanticField::Size => "size",
        ct_df::DfSemanticField::Used => "used",
        ct_df::DfSemanticField::Avail => "avail",
        ct_df::DfSemanticField::Pcent => "pcent",
        ct_df::DfSemanticField::File => "file",
        ct_df::DfSemanticField::Target => "target",
    }
}

fn field_value(row: &ct_df::DfSemanticRow, field: ct_df::DfSemanticField) -> CtValue {
    if let Some(value) = row.display_values.get(&field) {
        return CtValue::String(value.clone());
    }

    match field {
        ct_df::DfSemanticField::Source => string_or_nothing(row.source.as_ref()),
        ct_df::DfSemanticField::Fstype => string_or_nothing(row.fstype.as_ref()),
        ct_df::DfSemanticField::Itotal => int_or_nothing_u128(row.itotal),
        ct_df::DfSemanticField::Iused => int_or_nothing_u128(row.iused),
        ct_df::DfSemanticField::Iavail => int_or_nothing_u128(row.iavail),
        ct_df::DfSemanticField::Ipcent => int_or_nothing_u64(row.ipcent),
        ct_df::DfSemanticField::Size => int_or_nothing_u64(row.size),
        ct_df::DfSemanticField::Used => int_or_nothing_u64(row.used),
        ct_df::DfSemanticField::Avail => int_or_nothing_u64(row.avail),
        ct_df::DfSemanticField::Pcent => int_or_nothing_u64(row.pcent),
        ct_df::DfSemanticField::File => string_or_nothing(row.file.as_ref()),
        ct_df::DfSemanticField::Target => string_or_nothing(row.target.as_ref()),
    }
}

fn string_or_nothing(value: Option<&String>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.clone()),
        None => CtValue::Nothing,
    }
}

fn int_or_nothing_u64(value: Option<u64>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(i64::try_from(value).expect("df value fits in i64")),
        None => CtValue::Nothing,
    }
}

fn int_or_nothing_u128(value: Option<u128>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(i64::try_from(value).expect("df value fits in i64")),
        None => CtValue::Nothing,
    }
}

impl DataCommand for CmdDf {
    fn signature(&self) -> DataSignature {
        DataSignature::new("df", "structured file system usage")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible df arguments",
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
        let intent = DfIntent::from_call(call)?;
        let (value, classic_text) = DfCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: true,
                stderr_text: None,
                exit_code: 0,
                source: Some("df".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{DfIntent, field_name, row_kind_name, row_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("--output=source,target".into()), None),
                BoundArg::new(CtValue::String(".".into()), None),
            ],
            ..DataCall::named("df")
        };

        let intent = DfIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("df"),
                OsString::from("--output=source,target"),
                OsString::from(".")
            ]
        );
    }

    #[test]
    fn row_kind_name_uses_stable_strings() {
        assert_eq!(row_kind_name(ct_df::DfRowKind::Filesystem), "filesystem");
        assert_eq!(row_kind_name(ct_df::DfRowKind::Total), "total");
    }

    #[test]
    fn field_name_matches_public_contract() {
        assert_eq!(field_name(ct_df::DfSemanticField::Source), "source");
        assert_eq!(field_name(ct_df::DfSemanticField::Target), "target");
    }

    #[test]
    fn row_to_value_projects_selected_fields() {
        let value = row_to_value(
            &ct_df::DfSemanticRow {
                row_kind: ct_df::DfRowKind::Filesystem,
                source: Some("/dev/root".into()),
                fstype: Some("ext4".into()),
                itotal: Some(8),
                iused: Some(2),
                iavail: Some(6),
                ipcent: Some(25),
                size: Some(1024),
                used: Some(256),
                avail: Some(768),
                pcent: Some(25),
                file: None,
                target: Some("/".into()),
                display_values: Default::default(),
            },
            &[
                ct_df::DfSemanticField::Source,
                ct_df::DfSemanticField::Target,
            ],
        );

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("row_kind".into(), CtValue::String("filesystem".into())),
                ("is_total".into(), CtValue::Bool(false)),
                ("source".into(), CtValue::String("/dev/root".into())),
                ("target".into(), CtValue::String("/".into())),
            ])
        );
    }

    #[test]
    fn row_to_value_prefers_display_values_for_scaled_fields() {
        let mut display_values = std::collections::BTreeMap::new();
        display_values.insert(ct_df::DfSemanticField::Size, "69G".into());
        display_values.insert(ct_df::DfSemanticField::Used, "56G".into());
        display_values.insert(ct_df::DfSemanticField::Avail, "10G".into());

        let value = row_to_value(
            &ct_df::DfSemanticRow {
                row_kind: ct_df::DfRowKind::Filesystem,
                source: Some("/dev/root".into()),
                fstype: Some("ext4".into()),
                itotal: None,
                iused: None,
                iavail: None,
                ipcent: None,
                size: Some(73_650_106_368),
                used: Some(60_090_609_664),
                avail: Some(10_443_575_296),
                pcent: Some(86),
                file: None,
                target: Some("/".into()),
                display_values,
            },
            &[
                ct_df::DfSemanticField::Size,
                ct_df::DfSemanticField::Used,
                ct_df::DfSemanticField::Avail,
            ],
        );

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("row_kind".into(), CtValue::String("filesystem".into())),
                ("is_total".into(), CtValue::Bool(false)),
                ("size".into(), CtValue::String("69G".into())),
                ("used".into(), CtValue::String("56G".into())),
                ("avail".into(), CtValue::String("10G".into())),
            ])
        );
    }
}
