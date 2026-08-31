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

//! `interpreter` — ctengine Direct Interpreter（M1b MVP）。
//!
//! 执行循环：
//! 1. 接受已解析的 `Expr`（管线 AST）和初始 `CtPipelineData`
//! 2. 依次执行各阶段 `Call`，上一阶段输出为下一阶段输入
//! 3. 每次进入新阶段之前检查中断信号
//! 4. 将 AST `Call` 转换为 `DataCall`，从 `CommandRegistry` 查找并调用命令

use ctdsl::Expr;
use ctdsl::ast::{Arg, Call, Lit};
use ctpipeline::{CtPipelineData, CtType, CtValue};
use ctsig::{BoundArg, DataCall, DataSignature};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::time::Instant;

use crate::adapters::DataAdapter;
use crate::compare::{compare_values, resolve_field_path};
use crate::context::{DataEngineContext, SignalHandle};
use crate::display::{ColAlign, print_block_with_pager, render_ascii_table};
use crate::error::CtDiagnosticError;
use crate::execution::{CommandRunner, OutputFormat, OutputProfile, exit_code};
use crate::trace::{StageTrace, TraceStatus};

use crate::ir::IrOp;
#[cfg(feature = "feat_data_experimental")]
use crate::ir_compiler;

/// 执行已解析的管线表达式
pub fn eval_pipeline(
    expr: &Expr,
    mut input: CtPipelineData,
    ctx: &DataEngineContext,
) -> Result<CtPipelineData, CtDiagnosticError> {
    let pipeline_started = Instant::now();

    // 如果开启了 feature=feat_data_experimental，走 IR 路径
    #[cfg(feature = "feat_data_experimental")]
    let ir_pipeline = {
        let nir = crate::binder::Binder::bind(expr)?;
        ir_compiler::compile(&nir)
    };

    #[cfg(not(feature = "feat_data_experimental"))]
    let ir_pipeline = crate::ir::IrPipeline {
        ops: expr.stages().iter().map(IrOp::Command).collect(),
    };

    for (idx, op) in ir_pipeline.ops.iter().enumerate() {
        let cmd_text = match op {
            IrOp::Command(c) => format_call(c),
            IrOp::MergedWhereSelect {
                where_call,
                select_call,
            } => {
                let text = format!("{} | {}", format_call(where_call), format_call(select_call));
                text
            }
        };

        let rows_in = estimate_rows(&input);
        if ctx.signal.interrupted() {
            ctx.record_stage_trace(StageTrace {
                name: None,
                cmd: cmd_text,
                duration_ms: 0,
                rows_in,
                rows_out: 0,
                status: TraceStatus::Skipped,
            });
            ctx.set_trace_total_ms(pipeline_started.elapsed().as_millis() as u64);
            return Err(CtDiagnosticError::simple("interrupted by user").with_code(130));
        }

        let stage_started = Instant::now();

        let mut loop_result = Ok(std::mem::take(&mut input));
        let mut last_rows_out = 0;

        match op {
            IrOp::Command(call) => match loop_result {
                Ok(current_data) => match eval_call(call, current_data, ctx) {
                    Ok(out) => {
                        last_rows_out = estimate_rows(&out);
                        loop_result = Ok(out);
                    }
                    Err(err) => {
                        loop_result = Err(err);
                    }
                },
                Err(_) => {
                    // `loop_result` starts as `Ok` for each stage; this arm is only here to keep
                    // the match exhaustive if future refactors change initialization.
                }
            },
            IrOp::MergedWhereSelect {
                where_call,
                select_call,
            } => match loop_result {
                Ok(current_data) => {
                    match eval_merged_where_select(where_call, select_call, current_data, ctx) {
                        Ok(out) => {
                            last_rows_out = estimate_rows(&out);
                            loop_result = Ok(out);
                        }
                        Err(err) => {
                            loop_result = Err(err);
                        }
                    }
                }
                Err(_) => {
                    // Same as above: unreachable in current flow, retained for exhaustive matching.
                }
            },
        }

        match loop_result {
            Ok(final_data) => {
                ctx.record_stage_trace(StageTrace {
                    name: Some(format!("stage[{idx}]")),
                    cmd: cmd_text,
                    duration_ms: stage_started.elapsed().as_millis() as u64,
                    rows_in,
                    rows_out: last_rows_out,
                    status: TraceStatus::Ok,
                });
                // Put the successfully evaluated output back into 'input' for the next stage
                input = final_data;
            }
            Err(err) => {
                ctx.record_stage_trace(StageTrace {
                    name: Some(format!("stage[{idx}]")),
                    cmd: cmd_text,
                    duration_ms: stage_started.elapsed().as_millis() as u64,
                    rows_in,
                    rows_out: 0,
                    status: TraceStatus::Error(err.to_string()),
                });
                ctx.set_trace_total_ms(pipeline_started.elapsed().as_millis() as u64);
                return Err(err);
            }
        }
    }
    ctx.set_trace_total_ms(pipeline_started.elapsed().as_millis() as u64);
    Ok(input)
}

#[derive(Debug, Clone)]
struct WhereCondition {
    field: String,
    op: String,
    rhs: CtValue,
}

fn eval_merged_where_select(
    where_call: &Call,
    select_call: &Call,
    input: CtPipelineData,
    ctx: &DataEngineContext,
) -> Result<CtPipelineData, CtDiagnosticError> {
    let (cond, cols) = match (
        parse_where_condition_from_call(where_call),
        parse_select_columns_from_call(select_call),
    ) {
        (Some(cond), Some(cols)) if !cols.is_empty() => (cond, cols),
        _ => {
            // Fallback for non-standard argument patterns so behavior stays identical.
            let out = eval_call(where_call, input, ctx)?;
            return eval_call(select_call, out, ctx);
        }
    };

    let meta = ctpipeline::CtPipelineMetadata::default();
    match input {
        CtPipelineData::Value(CtValue::Record(fields), _) => {
            if record_matches(&fields, &cond) {
                Ok(CtPipelineData::Value(
                    CtValue::Record(project_record(fields, &cols)),
                    meta,
                ))
            } else {
                // Equivalent to: where => Empty; select => "select: empty input"
                Err(CtDiagnosticError::simple("select: empty input"))
            }
        }
        CtPipelineData::Value(CtValue::List(items), _) => {
            let mut projected = Vec::new();
            for item in items {
                if let CtValue::Record(fields) = item
                    && record_matches(&fields, &cond)
                {
                    projected.push(CtValue::Record(project_record(fields, &cols)));
                }
            }
            Ok(CtPipelineData::Value(CtValue::List(projected), meta))
        }
        CtPipelineData::Empty => Err(CtDiagnosticError::simple("select: empty input")),
        _ => Err(CtDiagnosticError::simple(
            "where: expected Record or List input",
        )),
    }
}

fn parse_where_condition_from_call(call: &Call) -> Option<WhereCondition> {
    if call.name != "where" {
        return None;
    }
    if call.args.len() != 1 {
        return None;
    }
    match &call.args[0] {
        Arg::Comparison { field, op, rhs, .. } => Some(WhereCondition {
            field: field.clone(),
            op: op.symbol().to_string(),
            rhs: lit_to_ct_value(rhs),
        }),
        _ => None,
    }
}

fn parse_select_columns_from_call(call: &Call) -> Option<Vec<String>> {
    if call.name != "select" {
        return None;
    }
    let mut cols = Vec::new();
    for arg in &call.args {
        match arg {
            Arg::Positional {
                value: Lit::String(s) | Lit::Ident(s),
                ..
            } => cols.push(s.clone()),
            _ => return None,
        }
    }
    Some(cols)
}

