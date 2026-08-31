use ctcore::ct_error::CTError;
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdFactor;

struct FactorIntent {
    argv: Vec<OsString>,
}

struct FactorCore;

impl FactorIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("factor"));

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

impl FactorCore {
    fn run_core(
        intent: &FactorIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = ctengine::run_with_optional_pipeline_stdin(
            "factor",
            input,
            ctengine::argv_uses_stdin(&intent.argv, &[]),
            || {
                Ok(ct_factor::factor_native_semantic(
                    intent.argv.iter().cloned(),
                ))
            },
            |err| CtDiagnosticError::simple(format!("factor: {err}")),
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
    let mut stderr = format!("factor: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'factor --help' for more information.\n");
    }
    stderr
}

fn semantic_to_value(semantic: &ct_factor::FactorSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_factor::FactorRow) -> CtValue {
    CtValue::Record(vec![
        ("number".into(), CtValue::String(row.number.clone())),
        (
            "factors".into(),
            CtValue::List(row.factors.iter().cloned().map(CtValue::String).collect()),
        ),
        (
            "factor_powers".into(),
            CtValue::List(
                row.factor_powers
                    .iter()
                    .map(|power| {
                        CtValue::Record(vec![
                            ("prime".into(), CtValue::String(power.prime.clone())),
                            ("exponent".into(), CtValue::Int(i64::from(power.exponent))),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("print_exponents".into(), CtValue::Bool(row.print_exponents)),
    ])
}

impl DataCommand for CmdFactor {
    fn signature(&self) -> DataSignature {
        DataSignature::new("factor", "structured prime factorization output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible factor arguments",
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
        let intent = FactorIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = FactorCore::run_core(&intent, input)?;
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
                source: Some("factor".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{FactorIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("--exponents".into()), None),
                BoundArg::new(CtValue::String("12".into()), None),
            ],
            ..DataCall::named("factor")
        };

        let intent = FactorIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("factor"),
                OsString::from("--exponents"),
                OsString::from("12"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_factor::FactorSemantic {
            rows: vec![ct_factor::FactorRow {
                number: "12".into(),
                factors: vec!["2".into(), "2".into(), "3".into()],
                factor_powers: vec![
                    ct_factor::FactorPower {
                        prime: "2".into(),
                        exponent: 2,
                    },
                    ct_factor::FactorPower {
                        prime: "3".into(),
                        exponent: 1,
                    },
                ],
                print_exponents: true,
            }],
            classic_text: "12: 2^2 3\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("number".into(), CtValue::String("12".into())),
                (
                    "factors".into(),
                    CtValue::List(vec![
                        CtValue::String("2".into()),
                        CtValue::String("2".into()),
                        CtValue::String("3".into()),
                    ]),
                ),
                (
                    "factor_powers".into(),
                    CtValue::List(vec![
                        CtValue::Record(vec![
                            ("prime".into(), CtValue::String("2".into())),
                            ("exponent".into(), CtValue::Int(2)),
                        ]),
                        CtValue::Record(vec![
                            ("prime".into(), CtValue::String("3".into())),
                            ("exponent".into(), CtValue::Int(1)),
                        ]),
                    ]),
                ),
                ("print_exponents".into(), CtValue::Bool(true)),
            ])])
        );
    }
}
