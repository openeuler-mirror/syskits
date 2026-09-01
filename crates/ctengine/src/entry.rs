/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! `run_data_entry` — `syskits data` 入口函数（M1b Real Interpreter）。
//!
//! M1b：接收 `data <expr>` 参数，通过 ctdsl 解析，再由 eval_pipeline 执行。
//!
//! 用法示例：
//! ```text
//! syskits data "from /etc/os-release | get NAME"
//! ```

use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtValue};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::context::{CommandRegistry, DataEngineContext};
use crate::error::CtDiagnosticError;
use crate::execution::{ExitPolicy, OutputFormat};
use crate::interpreter::{eval_pipeline, try_print_pipeline_data_with_profile_and_signal};
use crate::legacy_adapter::LegacyToolResolver;
use crate::{OutputProfile, exit_code};

/// `syskits data` 命令的顶层入口（M1b）
///
/// - `args`：`data` 之后的参数（不含 `data` 本身）
/// - `registry`：已注册 DataCommand 的注册表
///
/// 返回进程退出码（0 = 成功，1 = 解析/运行错误，130 = 中断）
pub fn run_data_entry_with_registry(args: &[OsString], registry: CommandRegistry) -> i32 {
    run_data_entry_with_registry_and_legacy(args, registry, None, None)
}