fn project_record(fields: Vec<(String, CtValue)>, cols: &[String]) -> Vec<(String, CtValue)> {
    cols.iter()
        .filter_map(|col| {
            fields
                .iter()
                .find(|(k, _)| k == col)
                .map(|(k, v)| (k.clone(), v.clone()))
        })
        .collect()
}

fn record_matches(fields: &[(String, CtValue)], cond: &WhereCondition) -> bool {
    resolve_field_path(fields, &cond.field)
        .map(|v| compare_values(v, &cond.op, &cond.rhs))
        .unwrap_or(false)
}

/// 执行单个 Call 节点
fn eval_call(
    call: &Call,
    input: CtPipelineData,
    ctx: &DataEngineContext,
) -> Result<CtPipelineData, CtDiagnosticError> {
    if let Some(external_name) = call.name.strip_prefix('~') {
        if external_name.is_empty() {
            return Err(CtDiagnosticError::with_span(
                "external command prefix `~` requires a command name",
                call.span.clone(),
            ));
        }
        let ext_args = external_args_from_call(call);
        let mut spec = crate::external::ExternalCallSpec::quick(external_name, &ext_args);
        spec.exit_policy = crate::external::ExternalExitPolicy::AllowNonZero;
        return crate::external::ExternalExecutor::run(spec, input, ctx);
    }

    if let Some(cmd) = ctx.registry.get(&call.name) {
        let sig = cmd.signature();
        if let Some(expected) = sig.input_type
            && !pipeline_data_matches_type(&input, expected)
        {
            return Err(CtDiagnosticError::with_span(
                format!(
                    "{}: input type mismatch, expected {:?}, got {:?}",
                    call.name,
                    expected,
                    pipeline_data_type(&input)
                ),
                call.span.clone(),
            ));
        }
        let data_call = build_data_call(call, Some(&sig))?;
        validate_data_call_against_signature(call, &data_call, &sig)?;
        let core = DataAdapter::new(cmd.as_ref());
        let out = CommandRunner::run(&core, &data_call, input, ctx)?;
        if let Some(expected) = sig.output_type
            && !pipeline_data_matches_type(&out, expected)
        {
            return Err(CtDiagnosticError::with_span(
                format!(
                    "{}: output type mismatch, expected {:?}, got {:?}",
                    call.name,
                    expected,
                    pipeline_data_type(&out)
                ),
                call.span.clone(),
            ));
        }
        return Ok(out);
    }

    if let Some(registry) = &ctx.plugin_registry {
        if let Some(cmd) = registry.get_command(&call.name) {
            let sig = cmd.signature();
            let data_call = build_data_call(call, Some(&sig))?;
            validate_data_call_against_signature(call, &data_call, &sig)?;
            return cmd.run(&data_call, input, ctx);
        }
    }

    if let Some(adapter) = &ctx.legacy_adapter {
        if adapter.can_resolve(&call.name) {
            let spec = adapter.external_call_spec(call)?;
            return crate::external::ExternalExecutor::run(spec, input, ctx);
        }
    }

    // 3rd fallback: treat as external binary
    let ext_args = external_args_from_call(call);
    let spec = crate::external::ExternalCallSpec::quick(&call.name, &ext_args);
    crate::external::ExternalExecutor::run(spec, input, ctx)
}

fn external_args_from_call(call: &Call) -> Vec<String> {
    let mut ext_args = Vec::new();
    for arg in &call.args {
        match arg {
            Arg::Positional { value, .. } => ext_args.push(value.to_string()),
            Arg::LongFlag { name, .. } => ext_args.push(format!("--{name}")),
            Arg::LongFlagValue { name, value, .. } => {
                ext_args.push(format!("--{name}"));
                ext_args.push(value.to_string());
            }
            Arg::ShortFlag { name, .. } => ext_args.push(format!("-{name}")),
            Arg::Comparison { field, op, rhs, .. } => {
                // Not standard for external, but best effort literal conversion
                ext_args.push(field.clone());
                ext_args.push(op.symbol().to_string());
                ext_args.push(rhs.to_string());
            }
            Arg::WhereExpr {
                conditions,
                logic_ops,
                ..
            } => {
                for (idx, (field, op, rhs)) in conditions.iter().enumerate() {
                    if idx > 0
                        && let Some(logic) = logic_ops.get(idx - 1)
                    {
                        ext_args.push(logic.clone());
                    }
                    ext_args.push(field.clone());
                    ext_args.push(op.symbol().to_string());
                    ext_args.push(rhs.to_string());
                }
            }
        }
    }
    ext_args
}

fn pipeline_data_type(data: &CtPipelineData) -> CtType {
    match data {
        CtPipelineData::Empty => CtType::Nothing,
        CtPipelineData::Value(v, _) => v.value_type(),
        CtPipelineData::ListStream(_) => CtType::ListStream,
        CtPipelineData::ByteStream(_) => CtType::ByteStream,
    }
}

fn pipeline_data_matches_type(data: &CtPipelineData, expected: CtType) -> bool {
    if expected == CtType::Any {
        return true;
    }
    let actual = pipeline_data_type(data);
    if actual == expected {
        return true;
    }
    matches!(
        (actual, expected),
        (CtType::ListStream, CtType::List) | (CtType::List, CtType::ListStream)
    )
}

