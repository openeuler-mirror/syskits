use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdTr;

struct TrIntent {
    argv: Vec<OsString>,
}

struct TrCore;

impl TrIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("tr"));

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

impl TrCore {
    fn run_core(
        intent: &TrIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "tr",
            input,
            true,
            || Ok(ct_tr::tr_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("tr: {err}")),
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
    let mut stderr = format!("tr: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'tr --help' for more information.\n");
    }
    stderr
}

fn opt_string_to_value(value: Option<&str>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.into()),
        None => CtValue::Nothing,
    }
}

fn row_to_value(semantic: &ct_tr::TrSemantic, row: &ct_tr::TrRow) -> CtValue {
    CtValue::Record(vec![
        (
            "operation".into(),
            CtValue::String(semantic.operation.clone()),
        ),
        ("complement".into(), CtValue::Bool(semantic.complement)),
        ("delete".into(), CtValue::Bool(semantic.delete)),
        ("squeeze".into(), CtValue::Bool(semantic.squeeze)),
        (
            "truncate_set1".into(),
            CtValue::Bool(semantic.truncate_set1),
        ),
        ("set1".into(), opt_string_to_value(semantic.set1.as_deref())),
        ("set2".into(), opt_string_to_value(semantic.set2.as_deref())),
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("line".into(), CtValue::String(row.line.clone())),
        (
            "byte_len".into(),
            CtValue::Int(i64::try_from(row.byte_len).expect("byte len fits")),
        ),
        ("terminated".into(), CtValue::Bool(row.terminated)),
    ])
}

fn semantic_to_value(semantic: &ct_tr::TrSemantic) -> CtValue {
    CtValue::List(
        semantic
            .rows
            .iter()
            .map(|row| row_to_value(semantic, row))
            .collect(),
    )
}

impl DataCommand for CmdTr {
    fn signature(&self) -> DataSignature {
        DataSignature::new("tr", "structured translated text rows")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible tr arguments",
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
        let intent = TrIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = TrCore::run_core(&intent, input)?;
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
                source: Some("tr".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{TrIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("a-z".into()), None),
                BoundArg::new(CtValue::String("A-Z".into()), None),
            ],
            ..DataCall::named("tr")
        };

        let intent = TrIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("tr"),
                OsString::from("a-z"),
                OsString::from("A-Z")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_tr::TrSemantic {
            operation: "translate".into(),
            complement: false,
            delete: false,
            squeeze: false,
            truncate_set1: false,
            set1: Some("a-z".into()),
            set2: Some("A-Z".into()),
            rows: vec![ct_tr::TrRow {
                row_index: 1,
                line: "ALPHA\n".into(),
                byte_len: 6,
                terminated: true,
            }],
            classic_text: "ALPHA\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("operation".into(), CtValue::String("translate".into())),
                ("complement".into(), CtValue::Bool(false)),
                ("delete".into(), CtValue::Bool(false)),
                ("squeeze".into(), CtValue::Bool(false)),
                ("truncate_set1".into(), CtValue::Bool(false)),
                ("set1".into(), CtValue::String("a-z".into())),
                ("set2".into(), CtValue::String("A-Z".into())),
                ("row_index".into(), CtValue::Int(1)),
                ("line".into(), CtValue::String("ALPHA\n".into())),
                ("byte_len".into(), CtValue::Int(6)),
                ("terminated".into(), CtValue::Bool(true)),
            ])])
        );
    }
}