/// `syskits data` 顶层入口（可选 legacy fallback）
pub fn run_data_entry_with_registry_and_legacy(
    args: &[OsString],
    registry: CommandRegistry,
    legacy_resolver: Option<LegacyToolResolver>,
    plugin_registry: Option<std::sync::Arc<dyn crate::context::PluginProvider>>,
) -> i32 {
    let started = Instant::now();
    let request = match resolve_data_request(args, std::io::stdout().is_terminal()) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("syskits data: {message}");
            return ExitPolicy::usage_error();
        }
    };

    // 将参数转换为 String 列表（保留原始 OsString 用于文件路径）
    let args_str: Vec<String> = request
        .pipeline_argv
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    #[allow(unused_variables)]
    let (is_workflow, workflow_path) =
        if !args_str.is_empty() && (args_str[0] == "-f" || args_str[0] == "--file") {
            if args_str.len() >= 2 {
                (true, args_str[1].clone())
            } else {
                eprintln!("syskits data: {} requires a path argument", args_str[0]);
                return ExitPolicy::usage_error();
            }
        } else {
            (false, String::new())
        };

    let profile = request.profile;
    // 3. 构建执行上下文
    let ctx = DataEngineContext::new(registry, legacy_resolver, plugin_registry)
        .with_signal(crate::context::SignalHandle::register_sigint())
        .with_output_format(profile.format);

    #[cfg(feature = "workflow")]
    if is_workflow {
        match std::fs::read_to_string(&workflow_path) {
            Ok(src) => match crate::workflow_parser::parse_yaml_workflow(&src) {
                Ok(script) => {
                    match crate::workflow::run_workflow(&script, CtPipelineData::Empty, &ctx) {
                        Ok(data) => {
                            let mut data = data;
                            inject_engine_success_metadata(
                                &mut data,
                                started.elapsed().as_millis() as u64,
                            );
                            let output_exit_code = PipelineExitTracker::from_data(&data);
                            match try_print_pipeline_data_with_profile_and_signal(
                                data,
                                &profile,
                                Some(&ctx.signal),
                            ) {
                                Ok(()) => return output_exit_code.exit_code(),
                                Err(e) => {
                                    eprintln!("syskits data: error: {e}");
                                    return ExitPolicy::from_diagnostic(&e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("syskits data: error: {e}");
                            return workflow_error_exit_code(&e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("syskits data: workflow parse error in {workflow_path}: {e}");
                    return exit_code::RUNTIME_ERROR;
                }
            },
            Err(e) => {
                eprintln!("syskits data: read error for {workflow_path}: {e}");
                return exit_code::RUNTIME_ERROR;
            }
        }
    }

    #[cfg(not(feature = "workflow"))]
    if is_workflow {
        eprintln!("syskits data: workflow feature is not enabled");
        return exit_code::RUNTIME_ERROR;
    }

    let input_str = argv_to_dsl_expr(&args_str);

    if input_str.trim().is_empty() {
        eprintln!("syskits data: no pipeline expression given");
        eprintln!("  usage: syskits data \"<expr>\"");
        eprintln!("         syskits data -f <workflow.skd>");
        eprintln!("  example: syskits data \"from /etc/os-release | get NAME\"");
        return ExitPolicy::usage_error();
    }

    match parse_and_eval_expr(&input_str, &ctx) {
        Ok(data) => {
            ctx.emit_trace_if_enabled();
            let mut data = data;
            inject_engine_success_metadata(&mut data, started.elapsed().as_millis() as u64);
            let output_exit_code = PipelineExitTracker::from_data(&data);
            match try_print_pipeline_data_with_profile_and_signal(data, &profile, Some(&ctx.signal))
            {
                Ok(()) => output_exit_code.exit_code(),
                Err(e) => {
                    eprintln!("syskits data: error: {e}");
                    ExitPolicy::from_diagnostic(&e)
                }
            }
        }
        Err(e) => {
            let code = ExitPolicy::from_diagnostic(&e);
            ctx.emit_trace_if_enabled();
            eprintln!("syskits data: error: {e}");
            code
        }
    }
}

struct PipelineExitTracker {
    fallback: i32,
    custom: Option<Arc<Mutex<BTreeMap<String, CtValue>>>>,
}

impl PipelineExitTracker {
    fn from_data(data: &CtPipelineData) -> Self {
        match data {
            CtPipelineData::Empty => Self {
                fallback: exit_code::SUCCESS,
                custom: None,
            },
            CtPipelineData::Value(_, meta) => Self::from_metadata(meta),
            CtPipelineData::ListStream(stream) => Self::from_metadata(&stream.metadata),
            CtPipelineData::ByteStream(stream) => Self::from_metadata(&stream.metadata),
        }
    }

    fn from_metadata(meta: &CtPipelineMetadata) -> Self {
        Self {
            fallback: meta.exit_code,
            custom: Some(meta.custom.clone()),
        }
    }

    fn exit_code(&self) -> i32 {
        self.custom
            .as_ref()
            .and_then(|custom| custom.lock().ok())
            .and_then(|guard| match guard.get("external.exit_code") {
                Some(CtValue::Int(code)) => i32::try_from(*code).ok(),
                _ => None,
            })
            .unwrap_or(self.fallback)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedDataRequest<'a> {
    pub profile: OutputProfile,
    pub pipeline_argv: &'a [OsString],
}

pub fn resolve_data_request(
    args: &[OsString],
    stdout_is_tty: bool,
) -> Result<ResolvedDataRequest<'_>, String> {
    let (format_override, pipeline_argv) = parse_data_cli_options(args)?;
    let profile = resolve_data_output_profile_with_cli(stdout_is_tty, format_override);

    Ok(ResolvedDataRequest {
        profile,
        pipeline_argv,
    })
}

fn parse_data_cli_options(
    args: &[OsString],
) -> Result<(Option<OutputFormat>, &[OsString]), String> {
    let mut format_override = None;
    let mut index = 0;

    while index < args.len() {
        let Some(arg) = args[index].to_str() else {
            break;
        };

        if let Some(value) = arg.strip_prefix("format=") {
            format_override = Some(parse_output_format(value)?);
            index += 1;
            continue;
        }

        if let Some(value) = arg.strip_prefix("--format=") {
            format_override = Some(parse_output_format(value)?);
            index += 1;
            continue;
        }

        if arg == "--format" {
            let Some(value) = args.get(index + 1).and_then(|value| value.to_str()) else {
                return Err("missing value after `--format`".into());
            };
            format_override = Some(parse_output_format(value)?);
            index += 2;
            continue;
        }

        break;
    }

    Ok((format_override, &args[index..]))
}

fn parse_output_format(raw: &str) -> Result<OutputFormat, String> {
    OutputFormat::parse(raw).ok_or_else(|| format!("unsupported format `{raw}`"))
}

#[cfg(feature = "workflow")]
fn workflow_error_exit_code(err: &crate::workflow::WorkflowError) -> i32 {
    match err {
        crate::workflow::WorkflowError::ParseStage { err, .. }
        | crate::workflow::WorkflowError::RunStage { err, .. } => ExitPolicy::from_diagnostic(err),
        crate::workflow::WorkflowError::EmptyScript => exit_code::RUNTIME_ERROR,
    }
}

#[cfg(test)]
fn resolve_data_output_profile(stdout_is_tty: bool) -> OutputProfile {
    resolve_data_output_profile_with_cli(stdout_is_tty, None)
}

fn resolve_data_output_profile_with_cli(
    stdout_is_tty: bool,
    cli_override: Option<OutputFormat>,
) -> OutputProfile {
    let base = OutputProfile::for_data_cli(stdout_is_tty);
    let format = std::env::var("SYSKITS_DATA_FORMAT").ok();
    let pager = std::env::var("SYSKITS_DATA_PAGER").ok();
    let mut profile = apply_profile_overrides(base, format.as_deref(), pager.as_deref());

    if let Some(format) = cli_override {
        profile.format = format;
    }

    profile
}

fn apply_profile_overrides(
    mut profile: OutputProfile,
    format: Option<&str>,
    pager: Option<&str>,
) -> OutputProfile {
    if let Some(raw) = format
        && let Some(parsed) = OutputFormat::parse(raw)
    {
        profile.format = parsed;
    }
    if let Some(raw) = pager {
        profile.use_pager = matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
    }
    profile
}

fn inject_engine_success_metadata(data: &mut CtPipelineData, duration_ms: u64) {
    let write_meta = |meta: &mut ctpipeline::CtPipelineMetadata| {
        if let Ok(mut custom) = meta.custom.lock() {
            custom.insert(
                "engine.exit_code".to_string(),
                CtValue::Int(exit_code::SUCCESS as i64),
            );
            custom.insert(
                "engine.duration_ms".to_string(),
                CtValue::Int(duration_ms.min(i64::MAX as u64) as i64),
            );
        }
    };

    match data {
        CtPipelineData::Empty => {}
        CtPipelineData::Value(_, meta) => write_meta(meta),
        CtPipelineData::ListStream(stream) => write_meta(&mut stream.metadata),
        CtPipelineData::ByteStream(stream) => write_meta(&mut stream.metadata),
    }
}

/// 对已解析表达式执行统一求值（CLI/REPL 共用）
pub fn eval_expr(
    expr: &ctdsl::Expr,
    ctx: &DataEngineContext,
) -> Result<CtPipelineData, CtDiagnosticError> {
    eval_pipeline(expr, CtPipelineData::Empty, ctx)
}

/// 统一的 parse + eval 入口（CLI/REPL 可复用）
pub fn parse_and_eval_expr(
    input: &str,
    ctx: &DataEngineContext,
) -> Result<CtPipelineData, CtDiagnosticError> {
    let expr = ctdsl::parse(input).map_err(parse_error_to_diagnostic)?;
    eval_expr(&expr, ctx)
}

/// DSL 管线操作符字符集：这些字符出现在 token 中时必须引号保护。
fn needs_dsl_quoting(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t' | '\n' | '|' | '"' | '\'' | '\\' | ';' | '(' | ')' | '{' | '}'
        )
    }) || s.is_empty()
}

/// 对单个 argv token 生成 DSL 安全的双引号字符串字面量。
/// 转义规则与 ctdsl lexer lex_string 对应：\ " \n \t。
fn dsl_quote_token(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 将 argv token 列表重建为 DSL 表达式字符串，对需要引号保护的 token 做转义。
///
/// 设计约束：argv 中的第一个 token 通常是命令名（不含特殊字符），其余 token
/// 如果包含空格/管道等字符则必须以双引号字面量形式传递，否则 DSL lexer 会
/// 将它们解析为多个独立 token 并破坏参数边界。
fn argv_to_dsl_expr(args: &[String]) -> String {
    if args.len() == 1 {
        return args[0].clone();
    }

    args.iter()
        .map(|a| {
            if needs_dsl_quoting(a) {
                dsl_quote_token(a)
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_error_to_diagnostic(err: ctdsl::ParseError) -> CtDiagnosticError {
    match err {
        ctdsl::ParseError::LexError { message, span }
        | ctdsl::ParseError::SyntaxError { message, span } => {
            CtDiagnosticError::with_span(message, span).with_code(exit_code::USAGE_ERROR)
        }
        ctdsl::ParseError::UnexpectedEof => {
            CtDiagnosticError::simple("unexpected end of input").with_code(exit_code::USAGE_ERROR)
        }
    }
}

/// `syskits data` 入口（使用空注册表，M1b 过渡期）
///
/// M1c 阶段将传入具体命令注册表来替换此函数。
pub fn run_data_entry(args: &[OsString]) -> i32 {
    run_data_entry_with_registry(args, CommandRegistry::empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::DataEngineContext;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_data_output_env<T>(
        format: Option<&str>,
        pager: Option<&str>,
        f: impl FnOnce() -> T,
    ) -> T {
        let _guard = env_lock().lock().unwrap();
        let saved_format = std::env::var("SYSKITS_DATA_FORMAT").ok();
        let saved_pager = std::env::var("SYSKITS_DATA_PAGER").ok();

        unsafe {
            match format {
                Some(value) => std::env::set_var("SYSKITS_DATA_FORMAT", value),
                None => std::env::remove_var("SYSKITS_DATA_FORMAT"),
            }
            match pager {
                Some(value) => std::env::set_var("SYSKITS_DATA_PAGER", value),
                None => std::env::remove_var("SYSKITS_DATA_PAGER"),
            }
        }

        let result = f();

        unsafe {
            match saved_format {
                Some(value) => std::env::set_var("SYSKITS_DATA_FORMAT", value),
                None => std::env::remove_var("SYSKITS_DATA_FORMAT"),
            }
            match saved_pager {
                Some(value) => std::env::set_var("SYSKITS_DATA_PAGER", value),
                None => std::env::remove_var("SYSKITS_DATA_PAGER"),
            }
        }

        result
    }

    #[test]
    fn test_run_data_entry_no_args_returns_usage_error() {
        // 无参数 → 打印 usage，返回 2
        let code = run_data_entry(&[]);
        assert_eq!(code, exit_code::USAGE_ERROR);
    }

    #[test]
    fn test_run_data_entry_unknown_command_returns_1() {
        // 有参数但命令未注册 → parse 成功，执行失败，返回 1
        let args: Vec<OsString> = vec![OsString::from("nonexistent_cmd")];
        let code = run_data_entry(&args);
        assert_eq!(code, 1);
    }

    #[test]
    fn test_parse_and_eval_expr_parse_error() {
        let ctx = DataEngineContext::empty_for_test();
        let err = parse_and_eval_expr("| bad", &ctx).expect_err("must fail");
        assert_eq!(err.code, exit_code::USAGE_ERROR);
    }

    #[test]
    fn test_parse_and_eval_expr_unknown_command_error() {
        let ctx = DataEngineContext::empty_for_test();
        let err = parse_and_eval_expr("unknown_cmd", &ctx).expect_err("must fail");
        assert_eq!(err.code, 1);
    }

    #[test]
    fn resolve_data_output_profile_defaults_to_auto() {
        with_data_output_env(None, None, || {
            let profile = resolve_data_output_profile(true);
            assert_eq!(profile.format, OutputFormat::Auto);
        });
    }

    #[test]
    fn resolve_data_output_profile_accepts_raw_env_override() {
        with_data_output_env(Some("raw"), None, || {
            let profile = resolve_data_output_profile(false);
            assert_eq!(profile.format, OutputFormat::Raw);
        });
    }

    #[test]
    fn resolve_data_request_supports_single_axis_assignment() {
        with_data_output_env(Some("raw"), None, || {
            let args = vec![
                OsString::from("format=json"),
                OsString::from("pwd"),
                OsString::from("--logical"),
            ];

            let parsed = resolve_data_request(&args, true).expect("parsed args");

            assert_eq!(parsed.profile.format, OutputFormat::Json);
            assert_eq!(
                parsed.pipeline_argv,
                &[OsString::from("pwd"), OsString::from("--logical")]
            );
        });
    }

    #[test]
    fn resolve_data_request_supports_long_format_flag() {
        with_data_output_env(None, None, || {
            let args = vec![
                OsString::from("--format"),
                OsString::from("raw"),
                OsString::from("pwd"),
            ];

            let parsed = resolve_data_request(&args, true).expect("parsed args");

            assert_eq!(parsed.profile.format, OutputFormat::Raw);
            assert_eq!(parsed.pipeline_argv, &[OsString::from("pwd")]);
        });
    }

    #[test]
    fn apply_profile_overrides_only_uses_format_and_pager() {
        let base = OutputProfile::for_data_cli(true);
        let profile = apply_profile_overrides(base, Some("json"), Some("on"));
        assert_eq!(profile.format, OutputFormat::Json);
        assert!(profile.use_pager);
    }

    #[test]
    fn test_inject_engine_success_metadata_value() {
        let mut data =
            CtPipelineData::Value(CtValue::Int(1), ctpipeline::CtPipelineMetadata::default());
        inject_engine_success_metadata(&mut data, 123);
        let CtPipelineData::Value(_, meta) = data else {
            panic!("expected value");
        };
        let guard = meta.custom.lock().unwrap();
        assert!(matches!(
            guard.get("engine.exit_code"),
            Some(CtValue::Int(code)) if *code == 0
        ));
        assert!(matches!(
            guard.get("engine.duration_ms"),
            Some(CtValue::Int(ms)) if *ms == 123
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_run_data_entry_external_nonzero_returns_nonzero() {
        let args: Vec<OsString> = vec![
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from("false"),
        ];
        let code = run_data_entry(&args);
        assert_ne!(code, 0);
    }

    #[cfg(feature = "workflow")]
    #[test]
    fn test_run_data_entry_workflow_file_not_found() {
        let args: Vec<OsString> = vec![
            OsString::from("-f"),
            OsString::from("/nonexistent/path/to/workflow.skd"),
        ];
        let code = run_data_entry(&args);
        assert_eq!(code, 1);
    }

    #[cfg(feature = "workflow")]
    #[test]
    fn test_workflow_error_exit_code_preserves_stage_error_code() {
        let err =
            CtDiagnosticError::simple("timed out").with_code(crate::execution::exit_code::TIMEOUT);
        let wf_err = crate::workflow::WorkflowError::RunStage {
            stage: "s1".to_string(),
            err,
        };
        assert_eq!(
            workflow_error_exit_code(&wf_err),
            crate::execution::exit_code::TIMEOUT
        );
    }

    #[test]
    fn argv_to_dsl_expr_plain_tokens_joined_with_space() {
        let args = vec!["cat".to_string(), "/etc/os-release".to_string()];
        assert_eq!(argv_to_dsl_expr(&args), "cat /etc/os-release");
    }

    #[test]
    fn argv_to_dsl_expr_quotes_token_with_space() {
        let args = vec!["cat".to_string(), "/tmp/a b.txt".to_string()];
        assert_eq!(argv_to_dsl_expr(&args), r#"cat "/tmp/a b.txt""#);
    }

    #[test]
    fn argv_to_dsl_expr_single_arg_is_raw_expression() {
        let args = vec!["ls -a".to_string()];
        assert_eq!(argv_to_dsl_expr(&args), "ls -a");
    }

    #[test]
    fn argv_to_dsl_expr_quotes_token_with_pipe() {
        let args = vec!["echo".to_string(), "a|b".to_string()];
        assert_eq!(argv_to_dsl_expr(&args), r#"echo "a|b""#);
    }

    #[test]
    fn argv_to_dsl_expr_escapes_backslash_and_quote() {
        let args = vec!["echo".to_string(), r#"a\"b"#.to_string()];
        assert_eq!(argv_to_dsl_expr(&args), r#"echo "a\\\"b""#);
    }

    #[test]
    fn argv_to_dsl_expr_quotes_empty_token() {
        let args = vec!["echo".to_string(), "".to_string()];
        assert_eq!(argv_to_dsl_expr(&args), r#"echo """#);
    }
}