/// 将 AST `Call` 转换为 `DataCall`（参数绑定）
///
/// 按 LLD §5.3 填充 head / command_name / positionals / flags / rest。
fn build_data_call(
    call: &Call,
    sig: Option<&DataSignature>,
) -> Result<DataCall, CtDiagnosticError> {
    let mut positionals: Vec<BoundArg> = Vec::new();
    let mut flags: HashMap<String, Option<BoundArg>> = HashMap::new();
    let is_run_external = call.name == "run-external";

    let mut idx = 0usize;
    while idx < call.args.len() {
        match &call.args[idx] {
            Arg::Positional { value, span } => {
                let value = if sig.is_some_and(DataSignature::allows_unknown) {
                    lit_to_unknown_argv_ct_value(value)
                } else {
                    lit_to_ct_value(value)
                };
                positionals.push(BoundArg::new(value, Some(span.clone())));
            }
            Arg::LongFlag { name, span } => {
                if (is_run_external && !is_run_external_control_flag(name))
                    || sig.is_some_and(|sig| sig.allows_unknown() && !signature_has_flag(sig, name))
                {
                    positionals.push(BoundArg::new(
                        CtValue::String(format!("--{name}")),
                        Some(span.clone()),
                    ));
                } else {
                    let expects_value = signature_flag_expects_value(sig, name);
                    if expects_value {
                        if let Some(Arg::Positional { value, span }) = call.args.get(idx + 1) {
                            flags.insert(
                                name.clone(),
                                Some(BoundArg::new(lit_to_ct_value(value), Some(span.clone()))),
                            );
                            idx += 1;
                        } else {
                            return Err(flag_value_missing_error(call, &format!("--{name}"), span));
                        }
                    } else {
                        flags.insert(name.clone(), None);
                    }
                }
            }
            Arg::LongFlagValue {
                name,
                value,
                value_span,
                span,
            } => {
                if is_run_external && !is_run_external_control_flag(name) {
                    positionals.push(BoundArg::new(
                        CtValue::String(format!("--{name}")),
                        Some(span.clone()),
                    ));
                    positionals.push(BoundArg::new(
                        lit_to_ct_value(value),
                        Some(value_span.clone()),
                    ));
                } else if sig
                    .is_some_and(|sig| sig.allows_unknown() && !signature_has_flag(sig, name))
                {
                    positionals.push(BoundArg::new(
                        CtValue::String(format!("--{name}={}", lit_to_ct_value(value).to_text())),
                        Some(span.clone()),
                    ));
                } else if signature_flag_expects_value(sig, name) {
                    flags.insert(
                        name.clone(),
                        Some(BoundArg::new(
                            lit_to_ct_value(value),
                            Some(value_span.clone()),
                        )),
                    );
                } else {
                    // Compatibility: if a switch flag was parsed as `--flag value`,
                    // treat the value as positional instead of swallowing it.
                    flags.insert(name.clone(), None);
                    positionals.push(BoundArg::new(
                        lit_to_ct_value(value),
                        Some(value_span.clone()),
                    ));
                }
            }
            Arg::ShortFlag { name, span } => {
                if is_run_external
                    || sig.is_some_and(|sig| {
                        sig.allows_unknown() && !signature_has_flag(sig, &name.to_string())
                    })
                {
                    positionals.push(BoundArg::new(
                        CtValue::String(format!("-{name}")),
                        Some(span.clone()),
                    ));
                } else {
                    let expects_value = signature_short_flag_expects_value(sig, *name);
                    if expects_value {
                        if let Some(Arg::Positional { value, span }) = call.args.get(idx + 1) {
                            flags.insert(
                                name.to_string(),
                                Some(BoundArg::new(lit_to_ct_value(value), Some(span.clone()))),
                            );
                            idx += 1;
                        } else {
                            return Err(flag_value_missing_error(call, &format!("-{name}"), span));
                        }
                    } else {
                        flags.insert(name.to_string(), None);
                    }
                }
            }
            Arg::Comparison {
                field,
                op,
                rhs,
                span,
            } => {
                // where 命令的比较表达式：打包为 Record 位置参数
                let record = CtValue::Record(vec![
                    ("field".into(), CtValue::String(field.clone())),
                    ("op".into(), CtValue::String(op.symbol().to_string())),
                    ("rhs".into(), lit_to_ct_value(rhs)),
                ]);
                positionals.push(BoundArg::new(record, Some(span.clone())));
            }
            Arg::WhereExpr {
                conditions,
                logic_ops,
                span,
            } => {
                let cond_values = conditions
                    .iter()
                    .map(|(field, op, rhs)| {
                        CtValue::Record(vec![
                            ("field".into(), CtValue::String(field.clone())),
                            ("op".into(), CtValue::String(op.symbol().to_string())),
                            ("rhs".into(), lit_to_ct_value(rhs)),
                        ])
                    })
                    .collect::<Vec<_>>();
                let logic_values = logic_ops
                    .iter()
                    .map(|op| CtValue::String(op.clone()))
                    .collect::<Vec<_>>();
                let record = CtValue::Record(vec![
                    ("conditions".into(), CtValue::List(cond_values)),
                    ("logic".into(), CtValue::List(logic_values)),
                ]);
                positionals.push(BoundArg::new(record, Some(span.clone())));
            }
        }
        idx += 1;
    }

    Ok(DataCall {
        head: Some(call.span.clone()),
        command_name: call.name.clone(),
        positionals,
        flags,
        rest: Vec::new(),
    })
}

fn validate_data_call_against_signature(
    call: &Call,
    data_call: &DataCall,
    sig: &DataSignature,
) -> Result<(), CtDiagnosticError> {
    if !sig.allows_unknown() {
        for flag_name in data_call.flags.keys() {
            if !signature_has_flag(sig, flag_name) {
                let flag_display = render_flag_name(flag_name);
                return Err(CtDiagnosticError::with_span(
                    format!("{}: unknown flag `{flag_display}`", call.name),
                    call.span.clone(),
                )
                .with_code(exit_code::USAGE_ERROR));
            }
        }
    }

    let required = sig.required_positionals().len();
    let optional = sig.optional_positionals().len();
    let actual = data_call.positionals.len();

    if actual < required {
        let missing_name = sig
            .required_positionals()
            .get(actual)
            .map(|arg| arg.name)
            .unwrap_or("argument");
        return Err(CtDiagnosticError::with_span(
            format!("{}: missing required argument `{missing_name}`", call.name),
            call.span.clone(),
        )
        .with_code(exit_code::USAGE_ERROR));
    }

    if sig.rest_positional_arg().is_none() {
        let max_positional = required + optional;
        if actual > max_positional {
            let unexpected = data_call
                .positionals
                .get(max_positional)
                .map(|arg| arg.value.to_text())
                .unwrap_or_else(|| "<unknown>".to_string());
            return Err(CtDiagnosticError::with_span(
                format!(
                    "{}: unexpected positional argument `{unexpected}`",
                    call.name
                ),
                call.span.clone(),
            )
            .with_code(exit_code::USAGE_ERROR));
        }
    }

    Ok(())
}

fn render_flag_name(name: &str) -> String {
    if name.chars().count() == 1 {
        format!("-{name}")
    } else {
        format!("--{name}")
    }
}

fn flag_value_missing_error(
    call: &Call,
    display: &str,
    span: &ctpipeline::CtSpan,
) -> CtDiagnosticError {
    CtDiagnosticError::with_span(
        format!("{}: flag `{display}` requires a value", call.name),
        span.clone(),
    )
    .with_code(exit_code::USAGE_ERROR)
}

fn signature_flag_expects_value(sig: Option<&DataSignature>, long_name: &str) -> bool {
    sig.and_then(|s| s.flags.iter().find(|f| f.long == long_name))
        .and_then(|flag| flag.value_type)
        .is_some()
}

fn signature_short_flag_expects_value(sig: Option<&DataSignature>, short_name: char) -> bool {
    sig.and_then(|s| s.flags.iter().find(|f| f.short == Some(short_name)))
        .and_then(|flag| flag.value_type)
        .is_some()
}

fn signature_has_flag(sig: &DataSignature, name: &str) -> bool {
    sig.flags.iter().any(|flag| {
        flag.long == name
            || flag
                .short
                .is_some_and(|short| name.chars().count() == 1 && name.starts_with(short))
    })
}

fn is_run_external_control_flag(name: &str) -> bool {
    matches!(
        name,
        "stdout-mode" | "stderr-mode" | "stdin-mode" | "exit-policy" | "timeout-ms"
    )
}

/// 将 AST 字面量转为 CtValue
pub fn lit_to_ct_value(lit: &Lit) -> CtValue {
    match lit {
        Lit::Int(n) => CtValue::Int(*n),
        Lit::Float(f) => CtValue::Float(*f),
        Lit::String(s) => CtValue::String(s.clone()),
        Lit::Bool(b) => CtValue::Bool(*b),
        Lit::Size(b) => CtValue::Size(*b),
        Lit::Duration(ns) => CtValue::Duration(*ns),
        Lit::DateTime(ns) => CtValue::DateTime(*ns),
        Lit::Ident(s) => CtValue::String(s.clone()),
    }
}

fn lit_to_unknown_argv_ct_value(lit: &Lit) -> CtValue {
    CtValue::String(lit.to_string())
}

fn interrupted_error() -> CtDiagnosticError {
    CtDiagnosticError::simple("interrupted by user").with_code(130)
}

fn print_pipeline_bytes(
    mut bs: ctpipeline::CtByteStream,
    signal: Option<&SignalHandle>,
) -> Result<(), CtDiagnosticError> {
    let mut stdout = std::io::stdout();
    let mut buf = [0u8; 8192];
    loop {
        if signal.is_some_and(|s| s.interrupted()) {
            return Err(interrupted_error());
        }
        let n = match bs.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::BrokenPipe {
                    break;
                }
                return Err(CtDiagnosticError::simple(format!(
                    "Error reading ByteStream: {e}"
                )));
            }
        };
        if n == 0 {
            break;
        }
        if let Err(e) = stdout.write_all(&buf[..n]) {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                break;
            }
            return Err(CtDiagnosticError::simple(format!(
                "Error writing ByteStream: {e}"
            )));
        }
    }
    Ok(())
}

