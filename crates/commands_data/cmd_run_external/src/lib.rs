/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

use ctengine::{
    CtDiagnosticError, DataCommand, DataEngineContext,
    external::{
        ExternalCallSpec, ExternalExecutor, ExternalExitPolicy, ExternalPathPolicy,
        ExternalStderrMode, ExternalStdinMode,
    },
};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};

#[derive(Default)]
pub struct CmdRunExternal;

const RUN_EXTERNAL_HELP: &str = r#"syskits data run-external

This is the syskits structured data pipeline run-external command.
It explicitly runs an external command from PATH while skipping syskits priority shims.

Usage:
  run-external <cmd> [args...]
  run-external --help
  run-external --version

Examples:
  run-external whoami
  run-external echo --help
"#;

impl DataCommand for CmdRunExternal {
    fn signature(&self) -> DataSignature {
        DataSignature::new(
            "run-external",
            "explicitly span and run an external command",
        )
        .positional(CtPositionalArg::required(
            "cmd",
            "The external command to run",
            CtType::String,
        ))
        .rest(CtPositionalArg::optional(
            "args",
            "Additional arguments for the command",
            CtType::String,
        ))
        .flag(CtFlag::with_value(
            "stderr-mode",
            None,
            "Mode for capturing stderr (inherit, merge, capture)",
            CtType::String,
        ))
        .flag(CtFlag::with_value(
            "stdin-mode",
            None,
            "Mode for sending stdin (raw, text, json, jsonlines)",
            CtType::String,
        ))
        .flag(CtFlag::with_value(
            "exit-policy",
            None,
            "Policy for non-zero exit code (fail, allow)",
            CtType::String,
        ))
        .flag(CtFlag::with_value(
            "timeout-ms",
            None,
            "Timeout for execution in milliseconds",
            CtType::Int,
        ))
        .flag(CtFlag::switch(
            "help",
            Some('h'),
            "show help for syskits data run-external",
        ))
        .flag(CtFlag::switch(
            "version",
            None,
            "show syskits data run-external version",
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
            return Ok(meta_text_output(RUN_EXTERNAL_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data run-external {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        let spec = build_spec(call)?;
        ExternalExecutor::run(spec, input, ctx)
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
            source: Some("run-external".into()),
            ..Default::default()
        },
    )
}

fn build_spec(call: &DataCall) -> Result<ExternalCallSpec, CtDiagnosticError> {
    let cmd_name = call
        .req::<String>(0)
        .map_err(|e| CtDiagnosticError::simple(e.to_string()))?;

    let mut ext_args = Vec::new();
    if let Ok(rest) = call.rest::<CtValue>(1) {
        for v in rest {
            ext_args.push(v.to_text());
        }
    }

    let mut spec = ExternalCallSpec::quick(&cmd_name, &ext_args);
    spec.path_policy = ExternalPathPolicy::SkipSyskitsPriority;

    if let Some(s) = call
        .get_flag::<String>("stderr-mode")
        .map_err(|e| CtDiagnosticError::simple(e.to_string()))?
    {
        spec.stderr_mode = parse_stderr_mode(&s)?;
    }

    if let Some(s) = call
        .get_flag::<String>("stdin-mode")
        .map_err(|e| CtDiagnosticError::simple(e.to_string()))?
    {
        spec.stdin_mode = parse_stdin_mode(&s)?;
    }

    if let Some(s) = call
        .get_flag::<String>("exit-policy")
        .map_err(|e| CtDiagnosticError::simple(e.to_string()))?
    {
        spec.exit_policy = parse_exit_policy(&s)?;
    }

    if let Some(ms) = call
        .get_flag::<i64>("timeout-ms")
        .map_err(|e| CtDiagnosticError::simple(e.to_string()))?
        && ms > 0
    {
        spec.timeout_ms = Some(ms as u64);
    }

    Ok(spec)
}

fn parse_stderr_mode(s: &str) -> Result<ExternalStderrMode, CtDiagnosticError> {
    match s.to_lowercase().as_str() {
        "inherit" => Ok(ExternalStderrMode::Inherit),
        "merge" => Ok(ExternalStderrMode::MergeToStdout),
        "capture" => Ok(ExternalStderrMode::Capture),
        _ => Err(CtDiagnosticError::simple(format!(
            "unknown stderr-mode '{s}'"
        ))),
    }
}

fn parse_stdin_mode(s: &str) -> Result<ExternalStdinMode, CtDiagnosticError> {
    match s.to_lowercase().as_str() {
        "raw" => Ok(ExternalStdinMode::Raw),
        "text" => Ok(ExternalStdinMode::TextLines),
        "json" => Ok(ExternalStdinMode::Json),
        "jsonlines" => Ok(ExternalStdinMode::JsonLines),
        _ => Err(CtDiagnosticError::simple(format!(
            "unknown stdin-mode '{s}'"
        ))),
    }
}

fn parse_exit_policy(s: &str) -> Result<ExternalExitPolicy, CtDiagnosticError> {
    match s.to_lowercase().as_str() {
        "fail" => Ok(ExternalExitPolicy::FailOnNonZero),
        "allow" => Ok(ExternalExitPolicy::AllowNonZero),
        _ => Err(CtDiagnosticError::simple(format!(
            "unknown exit-policy '{s}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctsig::BoundArg;

    fn call_with_flags(flags: Vec<(&str, CtValue)>) -> DataCall {
        let mut call = DataCall::named("run-external");
        call.positionals
            .push(BoundArg::new(CtValue::String("echo".into()), None));
        for (name, val) in flags {
            call.flags
                .insert(name.to_string(), Some(BoundArg::new(val, None)));
        }
        call
    }

    fn meta_call(name: &str) -> DataCall {
        let mut call = DataCall::named("run-external");
        call.flags.insert(name.to_string(), None);
        call
    }

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(ctengine::CommandRegistry::empty(), None, None)
    }

    #[test]
    fn parses_known_modes_and_timeout() {
        let call = call_with_flags(vec![
            ("stderr-mode", CtValue::String("capture".into())),
            ("stdin-mode", CtValue::String("text".into())),
            ("exit-policy", CtValue::String("allow".into())),
            ("timeout-ms", CtValue::Int(1500)),
        ]);
        let spec = build_spec(&call).unwrap();
        assert_eq!(spec.stderr_mode, ExternalStderrMode::Capture);
        assert_eq!(spec.stdin_mode, ExternalStdinMode::TextLines);
        assert_eq!(spec.exit_policy, ExternalExitPolicy::AllowNonZero);
        assert_eq!(spec.timeout_ms, Some(1500));
    }

    #[test]
    fn timeout_type_error_is_propagated() {
        let call = call_with_flags(vec![("timeout-ms", CtValue::String("bad".into()))]);
        let err = build_spec(&call).expect_err("expected type conversion error");
        assert!(err.to_string().contains("flag '--timeout-ms'"));
    }

    #[test]
    fn defaults_apply_when_flags_are_missing() {
        let call = call_with_flags(Vec::new());
        let spec = build_spec(&call).unwrap();
        assert_eq!(
            spec.stdout_mode,
            ctengine::external::ExternalStdoutMode::Raw
        );
        assert_eq!(spec.stderr_mode, ExternalStderrMode::Inherit);
        assert_eq!(spec.stdin_mode, ExternalStdinMode::Raw);
        assert_eq!(spec.exit_policy, ExternalExitPolicy::FailOnNonZero);
        assert_eq!(spec.path_policy, ExternalPathPolicy::SkipSyskitsPriority);
        assert_eq!(spec.timeout_ms, None);
    }

    #[test]
    fn preserves_dash_prefixed_external_arguments() {
        let mut call = DataCall::named("run-external");
        call.positionals
            .push(BoundArg::new(CtValue::String("echo".into()), None));
        call.positionals
            .push(BoundArg::new(CtValue::String("-n".into()), None));
        call.positionals
            .push(BoundArg::new(CtValue::String("hi".into()), None));

        let spec = build_spec(&call).unwrap();
        assert_eq!(spec.cmd, "echo");
        assert_eq!(spec.args, vec!["-n".to_string(), "hi".to_string()]);
    }

    #[test]
    fn run_external_help_output_does_not_require_command() {
        let out = CmdRunExternal
            .run(&meta_call("help"), CtPipelineData::Empty, &ctx())
            .expect("help should not require an external command");
        let CtPipelineData::Value(CtValue::String(text), meta) = out else {
            panic!("expected help text");
        };
        assert!(text.contains("syskits structured data pipeline run-external command"));
        assert!(text.contains("run-external echo --help"));
        assert_eq!(meta.exit_code, 0);
    }

    #[test]
    fn run_external_version_output() {
        let out = CmdRunExternal
            .run(&meta_call("version"), CtPipelineData::Empty, &ctx())
            .expect("version should not require an external command");
        let CtPipelineData::Value(CtValue::String(text), meta) = out else {
            panic!("expected version text");
        };
        assert!(text.starts_with("syskits data run-external "));
        assert_eq!(meta.exit_code, 0);
    }
}
