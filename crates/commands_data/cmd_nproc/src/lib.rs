use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdNproc;

struct NprocIntent {
    argv: Vec<OsString>,
}

struct NprocCore;

impl NprocIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("nproc"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("nproc: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv })
    }
}

impl NprocCore {
    fn run_core(intent: &NprocIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let semantic = ct_nproc::nproc_semantic(intent.argv.iter().cloned())
            .map_err(|e| CtDiagnosticError::simple(e.to_string()).with_code(e.code()))?;
        let classic_text = semantic.selected.to_string();
        Ok((semantic_to_value(&semantic), classic_text))
    }
}

fn semantic_to_value(semantic: &ct_nproc::NprocSemantic) -> CtValue {
    CtValue::Record(vec![
        ("selected".into(), int_value(semantic.selected)),
        ("available".into(), int_value(semantic.available)),
        ("all".into(), int_value(semantic.all)),
        ("ignore".into(), int_value(semantic.ignore)),
        (
            "query".into(),
            CtValue::String(query_name(semantic.query).to_string()),
        ),
        (
            "thread_limit".into(),
            match semantic.thread_limit {
                Some(value) => int_value(value),
                None => CtValue::Nothing,
            },
        ),
    ])
}

fn int_value(value: usize) -> CtValue {
    CtValue::Int(i64::try_from(value).expect("cpu count fits in i64"))
}

fn query_name(query: ct_nproc::NprocQuery) -> &'static str {
    match query {
        ct_nproc::NprocQuery::Available => "available",
        ct_nproc::NprocQuery::All => "all",
    }
}

impl DataCommand for CmdNproc {
    fn signature(&self) -> DataSignature {
        DataSignature::new("nproc", "structured processing-unit information")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible nproc arguments",
                CtType::Any,
            ))
            .input(CtType::Nothing)
            .output(CtType::Record)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = NprocIntent::from_call(call)?;
        let (value, classic_text) = NprocCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: true,
                stderr_text: None,
                exit_code: 0,
                source: Some("nproc".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{NprocIntent, query_name, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("--all".into()), None)],
            ..DataCall::named("nproc")
        };

        let intent = NprocIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("nproc"), OsString::from("--all")]
        );
    }

    #[test]
    fn query_name_uses_public_query_strings() {
        assert_eq!(query_name(ct_nproc::NprocQuery::Available), "available");
        assert_eq!(query_name(ct_nproc::NprocQuery::All), "all");
    }

    #[test]
    fn semantic_to_value_renders_stable_record_shape() {
        let value = semantic_to_value(&ct_nproc::NprocSemantic {
            query: ct_nproc::NprocQuery::Available,
            selected: 4,
            available: 4,
            all: 8,
            ignore: 0,
            thread_limit: None,
        });

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("selected".into(), CtValue::Int(4)),
                ("available".into(), CtValue::Int(4)),
                ("all".into(), CtValue::Int(8)),
                ("ignore".into(), CtValue::Int(0)),
                ("query".into(), CtValue::String("available".into())),
                ("thread_limit".into(), CtValue::Nothing),
            ])
        );
    }
}
