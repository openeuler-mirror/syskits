/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_retry` — 对表达式进行重试执行（面向运行时失败）。

use ctengine::ReusableInput;
use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctengine::execution::{CommandCore, CommandRunner};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::time::Duration;

#[derive(Default)]
pub struct CmdRetry;

struct RetryCore;
const INTERRUPT_POLL_INTERVAL: Duration = Duration::from_millis(50);

const RETRY_HELP: &str = r#"syskits data retry

This is the syskits structured data pipeline retry command.
It retries an expression when it fails with a runtime error.

Usage:
  retry <expr> [attempts] [interval]
  retry --help
  retry --version

Defaults:
  attempts: 3
  interval: 1s

Examples:
  retry 'http https://example.com' 5 1s
  retry 'assert true' 3 100ms
"#;

impl DataCommand for CmdRetry {
    fn signature(&self) -> DataSignature {
        DataSignature::new("retry", "retry expression on runtime errors")
            .positional(CtPositionalArg::required(
                "expr",
                "expression to execute repeatedly until success",
                CtType::String,
            ))
            .positional(CtPositionalArg::optional(
                "attempts",
                "max attempt count (default: 3)",
                CtType::Int,
            ))
            .positional(CtPositionalArg::optional(
                "interval",
                "retry interval duration (default: 1s)",
                CtType::Duration,
            ))
            .flag(CtFlag::switch(
                "help",
                Some('h'),
                "show help for syskits data retry",
            ))
            .flag(CtFlag::switch(
                "version",
                None,
                "show syskits data retry version",
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
            return Ok(meta_text_output(RETRY_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data retry {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        CommandRunner::run(&RetryCore, call, input, ctx)
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
            source: Some("retry".into()),
            ..Default::default()
        },
    )
}

impl CommandCore for RetryCore {
    fn run_core(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let expr_text: String = call
            .req(0)
            .map_err(|e| CtDiagnosticError::simple(format!("retry: {e}")))?;
        let attempts = parse_attempts(call)?;
        let interval_ns = parse_interval_ns(call)?;
        let expr = ctdsl::parse(&expr_text).map_err(|e| {
            CtDiagnosticError::simple(format!("retry: failed to parse expr: {e}"))
                .with_code(ctengine::exit_code::USAGE_ERROR)
        })?;

        if attempts == 1 {
            return ctengine::eval_pipeline(&expr, input, ctx);
        }

        let reusable = ReusableInput::from_pipeline(
            &input,
            "retry: streaming input cannot be retried safely, collect it first",
        )?;
        let mut last_err: Option<CtDiagnosticError> = None;

        for attempt in 1..=attempts {
            if ctx.signal.interrupted() {
                return Err(interrupted_error());
            }
            match ctengine::eval_pipeline(&expr, reusable.to_pipeline_data(), ctx) {
                Ok(out) => return Ok(out),
                Err(err) => {
                    if err.code == ctengine::exit_code::USAGE_ERROR {
                        return Err(CtDiagnosticError::simple(format!(
                            "retry: abort on usage error at attempt {attempt}: {err}"
                        ))
                        .with_code(err.code));
                    }
                    last_err = Some(err);
                    if attempt < attempts {
                        sleep_with_interrupt(Duration::from_nanos(interval_ns), ctx)?;
                    }
                }
            }
        }

        if let Some(err) = last_err {
            return Err(CtDiagnosticError::simple(format!(
                "retry: failed after {attempts} attempts: {err}"
            ))
            .with_code(err.code));
        }

        Err(CtDiagnosticError::simple(
            "retry: unexpected empty retry result",
        ))
    }
}

fn parse_attempts(call: &DataCall) -> Result<u32, CtDiagnosticError> {
    let raw: Option<i64> = call.opt(1).map_err(|e| {
        CtDiagnosticError::simple(format!("retry: {e}")).with_code(ctengine::exit_code::USAGE_ERROR)
    })?;
    let attempts = raw.unwrap_or(3);
    if attempts <= 0 {
        return Err(
            CtDiagnosticError::simple("retry: `attempts` must be a positive integer")
                .with_code(ctengine::exit_code::USAGE_ERROR),
        );
    }
    if attempts > u32::MAX as i64 {
        return Err(CtDiagnosticError::simple(format!(
            "retry: `attempts` is too large (max: {})",
            u32::MAX
        ))
        .with_code(ctengine::exit_code::USAGE_ERROR));
    }
    Ok(attempts as u32)
}

fn parse_interval_ns(call: &DataCall) -> Result<u64, CtDiagnosticError> {
    let raw: Option<CtValue> = call.opt(2).map_err(|e| {
        CtDiagnosticError::simple(format!("retry: {e}")).with_code(ctengine::exit_code::USAGE_ERROR)
    })?;
    let Some(raw) = raw else {
        return Ok(1_000_000_000);
    };
    let ns = match raw {
        CtValue::Duration(v) => v,
        CtValue::Int(v) => v.saturating_mul(1_000_000_000),
        CtValue::Float(v) => (v * 1_000_000_000.0) as i64,
        other => {
            return Err(CtDiagnosticError::simple(format!(
                "retry: `interval` must be Duration/Int/Float, got {:?}",
                other.value_type()
            ))
            .with_code(ctengine::exit_code::USAGE_ERROR));
        }
    };
    if ns <= 0 {
        return Err(CtDiagnosticError::simple("retry: `interval` must be > 0")
            .with_code(ctengine::exit_code::USAGE_ERROR));
    }
    Ok(ns as u64)
}

fn sleep_with_interrupt(total: Duration, ctx: &DataEngineContext) -> Result<(), CtDiagnosticError> {
    let mut waited = Duration::ZERO;
    while waited < total {
        if ctx.signal.interrupted() {
            return Err(interrupted_error());
        }
        let remain = total.saturating_sub(waited);
        let chunk = remain.min(INTERRUPT_POLL_INTERVAL);
        std::thread::sleep(chunk);
        waited = waited.saturating_add(chunk);
    }
    if ctx.signal.interrupted() {
        return Err(interrupted_error());
    }
    Ok(())
}

fn interrupted_error() -> CtDiagnosticError {
    CtDiagnosticError::simple("retry: interrupted by user")
        .with_code(ctengine::exit_code::INTERRUPTED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};
    use ctpipeline::CtPipelineMetadata;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FLAKY_COUNTER: AtomicUsize = AtomicUsize::new(0);
    static BAD_USAGE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[derive(Default)]
    struct FlakyCommand;

    impl ctengine::command::DataCommand for FlakyCommand {
        fn signature(&self) -> DataSignature {
            DataSignature::new("flaky", "fails first two attempts")
                .input(CtType::Any)
                .output(CtType::Int)
        }

        fn run(
            &self,
            _call: &DataCall,
            _input: CtPipelineData,
            _ctx: &DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            let n = FLAKY_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
            if n < 3 {
                return Err(CtDiagnosticError::simple("transient failure"));
            }
            Ok(CtPipelineData::Value(
                CtValue::Int(42),
                CtPipelineMetadata::builtin("flaky"),
            ))
        }
    }

    #[derive(Default)]
    struct BadUsage;

    impl ctengine::command::DataCommand for BadUsage {
        fn signature(&self) -> DataSignature {
            DataSignature::new("bad_usage", "always usage error")
                .input(CtType::Any)
                .output(CtType::Nothing)
        }

        fn run(
            &self,
            _call: &DataCall,
            _input: CtPipelineData,
            _ctx: &DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            BAD_USAGE_COUNTER.fetch_add(1, Ordering::SeqCst);
            Err(CtDiagnosticError::simple("invalid args")
                .with_code(ctengine::exit_code::USAGE_ERROR))
        }
    }

    fn ctx() -> DataEngineContext {
        let factories: Vec<(&'static str, ctengine::DataCommandFactory)> = vec![
            ("flaky", || Box::new(FlakyCommand)),
            ("bad_usage", || Box::new(BadUsage)),
        ];
        let registry = CommandRegistry::from_factories(&factories);
        DataEngineContext::new(registry, None, None)
    }

    fn call(args: Vec<CtValue>) -> DataCall {
        let mut c = DataCall::named("retry");
        for arg in args {
            c.positionals.push(ctsig::BoundArg::new(arg, None));
        }
        c
    }

    #[test]
    fn retry_eventually_succeeds() {
        FLAKY_COUNTER.store(0, Ordering::SeqCst);
        let out = CmdRetry
            .run(
                &call(vec![
                    CtValue::String("flaky".to_string()),
                    CtValue::Int(5),
                    CtValue::Duration(1_000_000),
                ]),
                CtPipelineData::Empty,
                &ctx(),
            )
            .expect("retry should succeed");
        assert!(matches!(out, CtPipelineData::Value(CtValue::Int(42), _)));
        assert_eq!(FLAKY_COUNTER.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn retry_abort_on_usage_error() {
        BAD_USAGE_COUNTER.store(0, Ordering::SeqCst);
        let err = CmdRetry
            .run(
                &call(vec![
                    CtValue::String("bad_usage".to_string()),
                    CtValue::Int(5),
                ]),
                CtPipelineData::Empty,
                &ctx(),
            )
            .expect_err("usage error should abort retries");
        assert_eq!(err.code, ctengine::exit_code::USAGE_ERROR);
        assert_eq!(BAD_USAGE_COUNTER.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn retry_rejects_out_of_range_attempts() {
        let err = CmdRetry
            .run(
                &call(vec![
                    CtValue::String("flaky".to_string()),
                    CtValue::Int((u32::MAX as i64) + 1),
                ]),
                CtPipelineData::Empty,
                &ctx(),
            )
            .expect_err("out-of-range attempts should fail");
        assert!(err.to_string().contains("too large"));
        assert_eq!(err.code, ctengine::exit_code::USAGE_ERROR);
    }

    #[test]
    fn retry_rejects_zero_attempts() {
        let err = CmdRetry
            .run(
                &call(vec![CtValue::String("flaky".to_string()), CtValue::Int(0)]),
                CtPipelineData::Empty,
                &ctx(),
            )
            .expect_err("zero attempts should fail with usage error");
        assert_eq!(err.code, ctengine::exit_code::USAGE_ERROR);
        assert!(err.to_string().contains("positive integer"));
    }

    #[test]
    fn retry_invalid_expression_returns_usage_error() {
        let err = CmdRetry
            .run(
                &call(vec![CtValue::String("| bad".to_string())]),
                CtPipelineData::Empty,
                &ctx(),
            )
            .expect_err("invalid expression should fail");
        assert_eq!(err.code, ctengine::exit_code::USAGE_ERROR);
    }

    #[test]
    fn retry_sleep_observes_interrupt_signal() {
        let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None)
            .with_signal(ctengine::context::SignalHandle::noop());
        ctx.signal.trigger();
        let err =
            sleep_with_interrupt(Duration::from_millis(10), &ctx).expect_err("must interrupt");
        assert_eq!(err.code, ctengine::exit_code::INTERRUPTED);
        assert!(err.to_string().contains("interrupted by user"));
    }
}