/// 将 `CtPipelineData` 打印到 stdout（稳定的一次性输出格式，CLI/workflow 使用）
pub fn try_print_pipeline_data(data: CtPipelineData) -> Result<(), CtDiagnosticError> {
    try_print_pipeline_data_with_profile(data, &OutputProfile::text_stream())
}

/// 按输出视图策略打印 `CtPipelineData`。
pub fn try_print_pipeline_data_with_profile(
    data: CtPipelineData,
    profile: &OutputProfile,
) -> Result<(), CtDiagnosticError> {
    try_print_pipeline_data_with_profile_and_signal(data, profile, None)
}

pub fn try_print_pipeline_data_with_profile_and_signal(
    data: CtPipelineData,
    profile: &OutputProfile,
    signal: Option<&SignalHandle>,
) -> Result<(), CtDiagnosticError> {
    match profile.format {
        OutputFormat::Classic => print_pipeline_data_classic(data, signal),
        OutputFormat::Text => print_pipeline_data_text(data, signal),
        OutputFormat::Table => print_pipeline_data_table(data, profile.use_pager, signal),
        OutputFormat::Json => print_pipeline_data_json(data, signal),
        OutputFormat::Raw => print_pipeline_data_raw(data, signal),
        OutputFormat::Auto => {
            print_pipeline_data_auto(data, profile.stdout_is_tty, profile.use_pager, signal)
        }
    }
}

fn print_pipeline_data_auto(
    data: CtPipelineData,
    stdout_is_tty: bool,
    use_pager: bool,
    signal: Option<&SignalHandle>,
) -> Result<(), CtDiagnosticError> {
    if stdout_is_tty {
        print_pipeline_data_table(data, use_pager, signal)
    } else {
        print_pipeline_data_text(data, signal)
    }
}

fn write_stderr_if_present(text: Option<&str>) -> Result<(), CtDiagnosticError> {
    if let Some(text) = text {
        let mut stderr = std::io::stderr().lock();
        stderr
            .write_all(text.as_bytes())
            .map_err(|e| CtDiagnosticError::simple(format!("write error: {e}")))?;
    }
    Ok(())
}

fn print_pipeline_data_classic(
    data: CtPipelineData,
    signal: Option<&SignalHandle>,
) -> Result<(), CtDiagnosticError> {
    match data {
        CtPipelineData::Value(value, metadata) => {
            if let Some(bytes) = metadata.classic_bytes {
                std::io::stdout()
                    .lock()
                    .write_all(&bytes)
                    .map_err(|e| CtDiagnosticError::simple(format!("write error: {e}")))?;
            } else if let Some(text) = metadata.classic_text {
                if metadata.classic_append_newline {
                    println!("{text}");
                } else {
                    std::io::stdout()
                        .lock()
                        .write_all(text.as_bytes())
                        .map_err(|e| CtDiagnosticError::simple(format!("write error: {e}")))?;
                }
            } else if let CtValue::List(items) = value {
                if let Some(text) = render_records_coreutils_text(&items) {
                    print!("{text}");
                } else {
                    println!("{}", format_ct_value(&CtValue::List(items)));
                }
            } else {
                println!("{}", format_ct_value(&value));
            }
            write_stderr_if_present(metadata.stderr_text.as_deref())?;
        }
        other => print_pipeline_data_text(other, signal)?,
    }
    Ok(())
}

fn print_pipeline_data_text(
    data: CtPipelineData,
    signal: Option<&SignalHandle>,
) -> Result<(), CtDiagnosticError> {
    match data {
        CtPipelineData::Empty => {}
        CtPipelineData::Value(v, metadata) => {
            println!("{}", format_ct_value(&v));
            write_stderr_if_present(metadata.stderr_text.as_deref())?;
        }
        CtPipelineData::ListStream(stream) => {
            for v in stream {
                if signal.is_some_and(|s| s.interrupted()) {
                    return Err(interrupted_error());
                }
                println!("{}", format_ct_value(&v));
            }
        }
        CtPipelineData::ByteStream(bs) => {
            print_pipeline_bytes(bs, signal)?;
        }
    }
    Ok(())
}

fn print_pipeline_data_raw(
    data: CtPipelineData,
    signal: Option<&SignalHandle>,
) -> Result<(), CtDiagnosticError> {
    match data {
        CtPipelineData::ByteStream(bs) => print_pipeline_bytes(bs, signal),
        other => print_pipeline_data_text(other, signal),
    }
}

fn print_pipeline_data_json(
    data: CtPipelineData,
    signal: Option<&SignalHandle>,
) -> Result<(), CtDiagnosticError> {
    let mut stdout = std::io::stdout();
    match data {
        CtPipelineData::Empty => {}
        CtPipelineData::Value(v, metadata) => {
            let json = ct_value_to_json(&v);
            serde_json::to_writer(&mut stdout, &json)
                .map_err(|e| CtDiagnosticError::simple(format!("json render failed: {e}")))?;
            println!();
            write_stderr_if_present(metadata.stderr_text.as_deref())?;
        }
        CtPipelineData::ListStream(stream) => {
            let mut values = Vec::new();
            for v in stream {
                if signal.is_some_and(|s| s.interrupted()) {
                    return Err(interrupted_error());
                }
                values.push(ct_value_to_json(&v));
            }
            serde_json::to_writer(&mut stdout, &values)
                .map_err(|e| CtDiagnosticError::simple(format!("json render failed: {e}")))?;
            println!();
        }
        CtPipelineData::ByteStream(bs) => {
            let json = ct_byte_stream_to_json(bs, signal)?;
            serde_json::to_writer(&mut stdout, &json)
                .map_err(|e| CtDiagnosticError::simple(format!("json render failed: {e}")))?;
            println!();
        }
    }
    Ok(())
}

fn ct_byte_stream_to_json(
    mut bs: ctpipeline::CtByteStream,
    signal: Option<&SignalHandle>,
) -> Result<serde_json::Value, CtDiagnosticError> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        if signal.is_some_and(|s| s.interrupted()) {
            return Err(interrupted_error());
        }
        let n = bs
            .read(&mut buf)
            .map_err(|e| CtDiagnosticError::simple(format!("json render failed: {e}")))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    Ok(serde_json::Value::Array(
        bytes
            .into_iter()
            .map(|b| serde_json::Value::Number((b as u64).into()))
            .collect(),
    ))
}

fn ct_value_to_json(v: &CtValue) -> serde_json::Value {
    match v {
        CtValue::Nothing => serde_json::Value::Null,
        CtValue::Bool(b) => serde_json::Value::Bool(*b),
        CtValue::Int(n) => serde_json::Value::Number((*n).into()),
        CtValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        CtValue::String(s) => serde_json::Value::String(s.clone()),
        CtValue::Binary(_) => serde_json::Value::String("<binary>".to_string()),
        CtValue::DateTime(_) | CtValue::Duration(_) | CtValue::Size(_) => {
            serde_json::Value::String(v.to_text())
        }
        CtValue::Record(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), ct_value_to_json(v)))
                .collect(),
        ),
        CtValue::List(items) => {
            serde_json::Value::Array(items.iter().map(ct_value_to_json).collect())
        }
        CtValue::Error(e) => serde_json::Value::String(format!("<error: {e}>")),
    }
}

