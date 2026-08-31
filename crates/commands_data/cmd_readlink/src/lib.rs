use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdReadlink;

struct ReadlinkIntent {
    argv: Vec<OsString>,
}

struct ReadlinkCore;

impl ReadlinkIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("readlink"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple(
                    "readlink: argument must be string",
                ));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl ReadlinkCore {
    fn run_core(intent: &ReadlinkIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_readlink::readlink_native_semantic(intent.argv.iter().cloned())
            .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?;
        Ok((semantic_to_value(&semantic), semantic.classic_text))
    }
}

fn semantic_to_value(semantic: &ct_readlink::ReadlinkSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_readlink::ReadlinkSemanticRow) -> CtValue {
    CtValue::Record(vec![
        ("input".into(), CtValue::String(row.input.clone())),
        (
            "resolved_path".into(),
            CtValue::String(row.resolved_path.clone()),
        ),
        (
            "resolution_mode".into(),
            CtValue::String(mode_name(row.mode).to_string()),
        ),
        ("no_newline".into(), CtValue::Bool(row.no_newline)),
        ("zero".into(), CtValue::Bool(row.zero)),
        ("quiet".into(), CtValue::Bool(row.quiet)),
        ("verbose".into(), CtValue::Bool(row.verbose)),
    ])
}

fn mode_name(mode: ct_readlink::ReadlinkMode) -> &'static str {
    match mode {
        ct_readlink::ReadlinkMode::Readlink => "readlink",
        ct_readlink::ReadlinkMode::Canonicalize => "canonicalize",
        ct_readlink::ReadlinkMode::CanonicalizeExisting => "canonicalize_existing",
        ct_readlink::ReadlinkMode::CanonicalizeMissing => "canonicalize_missing",
    }
}

impl DataCommand for CmdReadlink {
    fn signature(&self) -> DataSignature {
        DataSignature::new(
            "readlink",
            "structured symlink and canonicalized path resolution",
        )
        .rest(CtPositionalArg::optional(
            "arg",
            "GNU-compatible readlink arguments",
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
        let intent = ReadlinkIntent::from_call(call)?;
        let (value, classic_text) = ReadlinkCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: None,
                exit_code: 0,
                source: Some("readlink".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadlinkIntent, mode_name, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-f".into()), None),
                BoundArg::new(CtValue::String("link".into()), None),
            ],
            ..DataCall::named("readlink")
        };

        let intent = ReadlinkIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("readlink"),
                OsString::from("-f"),
                OsString::from("link")
            ]
        );
    }

    #[test]
    fn mode_name_uses_public_strings() {
        assert_eq!(mode_name(ct_readlink::ReadlinkMode::Readlink), "readlink");
        assert_eq!(
            mode_name(ct_readlink::ReadlinkMode::Canonicalize),
            "canonicalize"
        );
        assert_eq!(
            mode_name(ct_readlink::ReadlinkMode::CanonicalizeExisting),
            "canonicalize_existing"
        );
        assert_eq!(
            mode_name(ct_readlink::ReadlinkMode::CanonicalizeMissing),
            "canonicalize_missing"
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_readlink::ReadlinkSemantic {
            rows: vec![ct_readlink::ReadlinkSemanticRow {
                input: "link".into(),
                resolved_path: "/tmp/target".into(),
                mode: ct_readlink::ReadlinkMode::Canonicalize,
                no_newline: false,
                zero: false,
                quiet: false,
                verbose: false,
            }],
            classic_text: "/tmp/target\n".into(),
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("input".into(), CtValue::String("link".into())),
                (
                    "resolved_path".into(),
                    CtValue::String("/tmp/target".into())
                ),
                (
                    "resolution_mode".into(),
                    CtValue::String("canonicalize".into())
                ),
                ("no_newline".into(), CtValue::Bool(false)),
                ("zero".into(), CtValue::Bool(false)),
                ("quiet".into(), CtValue::Bool(false)),
                ("verbose".into(), CtValue::Bool(false)),
            ])])
        );
    }
}
