use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdNl;

struct NlIntent {
    argv: Vec<OsString>,
}

struct NlCore;

impl NlIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("nl"));

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

impl NlCore {
    fn run_core(
        intent: &NlIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "nl",
            input,
            ctengine::argv_uses_stdin(
                &intent.argv,
                &[
                    "-b",
                    "--body-numbering",
                    "-d",
                    "--section-delimiter",
                    "-f",
                    "--footer-numbering",
                    "-h",
                    "--header-numbering",
                    "-i",
                    "--line-increment",
                    "-l",
                    "--join-blank-lines",
                    "-n",
                    "--number-format",
                    "-s",
                    "--number-separator",
                    "-v",
                    "--starting-line-number",
                    "-w",
                    "--number-width",
                ],
            ),
            || Ok(ct_nl::nl_native_semantic(intent.argv.iter().cloned())),
            |err| CtDiagnosticError::simple(format!("nl: {err}")),
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
    let mut stderr = format!("nl: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'nl --help' for more information.\n");
    }
    stderr
}

fn semantic_to_value(semantic: &ct_nl::NlSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_nl::NlRow) -> CtValue {
    let mut fields = vec![
        ("kind".into(), CtValue::String(row.kind.clone())),
        ("section".into(), CtValue::String(row.section.clone())),
        ("numbered".into(), CtValue::Bool(row.numbered)),
        ("text".into(), CtValue::String(row.text.clone())),
    ];

    fields.push((
        "line_number".into(),
        match row.line_number {
            Some(value) => CtValue::Int(value),
            None => CtValue::Nothing,
        },
    ));
    fields.push((
        "rendered_number".into(),
        match &row.rendered_number {
            Some(value) => CtValue::String(value.clone()),
            None => CtValue::Nothing,
        },
    ));

    CtValue::Record(fields)
}

impl DataCommand for CmdNl {
    fn signature(&self) -> DataSignature {
        DataSignature::new("nl", "structured numbered line output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible nl arguments",
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
        let intent = NlIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = NlCore::run_core(&intent, input)?;
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
                source: Some("nl".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{NlIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("-ba".into()), None)],
            ..DataCall::named("nl")
        };

        let intent = NlIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("nl"), OsString::from("-ba")]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_nl::NlSemantic {
            rows: vec![ct_nl::NlRow {
                kind: "line".into(),
                section: "body".into(),
                numbered: true,
                line_number: Some(1),
                rendered_number: Some("     1".into()),
                text: "Line 1".into(),
            }],
            classic_text: "     1\tLine 1\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("kind".into(), CtValue::String("line".into())),
                ("section".into(), CtValue::String("body".into())),
                ("numbered".into(), CtValue::Bool(true)),
                ("text".into(), CtValue::String("Line 1".into())),
                ("line_number".into(), CtValue::Int(1)),
                ("rendered_number".into(), CtValue::String("     1".into())),
            ])])
        );
    }
}