fn print_pipeline_data_table(
    data: CtPipelineData,
    use_pager: bool,
    signal: Option<&SignalHandle>,
) -> Result<(), CtDiagnosticError> {
    match data {
        CtPipelineData::Empty => {}
        CtPipelineData::Value(CtValue::List(items), metadata) => {
            if let Some(table) = render_list_of_records_table(&items) {
                print_table_block(&table, use_pager);
            } else {
                println!("{}", format_ct_value(&CtValue::List(items)));
            }
            write_stderr_if_present(metadata.stderr_text.as_deref())?;
        }
        CtPipelineData::Value(CtValue::Record(fields), metadata) => {
            let rows = fields
                .iter()
                .map(|(k, v)| vec![k.clone(), format_ct_value(v)])
                .collect::<Vec<_>>();
            let aligns = vec![ColAlign::Left, ColAlign::Left];
            let table = render_ascii_table(vec!["field".into(), "value".into()], rows, aligns);
            print_table_block(&table, use_pager);
            write_stderr_if_present(metadata.stderr_text.as_deref())?;
        }
        CtPipelineData::Value(v, metadata) => {
            println!("{}", format_ct_value(&v));
            write_stderr_if_present(metadata.stderr_text.as_deref())?;
        }
        CtPipelineData::ListStream(stream) => {
            for v in stream {
                if signal.is_some_and(|s| s.interrupted()) {
                    return Err(interrupted_error());
                }
                println!("{}", format_ct_value(&v));
            }
        }
        CtPipelineData::ByteStream(bs) => {
            print_pipeline_bytes(bs, signal)?;
        }
    }
    Ok(())
}

