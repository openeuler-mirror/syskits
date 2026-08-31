use crate::config::{prompt_path_depth_from_env, repl_config_from_env, texts_for_current_locale};
use crate::control::{ControlAction, handle_control_command, push_history_line};
use crate::prompt::{ReplPrompt, build_completion_candidates, build_reedline_editor};
use ctengine::context::{CommandRegistry, DataEngineContext};
use ctengine::entry::eval_expr;
use ctengine::interpreter::print_pipeline_data_repl_with_signal;
use ctengine::legacy_adapter::LegacyToolResolver;
use ctsig::DataSignature;
use reedline::Signal;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use sys_locale::get_locale;

pub(crate) fn parse_pipeline_expr(input: &str) -> Result<ctdsl::Expr, String> {
    ctdsl::parse(input).map_err(|e| e.to_string())
}

fn print_precheck_diagnostics(
    diags: &[ctdsl::PrecheckDiagnostic],
    texts: &crate::config::ReplTexts,
) {
    for d in diags {
        let prefix = match d.level {
            ctdsl::PrecheckLevel::Error => &texts.precheck_error_prefix,
            ctdsl::PrecheckLevel::Warning => &texts.precheck_warning_prefix,
        };
        eprintln!("{} stage[{}]: {}", prefix, d.stage_index, d.message);
    }
}

pub(crate) fn has_precheck_error(diags: &[ctdsl::PrecheckDiagnostic]) -> bool {
    diags
        .iter()
        .any(|d| matches!(d.level, ctdsl::PrecheckLevel::Error))
}

fn looks_like_unknown_command_warning(message: &str) -> bool {
    message.starts_with("precheck: unknown command `")
}

pub(crate) fn filter_precheck_diags_for_repl(
    expr: &ctdsl::Expr,
    diags: Vec<ctdsl::PrecheckDiagnostic>,
    signatures: &HashMap<String, DataSignature>,
    legacy_resolver: Option<LegacyToolResolver>,
) -> Vec<ctdsl::PrecheckDiagnostic> {
    filter_precheck_diags_for_repl_with_known_command(expr, diags, signatures, |name| {
        legacy_resolver.is_some_and(|resolver| resolver(name).is_some())
    })
}

pub(crate) fn filter_precheck_diags_for_repl_with_known_command(
    expr: &ctdsl::Expr,
    diags: Vec<ctdsl::PrecheckDiagnostic>,
    signatures: &HashMap<String, DataSignature>,
    is_known_legacy_command: impl Fn(&str) -> bool,
) -> Vec<ctdsl::PrecheckDiagnostic> {
    diags
        .into_iter()
        .filter(|d| {
            if !looks_like_unknown_command_warning(&d.message) {
                return true;
            }
            let Some(stage) = expr.stages().get(d.stage_index) else {
                return true;
            };
            !stage.force_external
                && !signatures.contains_key(&stage.name)
                && !is_known_legacy_command(&stage.name)
        })
        .collect()
}

/// 运行交互式 REPL。
///
/// 返回值为进程退出码（0 = 正常退出，非 0 = 运行时错误）。
pub fn run_repl(
    registry: CommandRegistry,
    legacy_resolver: Option<LegacyToolResolver>,
    plugin_registry: Option<std::sync::Arc<dyn ctengine::context::PluginProvider>>,
) -> i32 {
    if let Some(lang) = get_locale() {
        rust_i18n::set_locale(&lang);
    }
    let texts = texts_for_current_locale();
    let config = repl_config_from_env();

    let signatures = Arc::new(registry.command_signatures());

    let mut editor = build_reedline_editor(
        build_completion_candidates(signatures.as_ref()),
        &texts,
        signatures.clone(),
        &config,
    );
    let prompt = ReplPrompt {
        prompt_left: texts.prompt_left.clone(),
        path_depth: prompt_path_depth_from_env(),
    };

    let ctx = DataEngineContext::new(registry, legacy_resolver, plugin_registry)
        .with_signal(ctengine::context::SignalHandle::register_sigint())
        .enable_trace();
    let mut debug_enabled = false;
    let mut history_lines: VecDeque<String> = VecDeque::new();

    println!("{}", texts.banner);

    loop {
        match editor.read_line(&prompt) {
            Ok(Signal::Success(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match handle_control_command(trimmed, &mut debug_enabled, &texts, &history_lines) {
                    ControlAction::Exit => break,
                    ControlAction::Continue => {
                        push_history_line(&mut history_lines, trimmed);
                        continue;
                    }
                    ControlAction::NotControl => {}
                }

                ctx.clear_trace();
                let expr = match parse_pipeline_expr(trimmed) {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!("parse error: {e}");
                        continue;
                    }
                };

                let precheck_diags = filter_precheck_diags_for_repl(
                    &expr,
                    ctdsl::precheck_expr(&expr, signatures.as_ref()),
                    signatures.as_ref(),
                    legacy_resolver,
                );
                if !precheck_diags.is_empty() {
                    print_precheck_diagnostics(&precheck_diags, &texts);
                    if has_precheck_error(&precheck_diags) {
                        push_history_line(&mut history_lines, trimmed);
                        continue;
                    }
                }

                match eval_expr(&expr, &ctx) {
                    Ok(data) => {
                        if debug_enabled {
                            ctx.emit_trace_if_enabled();
                        }
                        if let Err(e) = print_pipeline_data_repl_with_signal(data, &ctx.signal) {
                            eprintln!("error: {e}");
                        }
                    }
                    Err(e) => {
                        if debug_enabled {
                            ctx.emit_trace_if_enabled();
                        }
                        eprintln!("error: {e}");
                    }
                }
                // Clear one-shot interrupt state at REPL command boundary.
                // During command execution, ctengine checks the signal non-destructively.
                let _ = ctx.signal.take_interrupted();

                push_history_line(&mut history_lines, trimmed);
            }
            Ok(Signal::CtrlD) => {
                println!("{}", texts.exit_hint);
                break;
            }
            Ok(Signal::CtrlC) => {
                let _ = ctx.signal.take_interrupted();
                println!();
                continue;
            }
            Err(e) => {
                eprintln!("readline error: {e}");
                return 1;
            }
        }
    }

    0
}
