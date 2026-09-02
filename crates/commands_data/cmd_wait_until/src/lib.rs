/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_wait_until` — 轮询条件表达式直到成立或超时。

use ctengine::ReusableInput;
use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctengine::execution::{CommandCore, CommandRunner};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct CmdWaitUntil;

struct WaitUntilCore;
const INTERRUPT_POLL_INTERVAL: Duration = Duration::from_millis(50);

const WAIT_UNTIL_HELP: &str = r#"syskits data wait-until

This is the syskits structured data pipeline wait-until command.
It repeatedly evaluates an expression until it becomes truthy or times out.

Usage:
  wait-until <expr> [timeout] [interval]
  wait-until --help
  wait-until --version

Defaults:
  timeout: 60s
  interval: 1s

Examples:
  wait-until 'http https://example.com | get ok' 30s 1s
  wait-until 'assert true' 5s 500ms
"#;

impl DataCommand for CmdWaitUntil {
    fn signature(&self) -> DataSignature {
        DataSignature::new("wait-until", "poll expression until it becomes true")
            .positional(CtPositionalArg::required(
                "expr",
                "condition expression to evaluate repeatedly",
                CtType::String,
            ))
            .positional(CtPositionalArg::optional(
                "timeout",
                "max wait duration (e.g. 30s, 2min); default: 60s",
                CtType::Duration,
            ))
            .positional(CtPositionalArg::optional(
                "interval",
                "poll interval duration (e.g. 1s); default: 1s",
                CtType::Duration,
            ))
            .flag(CtFlag::switch(
                "help",
                Some('h'),
                "show help for syskits data wait-until",
            ))
            .flag(CtFlag::switch(
                "version",
                None,
                "show syskits data wait-until version",
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
            return Ok(meta_text_output(WAIT_UNTIL_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data wait-until {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        CommandRunner::run(&WaitUntilCore, call, input, ctx)
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
            source: Some("wait-until".into()),
            ..Default::default()
        },
    )
}

impl CommandCore for WaitUntilCore {
    fn run_core(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let expr_text: String = call
            .req(0)
            .map_err(|e| CtDiagnosticError::simple(format!("wait-until: {e}")))?;
        let timeout_ns = parse_duration_arg(call, 1, 60_000_000_000, "timeout")?;
        let interval_ns = parse_duration_arg(call, 2, 1_000_000_000, "interval")?;

        let expr = ctdsl::parse(&expr_text).map_err(|e| {
            CtDiagnosticError::simple(format!("wait-until: failed to parse expression: {e}"))
                .with_code(ctengine::exit_code::USAGE_ERROR)
        })?;
        let reusable_input = ReusableInput::from_pipeline(
            &input,
            "wait-until: streaming input is not supported, collect it before waiting",
        )?;

        let start = Instant::now();
        let timeout = Duration::from_nanos(timeout_ns);
        let interval = Duration::from_nanos(interval_ns);
        let mut attempts = 0_u64;

        loop {
            if ctx.signal.interrupted() {
                return Err(interrupted_error());
            }
            attempts = attempts.saturating_add(1);
            let out = ctengine::eval_pipeline(&expr, reusable_input.to_pipeline_data(), ctx)
                .map_err(|e| {
                    CtDiagnosticError::simple(format!(
                        "wait-until: condition evaluation failed on attempt {attempts}: {e}"
                    ))
                    .with_code(e.code)
                })?;

            if pipeline_truthy(out)? {
                return if input.is_empty() {
                    Ok(CtPipelineData::Value(
                        CtValue::Bool(true),
                        CtPipelineMetadata::builtin("wait-until"),
                    ))
                } else {
                    Ok(input)
                };
            }

            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return Err(CtDiagnosticError::simple(format!(
                    "wait-until: timed out after {}ms (attempts={attempts})",
                    elapsed.as_millis()
                ))
                .with_code(ctengine::exit_code::TIMEOUT));
            }

            let remain = timeout.saturating_sub(elapsed);
            sleep_with_interrupt(interval.min(remain), ctx)?;
        }
    }
}

fn parse_duration_arg(
    call: &DataCall,
    pos: usize,
    default_ns: u64,
    name: &str,
) -> Result<u64, CtDiagnosticError> {
    let value: Option<CtValue> = call.opt(pos).map_err(|e| {
        CtDiagnosticError::simple(format!("wait-until: {e}"))
            .with_code(ctengine::exit_code::USAGE_ERROR)
    })?;
    let Some(value) = value else {
        return Ok(default_ns);
    };

    let ns = match value {
        CtValue::Duration(v) => v,
        CtValue::Int(v) => v.saturating_mul(1_000_000_000),
        CtValue::Float(v) => (v * 1_000_000_000.0) as i64,
        other => {
            return Err(CtDiagnosticError::simple(format!(
                "wait-until: `{name}` must be Duration/Int/Float, got {:?}",
                other.value_type()
            ))
            .with_code(ctengine::exit_code::USAGE_ERROR));
        }
    };
    if ns <= 0 {
        return Err(
            CtDiagnosticError::simple(format!("wait-until: `{name}` must be > 0"))
                .with_code(ctengine::exit_code::USAGE_ERROR),
        );
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
    CtDiagnosticError::simple("wait-until: interrupted by user")
        .with_code(ctengine::exit_code::INTERRUPTED)
}

fn pipeline_truthy(data: CtPipelineData) -> Result<bool, CtDiagnosticError> {
    match data.collect_values() {
        CtPipelineData::Empty => Ok(false),
        CtPipelineData::Value(v, _) => value_truthy(&v),
        CtPipelineData::ListStream(_) => unreachable!("collect_values should materialize list"),
        CtPipelineData::ByteStream(_) => Err(CtDiagnosticError::simple(
            "wait-until: condition expression returned binary stream, expected value",
        )),
    }
}

fn value_truthy(value: &CtValue) -> Result<bool, CtDiagnosticError> {
    let ret = match value {
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
        CtValue::Error(e) => {
            return Err(CtDiagnosticError::simple(format!(
                "wait-until: condition produced error value: {e}"
            )));
        }
    };
    Ok(ret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct ProbeSuccessAfter3;

    static PROBE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    impl ctengine::command::DataCommand for ProbeSuccessAfter3 {
        fn signature(&self) -> DataSignature {
            DataSignature::new("probe3", "test probe")
                .input(CtType::Any)
                .output(CtType::Bool)
        }

        fn run(
            &self,
            _call: &DataCall,
            _input: CtPipelineData,
            _ctx: &DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            let n = PROBE_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(CtPipelineData::Value(
                CtValue::Bool(n >= 3),
                CtPipelineMetadata::builtin("probe3"),
            ))
        }
    }

    #[derive(Default)]
    struct ProbeAlwaysFalse;

    impl ctengine::command::DataCommand for ProbeAlwaysFalse {
        fn signature(&self) -> DataSignature {
            DataSignature::new("probe_false", "always false")
                .input(CtType::Any)
                .output(CtType::Bool)
        }

        fn run(
            &self,
            _call: &DataCall,
            _input: CtPipelineData,
            _ctx: &DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            Ok(CtPipelineData::Value(
                CtValue::Bool(false),
                CtPipelineMetadata::builtin("probe_false"),
            ))
        }
    }

    #[derive(Default)]
    struct ProbeTimeout;

    impl ctengine::command::DataCommand for ProbeTimeout {
        fn signature(&self) -> DataSignature {
            DataSignature::new("probe_timeout", "always timeout")
                .input(CtType::Any)
                .output(CtType::Nothing)
        }

        fn run(
            &self,
            _call: &DataCall,
            _input: CtPipelineData,
            _ctx: &DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            Err(CtDiagnosticError::simple("probe timeout").with_code(ctengine::exit_code::TIMEOUT))
        }
    }

    fn ctx_with_probes() -> DataEngineContext {
        let factories: Vec<(&'static str, ctengine::DataCommandFactory)> = vec![
            ("probe3", || Box::new(ProbeSuccessAfter3)),
            ("probe_false", || Box::new(ProbeAlwaysFalse)),
            ("probe_timeout", || Box::new(ProbeTimeout)),
        ];
        let registry = CommandRegistry::from_factories(&factories);
        DataEngineContext::new(registry, None, None)
    }

    fn call(args: Vec<CtValue>) -> DataCall {
        let mut c = DataCall::named("wait-until");
        for v in args {
            c.positionals.push(ctsig::BoundArg::new(v, None));
        }
        c
    }

    #[test]
    fn wait_until_success_after_retries() {
        PROBE_COUNTER.store(0, Ordering::SeqCst);
        let cmd = CmdWaitUntil;
        let out = cmd
            .run(
                &call(vec![
                    CtValue::String("probe3".to_string()),
                    CtValue::Duration(2_000_000_000),
                    CtValue::Duration(10_000_000),
                ]),
                CtPipelineData::Empty,
                &ctx_with_probes(),
            )
            .expect("should eventually pass");
        assert!(matches!(out, CtPipelineData::Value(CtValue::Bool(true), _)));
        assert!(PROBE_COUNTER.load(Ordering::SeqCst) >= 3);
    }

    #[test]
    fn wait_until_timeout_returns_124() {
        let cmd = CmdWaitUntil;
        let err = cmd
            .run(
                &call(vec![
                    CtValue::String("probe_false".to_string()),
                    CtValue::Duration(50_000_000),
                    CtValue::Duration(10_000_000),
                ]),
                CtPipelineData::Empty,
                &ctx_with_probes(),
            )
            .expect_err("should timeout");
        assert_eq!(err.code, ctengine::exit_code::TIMEOUT);
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn wait_until_preserves_nested_error_code() {
        let cmd = CmdWaitUntil;
        let err = cmd
            .run(
                &call(vec![CtValue::String("probe_timeout".to_string())]),
                CtPipelineData::Empty,
                &ctx_with_probes(),
            )
            .expect_err("should propagate nested timeout");
        assert_eq!(err.code, ctengine::exit_code::TIMEOUT);
        assert!(err.to_string().contains("condition evaluation failed"));
    }

    #[test]
    fn wait_until_invalid_duration_arg_returns_usage_error() {
        let cmd = CmdWaitUntil;
        // timeout = 0 is invalid (must be > 0)
        let err = cmd
            .run(
                &call(vec![
                    CtValue::String("probe3".to_string()),
                    CtValue::Duration(0), // zero timeout: invalid
                ]),
                CtPipelineData::Empty,
                &ctx_with_probes(),
            )
            .expect_err("zero timeout should fail with usage error");
        assert_eq!(err.code, ctengine::exit_code::USAGE_ERROR);
        assert!(err.to_string().contains("must be > 0"));
    }

    #[test]
    fn wait_until_sleep_observes_interrupt_signal() {
        let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None)
            .with_signal(ctengine::context::SignalHandle::noop());
        ctx.signal.trigger();
        let err =
            sleep_with_interrupt(Duration::from_millis(10), &ctx).expect_err("must interrupt");
        assert_eq!(err.code, ctengine::exit_code::INTERRUPTED);
        assert!(err.to_string().contains("interrupted by user"));
    }
}