fn render_records_coreutils_text(items: &[CtValue]) -> Option<String> {
    if items.is_empty() || !items.iter().all(|v| matches!(v, CtValue::Record(_))) {
        return None;
    }

    let headers = stable_record_headers(items);
    if headers.is_empty() {
        return None;
    }

    let mut out = String::new();
    for item in items {
        let CtValue::Record(fields) = item else {
            continue;
        };
        let row = headers
            .iter()
            .map(|h| {
                fields
                    .iter()
                    .find(|(k, _)| k == h)
                    .map(|(_, v)| format_ct_value_coreutils(v))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        out.push_str(&row.join("\t"));
        out.push('\n');
    }

    Some(out)
}

fn stable_record_headers(items: &[CtValue]) -> Vec<String> {
    const PREFERRED: &[&str] = &[
        "name",
        "type",
        "size",
        "modified",
        "permissions",
        "pid",
        "cpu",
        "mem",
        "status",
        "mount",
        "used",
        "available",
    ];

    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        if let CtValue::Record(fields) = item {
            for (k, _) in fields {
                seen.insert(k.clone());
            }
        }
    }

    let mut headers = Vec::new();
    for key in PREFERRED {
        if seen.remove(*key) {
            headers.push((*key).to_string());
        }
    }
    let mut rest = seen.into_iter().collect::<Vec<_>>();
    rest.sort();
    headers.extend(rest);
    headers
}

fn format_ct_value_coreutils(v: &CtValue) -> String {
    match v {
        CtValue::Nothing => String::new(),
        CtValue::Size(bytes) => bytes.to_string(),
        _ => format_ct_value(v),
    }
}

fn print_table_block(block: &str, use_pager: bool) {
    if use_pager {
        print_block_with_pager(block);
    } else {
        println!("{block}");
    }
}

/// 兼容旧签名：忽略打印失败（例如 BrokenPipe）。
#[deprecated(note = "use try_print_pipeline_data for error propagation")]
pub fn print_pipeline_data(data: CtPipelineData) {
    let _ = try_print_pipeline_data(data);
}

/// 将 `CtPipelineData` 打印到 stdout（REPL 友好展示：Record/List<Record> 表格化）
pub fn print_pipeline_data_repl(data: CtPipelineData) -> Result<(), CtDiagnosticError> {
    try_print_pipeline_data_with_profile(data, &OutputProfile::for_repl())
}

pub fn print_pipeline_data_repl_with_signal(
    data: CtPipelineData,
    signal: &SignalHandle,
) -> Result<(), CtDiagnosticError> {
    try_print_pipeline_data_with_profile_and_signal(data, &OutputProfile::for_repl(), Some(signal))
}

fn render_list_of_records_table(items: &[CtValue]) -> Option<String> {
    if items.is_empty() {
        // Keep empty list rendering consistent with format_ct_value => "[]".
        return None;
    }
    if !items.iter().all(|v| matches!(v, CtValue::Record(_))) {
        return None;
    }

    let mut headers: Vec<String> = Vec::new();
    for item in items {
        if let CtValue::Record(fields) = item {
            for (k, _) in fields {
                if !headers.iter().any(|h| h == k) {
                    headers.push(k.clone());
                }
            }
        }
    }

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(items.len());
    let mut aligns: Vec<ColAlign> = vec![ColAlign::Right; headers.len()];
    for item in items {
        let CtValue::Record(fields) = item else {
            continue;
        };
        let mut row = Vec::with_capacity(headers.len());
        for (idx, h) in headers.iter().enumerate() {
            let value = fields
                .iter()
                .find(|(k, _)| k == h)
                .map(|(_, v)| {
                    if !matches!(v, CtValue::Int(_) | CtValue::Float(_)) {
                        aligns[idx] = ColAlign::Left;
                    }
                    format_ct_value(v)
                })
                .unwrap_or_else(|| {
                    aligns[idx] = ColAlign::Left;
                    String::new()
                });
            row.push(value);
        }
        rows.push(row);
    }

    Some(render_ascii_table(headers, rows, aligns))
}

/// 将 CtValue 格式化为可打印字符串
pub fn format_ct_value(v: &CtValue) -> String {
    match v {
        CtValue::Nothing => String::new(),
        CtValue::Bool(b) => b.to_string(),
        CtValue::Int(n) => n.to_string(),
        CtValue::Float(f) => f.to_string(),
        CtValue::String(s) => s.clone(),
        CtValue::Binary(b) => format!("<binary {} bytes>", b.len()),
        CtValue::DateTime(_) | CtValue::Duration(_) | CtValue::Size(_) => v.to_text(),
        CtValue::Record(fields) => {
            let pairs: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{k}: {}", format_ct_value(v)))
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        CtValue::List(items) => {
            let elems: Vec<String> = items.iter().map(format_ct_value).collect();
            format!("[{}]", elems.join(", "))
        }
        CtValue::Error(e) => format!("<error: {e}>"),
    }
}

fn estimate_rows(data: &CtPipelineData) -> usize {
    match data {
        CtPipelineData::Empty => 0,
        CtPipelineData::Value(CtValue::List(items), _) => items.len(),
        CtPipelineData::Value(CtValue::Nothing, _) => 0,
        CtPipelineData::Value(_, _) => 1,
        CtPipelineData::ListStream(_) => 0,
        CtPipelineData::ByteStream(_) => 0,
    }
}

fn format_call(call: &Call) -> String {
    if call.args.is_empty() {
        return call.name.clone();
    }
    let mut parts = Vec::with_capacity(call.args.len() + 1);
    parts.push(call.name.clone());
    for arg in &call.args {
        match arg {
            Arg::Positional { value, .. } => parts.push(value.to_string()),
            Arg::LongFlag { name, .. } => parts.push(format!("--{name}")),
            Arg::LongFlagValue { name, value, .. } => {
                parts.push(format!("--{name}"));
                parts.push(value.to_string());
            }
            Arg::ShortFlag { name, .. } => parts.push(format!("-{name}")),
            Arg::Comparison { field, op, rhs, .. } => {
                parts.push(field.clone());
                parts.push(op.symbol().to_string());
                parts.push(rhs.to_string());
            }
            Arg::WhereExpr {
                conditions,
                logic_ops,
                ..
            } => {
                for (idx, (field, op, rhs)) in conditions.iter().enumerate() {
                    if idx > 0
                        && let Some(logic) = logic_ops.get(idx - 1)
                    {
                        parts.push(logic.clone());
                    }
                    parts.push(field.clone());
                    parts.push(op.symbol().to_string());
                    parts.push(rhs.to_string());
                }
            }
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::DataCommand;
    use crate::context::{CommandRegistry, DataEngineContext};
    use ctpipeline::{CtByteStream, CtPipelineMetadata};
    use ctsig::{CtFlag, CtPositionalArg, DataSignature};
    use std::io::{self, Read};

    // ── 测试用 stub 命令 ──────────────────────────────────

    struct Echo;
    impl DataCommand for Echo {
        fn signature(&self) -> DataSignature {
            DataSignature::new("echo", "pass input through")
        }
        fn run(
            &self,
            _call: &DataCall,
            input: CtPipelineData,
            _ctx: &DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            Ok(input)
        }
    }

    struct Const;
    impl DataCommand for Const {
        fn signature(&self) -> DataSignature {
            DataSignature::new("const", "returns fixed value 42")
        }
        fn run(
            &self,
            _call: &DataCall,
            _input: CtPipelineData,
            _ctx: &DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            Ok(CtPipelineData::Value(
                CtValue::Int(42),
                CtPipelineMetadata::default(),
            ))
        }
    }

    fn make_ctx() -> DataEngineContext {
        let factories: Vec<(&'static str, crate::command::DataCommandFactory)> = vec![
            (
                "echo",
                (|| Box::new(Echo)) as crate::command::DataCommandFactory,
            ),
            (
                "const",
                (|| Box::new(Const)) as crate::command::DataCommandFactory,
            ),
        ];
        DataEngineContext::new(CommandRegistry::from_factories(&factories), None, None)
    }

    #[test]
    fn test_eval_single_command_passthrough() {
        let expr = ctdsl::parse("echo").unwrap();
        let input = CtPipelineData::Value(CtValue::Int(1), CtPipelineMetadata::default());
        let ctx = make_ctx();
        let result = eval_pipeline(&expr, input, &ctx).unwrap();
        assert!(matches!(result, CtPipelineData::Value(CtValue::Int(1), _)));
    }

    #[test]
    fn test_eval_pipeline_two_stages() {
        let expr = ctdsl::parse("const | echo").unwrap();
        let result = eval_pipeline(&expr, CtPipelineData::Empty, &make_ctx()).unwrap();
        assert!(matches!(result, CtPipelineData::Value(CtValue::Int(42), _)));
    }

    #[test]
    fn test_eval_unknown_command_error() {
        // Since the interpreter now falls back to external process execution,
        // an unknown command will fail with a "failed to spawn" or IO error
        let expr = ctdsl::parse("__syskits_no_such_cmd_xyz__").unwrap();
        let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None);
        let err = eval_pipeline(&expr, CtPipelineData::Empty, &ctx).unwrap_err();
        // The error should come from failing to spawn the external process
        assert!(
            err.to_string().contains("failed to spawn") || err.to_string().contains("No such file"),
            "Unexpected error: {err}"
        );
    }

    #[test]
    fn test_eval_interrupt_signal() {
        let expr = ctdsl::parse("echo").unwrap();
        let ctx = make_ctx();
        ctx.signal.trigger();
        let err = eval_pipeline(&expr, CtPipelineData::Empty, &ctx).unwrap_err();
        assert!(err.to_string().contains("interrupted"));
    }

    #[test]
    fn test_eval_interrupt_signal_not_consumed() {
        let expr = ctdsl::parse("echo").unwrap();
        let ctx = make_ctx();
        ctx.signal.trigger();
        let _ = eval_pipeline(&expr, CtPipelineData::Empty, &ctx).unwrap_err();
        assert!(ctx.signal.interrupted());
    }

    #[test]
    fn test_lit_to_ct_value() {
        assert!(matches!(lit_to_ct_value(&Lit::Int(7)), CtValue::Int(7)));
        assert!(matches!(
            lit_to_ct_value(&Lit::Bool(true)),
            CtValue::Bool(true)
        ));
        assert!(matches!(
            lit_to_ct_value(&Lit::Ident("x".into())),
            CtValue::String(_)
        ));
    }

    #[test]
    fn test_format_ct_value() {
        assert_eq!(format_ct_value(&CtValue::Int(42)), "42");
        assert_eq!(format_ct_value(&CtValue::Nothing), "");
        assert_eq!(format_ct_value(&CtValue::Bool(false)), "false");
    }

    #[test]
    fn test_stripped_width_counts_cjk_and_ignores_ansi_escape() {
        assert_eq!(crate::display::stripped_width("ab中"), 4);
        assert_eq!(crate::display::stripped_width("\u{1b}[31m中\u{1b}[0m"), 2);
    }

    #[test]
    fn test_clip_with_ellipsis_respects_display_width() {
        assert_eq!(crate::display::clip_with_ellipsis("中文测试", 5), "中...");
    }

    #[test]
    fn test_render_list_of_records_table() {
        let items = vec![
            CtValue::Record(vec![
                ("name".into(), CtValue::String("a".into())),
                ("size".into(), CtValue::Int(10)),
            ]),
            CtValue::Record(vec![
                ("name".into(), CtValue::String("b".into())),
                ("size".into(), CtValue::Int(20)),
            ]),
        ];
        let table = render_list_of_records_table(&items).expect("table expected");
        println!("RENDERED TABLE:\n{table}");
        assert!(table.contains("name"));
        assert!(table.contains("size"));
        assert!(table.contains("a"));
        assert!(table.contains("20"));
    }

    #[test]
    fn test_render_list_of_records_table_none_for_non_record() {
        let items = vec![CtValue::Int(1), CtValue::Int(2)];
        assert!(render_list_of_records_table(&items).is_none());
    }

    #[test]
    fn test_render_list_of_records_table_none_for_empty_list() {
        let items: Vec<CtValue> = vec![];
        assert!(render_list_of_records_table(&items).is_none());
    }

    #[test]
    fn test_render_records_coreutils_text() {
        let items = vec![
            CtValue::Record(vec![
                ("name".to_string(), CtValue::String("a.txt".to_string())),
                ("size".to_string(), CtValue::Size(42)),
            ]),
            CtValue::Record(vec![
                ("name".to_string(), CtValue::String("b.txt".to_string())),
                ("size".to_string(), CtValue::Size(1024)),
            ]),
        ];
        let text = render_records_coreutils_text(&items).expect("should render");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "a.txt\t42");
        assert_eq!(lines[1], "b.txt\t1024");
    }

    #[test]
    fn test_ct_value_to_json_record_is_object() {
        let value = CtValue::Record(vec![
            ("name".to_string(), CtValue::String("a.txt".to_string())),
            ("size".to_string(), CtValue::Size(42)),
        ]);
        let json = ct_value_to_json(&value);
        let serde_json::Value::Object(obj) = json else {
            panic!("record must render as json object");
        };
        assert_eq!(
            obj.get("name"),
            Some(&serde_json::Value::String("a.txt".to_string()))
        );
        assert!(matches!(
            obj.get("size"),
            Some(serde_json::Value::String(_))
        ));
    }

    #[test]
    fn test_ct_byte_stream_to_json_is_numeric_array() {
        let stream = ctpipeline::CtByteStream::new(
            std::io::Cursor::new(vec![0_u8, 255_u8, 16_u8]),
            CtPipelineMetadata::default(),
        );
        let json =
            ct_byte_stream_to_json(stream, None).expect("byte stream should serialize to json");
        assert_eq!(json, serde_json::json!([0, 255, 16]));
    }

    #[test]
    fn test_pipeline_data_matches_list_and_stream() {
        let list_data = CtPipelineData::Value(
            CtValue::List(vec![CtValue::Int(1)]),
            CtPipelineMetadata::default(),
        );
        assert!(pipeline_data_matches_type(&list_data, CtType::List));
        assert!(pipeline_data_matches_type(&list_data, CtType::ListStream));
    }

    #[test]
    fn test_eval_pipeline_trace_records_stage_details() {
        let expr = ctdsl::parse("const | echo").unwrap();
        let ctx = make_ctx().enable_trace();
        let _ = eval_pipeline(&expr, CtPipelineData::Empty, &ctx).unwrap();
        let trace = ctx.trace_snapshot().unwrap();
        assert_eq!(trace.stages.len(), 2);
        assert!(matches!(trace.stages[0].status, TraceStatus::Ok));
        assert!(matches!(trace.stages[1].status, TraceStatus::Ok));
        assert!(trace.stages[0].cmd.contains("const"));
        assert!(trace.stages[1].cmd.contains("echo"));
    }

    #[test]
    fn test_eval_pipeline_trace_error_status() {
        let expr = ctdsl::parse("__syskits_no_such_cmd_xyz__").unwrap();
        let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None).enable_trace();
        let _ = eval_pipeline(&expr, CtPipelineData::Empty, &ctx).unwrap_err();
        let trace = ctx.trace_snapshot().unwrap();
        assert_eq!(trace.stages.len(), 1);
        assert!(matches!(trace.stages[0].status, TraceStatus::Error(_)));
    }

    fn parse_single_call(src: &str) -> Call {
        let expr = ctdsl::parse(src).unwrap();
        expr.stages()[0].clone()
    }

    fn run_external_signature() -> DataSignature {
        DataSignature::new("run-external", "run external command").flag(CtFlag::with_value(
            "stdout-mode",
            None,
            "stdout decode mode",
            CtType::String,
        ))
    }

    #[test]
    fn test_build_data_call_run_external_preserves_dash_args() {
        let call = parse_single_call("run-external echo -n hi --stdout-mode text");
        let sig = run_external_signature();
        let data_call = build_data_call(&call, Some(&sig)).expect("build data call");

        let positionals: Vec<String> = data_call
            .positionals
            .iter()
            .map(|arg| arg.value.to_text())
            .collect();
        assert_eq!(positionals, vec!["echo", "-n", "hi"]);
        assert_eq!(
            data_call
                .get_flag::<String>("stdout-mode")
                .expect("flag extraction"),
            Some("text".to_string())
        );
    }

    #[test]
    fn test_build_data_call_unknown_gnu_short_args_preserve_argv_strings() {
        let call = parse_single_call("ls -1 -la");
        let sig = DataSignature::new("ls", "ls")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible args",
                CtType::Any,
            ))
            .allow_unknown_args(true);
        let data_call = build_data_call(&call, Some(&sig)).expect("build data call");

        let values: Vec<&CtValue> = data_call.positionals.iter().map(|arg| &arg.value).collect();
        assert_eq!(
            values,
            vec![
                &CtValue::String("-1".into()),
                &CtValue::String("-la".into())
            ]
        );

        let call = parse_single_call("head -5");
        let sig = DataSignature::new("head", "head")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible args",
                CtType::Any,
            ))
            .allow_unknown_args(true);
        let data_call = build_data_call(&call, Some(&sig)).expect("build data call");
        assert_eq!(
            data_call
                .positionals
                .iter()
                .map(|arg| &arg.value)
                .collect::<Vec<_>>(),
            vec![&CtValue::String("-5".into())]
        );
    }

    #[test]
    fn test_build_data_call_unknown_gnu_positionals_stringify_literals() {
        let call = parse_single_call("seq 1 3 true 2.5");
        let sig = DataSignature::new("seq", "seq")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible args",
                CtType::Any,
            ))
            .allow_unknown_args(true);
        let data_call = build_data_call(&call, Some(&sig)).expect("build data call");

        let values: Vec<&CtValue> = data_call.positionals.iter().map(|arg| &arg.value).collect();
        assert_eq!(
            values,
            vec![
                &CtValue::String("1".into()),
                &CtValue::String("3".into()),
                &CtValue::String("true".into()),
                &CtValue::String("2.5".into()),
            ]
        );
    }

    #[test]
    fn test_build_data_call_switch_flag_does_not_swallow_positional() {
        let call = parse_single_call("stat --dereference /etc/passwd");
        let sig = DataSignature::new("stat", "stat")
            .positional(CtPositionalArg::required(
                "path",
                "path to inspect",
                CtType::String,
            ))
            .flag(CtFlag::switch("dereference", Some('L'), "follow symlink"));
        let data_call = build_data_call(&call, Some(&sig)).expect("build data call");

        assert!(data_call.has_flag("dereference"));
        let path = data_call
            .req::<String>(0)
            .expect("path positional should exist");
        assert_eq!(path, "/etc/passwd");
    }

    #[test]
    fn test_build_data_call_value_flag_requires_value() {
        let call = parse_single_call("run-external echo --stdout-mode");
        let sig = run_external_signature();
        let err = build_data_call(&call, Some(&sig)).expect_err("missing value must fail");
        assert_eq!(err.code, crate::execution::exit_code::USAGE_ERROR);
        assert!(err.to_string().contains("requires a value"));
    }

    #[test]
    fn test_validate_data_call_rejects_unknown_flag() {
        let call = parse_single_call("from text abc --bogus");
        let sig = DataSignature::new("from", "from")
            .positional(CtPositionalArg::optional(
                "format",
                "format",
                CtType::String,
            ))
            .positional(CtPositionalArg::optional(
                "source",
                "source",
                CtType::String,
            ))
            .flag(CtFlag::switch("help", Some('h'), "help"));
        let data_call = build_data_call(&call, Some(&sig)).expect("build data call");
        let err =
            validate_data_call_against_signature(&call, &data_call, &sig).expect_err("must fail");
        assert_eq!(err.code, crate::execution::exit_code::USAGE_ERROR);
        assert!(err.to_string().contains("unknown flag"));
    }

    #[test]
    fn test_validate_data_call_rejects_unexpected_positional() {
        let call = parse_single_call("from text abc extra");
        let sig = DataSignature::new("from", "from")
            .positional(CtPositionalArg::optional(
                "format",
                "format",
                CtType::String,
            ))
            .positional(CtPositionalArg::optional(
                "source",
                "source",
                CtType::String,
            ));
        let data_call = build_data_call(&call, Some(&sig)).expect("build data call");
        let err =
            validate_data_call_against_signature(&call, &data_call, &sig).expect_err("must fail");
        assert_eq!(err.code, crate::execution::exit_code::USAGE_ERROR);
        assert!(err.to_string().contains("unexpected positional argument"));
    }

    #[test]
    fn test_validate_data_call_allows_rest_positionals() {
        let call = parse_single_call("run-external echo -n hi");
        let sig = DataSignature::new("run-external", "run external")
            .positional(CtPositionalArg::required("cmd", "command", CtType::String))
            .rest(CtPositionalArg::optional("args", "args", CtType::String));
        let data_call = build_data_call(&call, Some(&sig)).expect("build data call");
        validate_data_call_against_signature(&call, &data_call, &sig).expect("must pass");
    }

    struct BrokenReader;

    impl Read for BrokenReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("broken"))
        }
    }

    #[test]
    fn test_print_pipeline_data_propagates_bytestream_error() {
        let data = CtPipelineData::ByteStream(CtByteStream::new(
            BrokenReader,
            CtPipelineMetadata::default(),
        ));
        assert!(try_print_pipeline_data(data).is_err());
    }

    #[test]
    fn test_eval_merged_where_select_list_fused() {
        let where_call = parse_single_call("where n >= 3");
        let select_call = parse_single_call("select name");
        let input = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("a".into())),
                    ("n".into(), CtValue::Int(1)),
                ]),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("b".into())),
                    ("n".into(), CtValue::Int(3)),
                ]),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("c".into())),
                    ("n".into(), CtValue::Int(4)),
                ]),
            ]),
            CtPipelineMetadata::default(),
        );
        let out = eval_merged_where_select(&where_call, &select_call, input, &make_ctx()).unwrap();
        match out {
            CtPipelineData::Value(CtValue::List(rows), _) => {
                assert_eq!(rows.len(), 2);
                match &rows[0] {
                    CtValue::Record(cols) => {
                        assert_eq!(cols.len(), 1);
                        assert_eq!(cols[0].0, "name");
                        assert_eq!(format_ct_value(&cols[0].1), "b");
                    }
                    _ => panic!("expected record"),
                }
            }
            _ => panic!("expected list output"),
        }
    }

    #[test]
    fn test_eval_merged_where_select_record_no_match_matches_select_error() {
        let where_call = parse_single_call("where n > 1");
        let select_call = parse_single_call("select name");
        let input = CtPipelineData::Value(
            CtValue::Record(vec![
                ("name".into(), CtValue::String("a".into())),
                ("n".into(), CtValue::Int(1)),
            ]),
            CtPipelineMetadata::default(),
        );
        let err = eval_merged_where_select(&where_call, &select_call, input, &make_ctx())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("select: empty input"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_eval_merged_where_select_empty_input_matches_select_error() {
        let where_call = parse_single_call("where n > 1");
        let select_call = parse_single_call("select name");
        let err = eval_merged_where_select(
            &where_call,
            &select_call,
            CtPipelineData::Empty,
            &make_ctx(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("select: empty input"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_eval_merged_where_select_semantic_equivalence_smoke() {
        let where_call = parse_single_call("where n >= 2");
        let select_call = parse_single_call("select name");

        let cond = parse_where_condition_from_call(&where_call).unwrap();
        let cols = parse_select_columns_from_call(&select_call).unwrap();

        let sequential_ref = |input: CtPipelineData| -> Result<CtPipelineData, CtDiagnosticError> {
            let meta = CtPipelineMetadata::default();
            match input {
                CtPipelineData::Value(CtValue::Record(fields), _) => {
                    if record_matches(&fields, &cond) {
                        Ok(CtPipelineData::Value(
                            CtValue::Record(project_record(fields, &cols)),
                            meta,
                        ))
                    } else {
                        Err(CtDiagnosticError::simple("select: empty input"))
                    }
                }
                CtPipelineData::Value(CtValue::List(items), _) => {
                    let filtered: Vec<CtValue> = items
                        .into_iter()
                        .filter(|item| match item {
                            CtValue::Record(fields) => record_matches(fields, &cond),
                            _ => false,
                        })
                        .collect();
                    let projected: Vec<CtValue> = filtered
                        .into_iter()
                        .map(|item| match item {
                            CtValue::Record(fields) => {
                                CtValue::Record(project_record(fields, &cols))
                            }
                            other => other,
                        })
                        .collect();
                    Ok(CtPipelineData::Value(CtValue::List(projected), meta))
                }
                CtPipelineData::Empty => Err(CtDiagnosticError::simple("select: empty input")),
                _ => Err(CtDiagnosticError::simple(
                    "where: expected Record or List input",
                )),
            }
        };

        let make_record_input = |n: i64| {
            CtPipelineData::Value(
                CtValue::Record(vec![
                    ("name".into(), CtValue::String(format!("v{n}"))),
                    ("n".into(), CtValue::Int(n)),
                ]),
                CtPipelineMetadata::default(),
            )
        };

        for n in -32..=32 {
            let fused = eval_merged_where_select(
                &where_call,
                &select_call,
                make_record_input(n),
                &make_ctx(),
            )
            .map_err(|e| e.to_string());
            let sequential = sequential_ref(make_record_input(n)).map_err(|e| e.to_string());
            let fused_norm = fused
                .as_ref()
                .map(|v| format!("{v:?}"))
                .unwrap_or_else(|e| e.clone());
            let sequential_norm = sequential
                .as_ref()
                .map(|v| format!("{v:?}"))
                .unwrap_or_else(|e| e.clone());
            assert_eq!(fused_norm, sequential_norm, "mismatch for n={n}");
        }

        for len in 0..=20usize {
            let make_list_input = || {
                let mut rows = Vec::new();
                for i in 0..len {
                    rows.push(CtValue::Record(vec![
                        ("name".into(), CtValue::String(format!("r{i}"))),
                        ("n".into(), CtValue::Int((i as i64) - 8)),
                    ]));
                }
                CtPipelineData::Value(CtValue::List(rows), CtPipelineMetadata::default())
            };
            let fused =
                eval_merged_where_select(&where_call, &select_call, make_list_input(), &make_ctx())
                    .map_err(|e| e.to_string());
            let sequential = sequential_ref(make_list_input()).map_err(|e| e.to_string());
            let fused_norm = fused
                .as_ref()
                .map(|v| format!("{v:?}"))
                .unwrap_or_else(|e| e.clone());
            let sequential_norm = sequential
                .as_ref()
                .map(|v| format!("{v:?}"))
                .unwrap_or_else(|e| e.clone());
            assert_eq!(fused_norm, sequential_norm, "mismatch for list len={len}");
        }
    }

    #[test]
    #[ignore = "manual benchmark; run with: cargo test -p ctengine benchmark_merged_where_select -- --ignored --nocapture"]
    fn benchmark_merged_where_select() {
        use std::time::Instant;

        let where_call = parse_single_call("where n >= 512");
        let select_call = parse_single_call("select name");
        let cond = parse_where_condition_from_call(&where_call).unwrap();
        let cols = parse_select_columns_from_call(&select_call).unwrap();

        let make_input = || {
            let mut rows = Vec::with_capacity(20_000);
            for i in 0..20_000i64 {
                rows.push(CtValue::Record(vec![
                    ("name".into(), CtValue::String(format!("r{i}"))),
                    ("n".into(), CtValue::Int(i % 1024)),
                    ("v".into(), CtValue::Int(i)),
                ]));
            }
            CtPipelineData::Value(CtValue::List(rows), CtPipelineMetadata::default())
        };

        let sequential_ref = |input: CtPipelineData| -> Result<CtPipelineData, CtDiagnosticError> {
            let meta = CtPipelineMetadata::default();
            match input {
                CtPipelineData::Value(CtValue::List(items), _) => {
                    let filtered: Vec<CtValue> = items
                        .into_iter()
                        .filter(|item| match item {
                            CtValue::Record(fields) => record_matches(fields, &cond),
                            _ => false,
                        })
                        .collect();
                    let projected: Vec<CtValue> = filtered
                        .into_iter()
                        .map(|item| match item {
                            CtValue::Record(fields) => {
                                CtValue::Record(project_record(fields, &cols))
                            }
                            other => other,
                        })
                        .collect();
                    Ok(CtPipelineData::Value(CtValue::List(projected), meta))
                }
                CtPipelineData::Value(CtValue::Record(fields), _) => {
                    if record_matches(&fields, &cond) {
                        Ok(CtPipelineData::Value(
                            CtValue::Record(project_record(fields, &cols)),
                            meta,
                        ))
                    } else {
                        Err(CtDiagnosticError::simple("select: empty input"))
                    }
                }
                CtPipelineData::Empty => Err(CtDiagnosticError::simple("select: empty input")),
                _ => Err(CtDiagnosticError::simple(
                    "where: expected Record or List input",
                )),
            }
        };

        let rounds = 20usize;
        let mut merged_ms = 0u128;
        let mut sequential_ms = 0u128;

        for _ in 0..rounds {
            let t0 = Instant::now();
            let _ = eval_merged_where_select(&where_call, &select_call, make_input(), &make_ctx())
                .expect("merged eval failed");
            merged_ms += t0.elapsed().as_millis();

            let t1 = Instant::now();
            let _ = sequential_ref(make_input()).expect("sequential ref failed");
            sequential_ms += t1.elapsed().as_millis();
        }

        println!(
            "where+select benchmark rounds={rounds}: merged={merged_ms}ms sequential={sequential_ms}ms"
        );
    }
}
