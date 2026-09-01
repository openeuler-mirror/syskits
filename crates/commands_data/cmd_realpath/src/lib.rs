use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdRealpath;

struct RealpathIntent {
    argv: Vec<OsString>,
}

struct RealpathCore;

impl RealpathIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("realpath"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple(
                    "realpath: argument must be string",
                ));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl RealpathCore {
    fn run_core(intent: &RealpathIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_realpath::realpath_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((semantic_to_value(&semantic), semantic.classic_text))
    }
}

fn semantic_to_value(semantic: &ct_realpath::RealpathSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn display_columns() -> CtValue {
    CtValue::List(
        ["input", "resolved_path", "output_path", "resolution_mode"]
            .into_iter()
            .map(|name| CtValue::String(name.into()))
            .collect(),
    )
}

fn row_to_value(row: &ct_realpath::RealpathSemanticRow) -> CtValue {
    CtValue::Record(vec![
        ("input".into(), CtValue::String(row.input.clone())),
        (
            "resolved_path".into(),
            CtValue::String(row.resolved_path.clone()),
        ),
        (
            "output_path".into(),
            CtValue::String(row.output_path.clone()),
        ),
        (
            "resolution_mode".into(),
            CtValue::String(resolution_mode_name(row.resolution_mode).to_string()),
        ),
        (
            "missing_handling".into(),
            CtValue::String(missing_handling_name(row.missing_handling).to_string()),
        ),
    ])
}

fn resolution_mode_name(mode: ct_realpath::RealpathResolutionMode) -> &'static str {
    match mode {
        ct_realpath::RealpathResolutionMode::None => "none",
        ct_realpath::RealpathResolutionMode::Physical => "physical",
        ct_realpath::RealpathResolutionMode::Logical => "logical",
    }
}

fn missing_handling_name(mode: ct_realpath::RealpathMissingHandling) -> &'static str {
    match mode {
        ct_realpath::RealpathMissingHandling::Normal => "normal",
        ct_realpath::RealpathMissingHandling::Existing => "existing",
        ct_realpath::RealpathMissingHandling::Missing => "missing",
    }
}

impl DataCommand for CmdRealpath {
    fn signature(&self) -> DataSignature {
        DataSignature::new(
            "realpath",
            "structured absolute and relative path resolution",
        )
        .rest(CtPositionalArg::optional(
            "arg",
            "GNU-compatible realpath arguments",
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
        let intent = RealpathIntent::from_call(call)?;
        let (value, classic_text) = RealpathCore::run_core(&intent)?;
        let metadata = CtPipelineMetadata {
            classic_text: Some(classic_text),
            classic_bytes: None,
            classic_append_newline: false,
            stderr_text: None,
            exit_code: 0,
            source: Some("realpath".into()),
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
    use super::{
        RealpathIntent, display_columns, missing_handling_name, resolution_mode_name,
        semantic_to_value,
    };
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("--relative-to".into()), None),
                BoundArg::new(CtValue::String("/tmp".into()), None),
                BoundArg::new(CtValue::String("file".into()), None),
            ],
            ..DataCall::named("realpath")
        };

        let intent = RealpathIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("realpath"),
                OsString::from("--relative-to"),
                OsString::from("/tmp"),
                OsString::from("file")
            ]
        );
    }

    #[test]
    fn mode_names_are_stable() {
        assert_eq!(
            resolution_mode_name(ct_realpath::RealpathResolutionMode::None),
            "none"
        );
        assert_eq!(
            resolution_mode_name(ct_realpath::RealpathResolutionMode::Physical),
            "physical"
        );
        assert_eq!(
            resolution_mode_name(ct_realpath::RealpathResolutionMode::Logical),
            "logical"
        );
        assert_eq!(
            missing_handling_name(ct_realpath::RealpathMissingHandling::Normal),
            "normal"
        );
        assert_eq!(
            missing_handling_name(ct_realpath::RealpathMissingHandling::Existing),
            "existing"
        );
        assert_eq!(
            missing_handling_name(ct_realpath::RealpathMissingHandling::Missing),
            "missing"
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_realpath::RealpathSemantic {
            rows: vec![ct_realpath::RealpathSemanticRow {
                input: "file".into(),
                resolved_path: "/tmp/file".into(),
                output_path: "file".into(),
                resolution_mode: ct_realpath::RealpathResolutionMode::Physical,
                missing_handling: ct_realpath::RealpathMissingHandling::Normal,
            }],
            classic_text: "file\n".into(),
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("input".into(), CtValue::String("file".into())),
                ("resolved_path".into(), CtValue::String("/tmp/file".into())),
                ("output_path".into(), CtValue::String("file".into())),
                ("resolution_mode".into(), CtValue::String("physical".into())),
                ("missing_handling".into(), CtValue::String("normal".into())),
            ])])
        );
    }

    #[test]
    fn display_columns_hide_missing_handling() {
        let CtValue::List(columns) = display_columns() else {
            panic!("expected list value");
        };
        let names = columns
            .into_iter()
            .map(|value| match value {
                CtValue::String(name) => name,
                other => panic!("unexpected display column: {other:?}"),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec!["input", "resolved_path", "output_path", "resolution_mode",]
        );
    }
}
