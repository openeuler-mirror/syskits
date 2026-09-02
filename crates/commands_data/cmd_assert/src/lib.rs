/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_assert` — 断言条件成立，否则让管线失败。

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctengine::execution::{CommandCore, CommandRunner};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};

#[derive(Default)]
pub struct CmdAssert;

struct AssertCore;

const ASSERT_HELP: &str = r#"syskits data assert

This is the syskits structured data pipeline assert command.
It fails the pipeline unless a condition or input value is truthy.

Usage:
  assert [condition] [message]
  assert --help
  assert --version

Examples:
  assert true
  from json '{"ok":true}' | get ok | assert
  assert false "expected condition to pass"
"#;

impl DataCommand for CmdAssert {
    fn signature(&self) -> DataSignature {
        DataSignature::new(
            "assert",
            "assert condition is true, otherwise fail the pipeline",
        )
        .positional(CtPositionalArg::optional(
            "condition",
            "condition to assert (defaults to pipeline input value)",
            CtType::Any,
        ))
        .positional(CtPositionalArg::optional(
            "message",
            "custom failure message",
            CtType::String,
        ))
        .flag(CtFlag::switch(
            "help",
            Some('h'),
            "show help for syskits data assert",
        ))
        .flag(CtFlag::switch(
            "version",
            None,
            "show syskits data assert version",
        ))
        .input(CtType::Any)
        .output(CtType::Any)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        if call.has_flag("help") || call.has_flag("h") {
            return Ok(meta_text_output(ASSERT_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data assert {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        CommandRunner::run(&AssertCore, call, input, ctx)
    }
}

fn meta_text_output(text: String) -> CtPipelineData {
    CtPipelineData::Value(
        CtValue::String(text.clone()),
        CtPipelineMetadata {
            classic_text: Some(text),
            classic_bytes: None,
            classic_append_newline: false,
            exit_code: 0,
            source: Some("assert".into()),
            ..Default::default()
        },
    )
}

impl CommandCore for AssertCore {
    fn run_core(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let cond_arg: Option<CtValue> = call
            .opt(0)
            .map_err(|e| CtDiagnosticError::simple(format!("assert: {e}")))?;
        let msg_arg: Option<String> = call
            .opt(1)
            .map_err(|e| CtDiagnosticError::simple(format!("assert: {e}")))?;

        let passed = if let Some(ref cond) = cond_arg {
            truthy(cond)?
        } else {
            match &input {
                CtPipelineData::Empty => {
                    return Err(CtDiagnosticError::simple(
                        "assert: missing condition (provide an argument or pipe in a value)",
                    ));
                }
                CtPipelineData::Value(v, _) => truthy(v)?,
                CtPipelineData::ListStream(_) | CtPipelineData::ByteStream(_) => {
                    return Err(CtDiagnosticError::simple(
                        "assert: streaming input requires an explicit condition argument",
                    ));
                }
            }
        };

        if !passed {
            let msg = msg_arg.unwrap_or_else(|| "assert: condition evaluated to false".to_string());
            return Err(CtDiagnosticError::simple(msg));
        }

        if input.is_empty() {
            return Ok(CtPipelineData::Value(
                CtValue::Bool(true),
                CtPipelineMetadata::builtin("assert"),
            ));
        }
        Ok(input)
    }
}

fn truthy(value: &CtValue) -> Result<bool, CtDiagnosticError> {
    let result = match value {
        CtValue::Nothing => false,
        CtValue::Bool(v) => *v,
        CtValue::Int(v) => *v != 0,
        CtValue::Float(v) => !v.is_nan() && *v != 0.0,
        CtValue::String(v) => !v.trim().is_empty(),
        CtValue::Binary(v) => !v.is_empty(),
        CtValue::DateTime(v) => *v != 0,
        CtValue::Duration(v) => *v != 0,
        CtValue::Size(v) => *v != 0,
        CtValue::Record(v) => !v.is_empty(),
        CtValue::List(v) => !v.is_empty(),
        CtValue::Error(err) => {
            return Err(CtDiagnosticError::simple(format!(
                "assert: cannot evaluate error value: {err}"
            )));
        }
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn call(args: &[CtValue]) -> DataCall {
        let mut c = DataCall::named("assert");
        for v in args {
            c.positionals.push(ctsig::BoundArg::new(v.clone(), None));
        }
        c
    }

    #[test]
    fn assert_with_true_arg_returns_true_on_empty_input() {
        let out = CmdAssert
            .run(&call(&[CtValue::Bool(true)]), CtPipelineData::Empty, &ctx())
            .expect("assert should pass");
        assert!(matches!(out, CtPipelineData::Value(CtValue::Bool(true), _)));
    }

    #[test]
    fn assert_false_with_message_fails() {
        let err = CmdAssert
            .run(
                &call(&[CtValue::Bool(false), CtValue::String("boom".to_string())]),
                CtPipelineData::Empty,
                &ctx(),
            )
            .expect_err("assert should fail");
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn assert_uses_input_when_no_condition_arg() {
        let input = CtPipelineData::Value(CtValue::Int(1), CtPipelineMetadata::default());
        let out = CmdAssert
            .run(&call(&[]), input, &ctx())
            .expect("assert should pass for non-zero input");
        assert!(matches!(out, CtPipelineData::Value(CtValue::Int(1), _)));
    }

    #[test]
    fn assert_input_false_fails() {
        let input = CtPipelineData::Value(CtValue::Int(0), CtPipelineMetadata::default());
        let err = CmdAssert
            .run(&call(&[]), input, &ctx())
            .expect_err("assert should fail");
        assert!(err.to_string().contains("condition evaluated to false"));
    }
}
