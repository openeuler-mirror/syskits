use crate::error::CtDiagnosticError;
use crate::trace::{StageTrace, TraceStatus};
use ctpipeline::pipeline_data::CtPipelineData;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Default)]
pub enum OnFailure {
    #[default]
    Fail,
    Continue,
    Goto(String),
}

#[derive(Debug, Clone)]
pub struct WorkflowStage {
    pub name: String,
    pub expr: Option<String>,
    pub if_cond: Option<String>,
    pub else_expr: Option<String>,
    pub foreach: Option<String>,
    pub var: Option<String>,
    pub timeout_ms: Option<u64>,
    pub retry: Option<u32>,
    pub on_failure: OnFailure,
    pub checkpoint: bool,
}

#[derive(Debug, Clone)]
pub struct WorkflowScript {
    pub stages: Vec<WorkflowStage>,
}

#[derive(Debug)]
pub enum WorkflowError {
    ParseStage {
        stage: String,
        err: CtDiagnosticError,
    },
    RunStage {
        stage: String,
        err: CtDiagnosticError,
    },
    EmptyScript,
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkflowError::ParseStage { stage, err } => {
                write!(f, "workflow stage '{stage}': {err}")
            }
            WorkflowError::RunStage { stage, err } => {
                write!(f, "workflow stage '{stage}': {err}")
            }
            WorkflowError::EmptyScript => write!(f, "empty workflow script"),
        }
    }
}

impl std::error::Error for WorkflowError {}

impl From<WorkflowError> for CtDiagnosticError {
    fn from(err: WorkflowError) -> Self {
        CtDiagnosticError::simple(err.to_string())
    }
}

pub fn run_workflow(
    script: &WorkflowScript,
    input: CtPipelineData,
    ctx: &crate::context::DataEngineContext,
) -> Result<CtPipelineData, WorkflowError> {
    if script.stages.is_empty() {
        return Err(WorkflowError::EmptyScript);
    }

    // 每次 goto 或 stage 推进都消耗一次配额，防止配置错误导致无限循环。
    // 上限 = stage 数 * 10，给足 on_failure goto 重试空间同时拒绝死循环。
    let max_transitions = script.stages.len().saturating_mul(10).max(100);
    let mut transition_count: usize = 0;

    let mut current_data = input;
    let mut vars = crate::workflow_vars::WorkflowVars::new();
    let checkpoint_run_id = new_checkpoint_run_id();

    let mut stage_idx = 0;
    while stage_idx < script.stages.len() {
        transition_count += 1;
        if transition_count > max_transitions {
            return Err(WorkflowError::RunStage {
                stage: script.stages[stage_idx].name.clone(),
                err: CtDiagnosticError::simple(format!(
                    "workflow exceeded maximum stage transitions ({max_transitions}); \
                     possible infinite loop via on_failure: goto"
                )),
            });
        }

        let stage = &script.stages[stage_idx];
        let needs_input_recovery = !matches!(stage.on_failure, OnFailure::Fail);

        // 1. Determine if we need to clone current_data
        let requires_clone =
            stage.if_cond.is_some() || stage.foreach.is_some() || stage.retry.unwrap_or(0) > 0;
        if requires_clone {
            current_data = current_data.collect_values();
        }

        let max_attempts = stage.retry.unwrap_or(0) + 1;
        let mut attempt = 0;
        let mut final_result = CtPipelineData::Empty;
        let mut stage_err = None;
        let mut recovery_input = None;

        while attempt < max_attempts {
            attempt += 1;
            let started = Instant::now();

            let stage_input = if requires_clone {
                match clone_in_memory(&current_data) {
                    Ok(d) => d,
                    Err(e) => {
                        stage_err = Some(e);
                        break;
                    }
                }
            } else {
                let stage_input = std::mem::take(&mut current_data);
                if needs_input_recovery {
                    let (stage_input, backup) = split_pipeline_data_for_recovery(stage_input);
                    recovery_input = backup;
                    stage_input
                } else {
                    stage_input
                }
            };

            match execute_stage(stage, stage_input, &mut vars, ctx, &started) {
                Ok(mut res) => {
                    if let Some(timeout_ms) = stage.timeout_ms {
                        if timeout_exceeded(&started, timeout_ms) {
                            stage_err = Some(materialize_timeout_error(&started, timeout_ms));
                            continue;
                        }
                        match materialize_pipeline_data_with_timeout(res, &started, timeout_ms) {
                            Ok(materialized) => {
                                res = materialized;
                            }
                            Err(e) => {
                                stage_err = Some(e);
                                continue;
                            }
                        }
                    }

                    if stage.checkpoint {
                        res = res.collect_values();
                        if let Err(e) =
                            persist_checkpoint(stage, stage_idx, &res, &checkpoint_run_id)
                        {
                            stage_err = Some(e);
                            continue;
                        }
                    }

                    final_result = res;
                    stage_err = None;
                    break;
                }
                Err(e) => {
                    stage_err = Some(e);
                }
            }
        }

        if let Some(err) = stage_err {
            match &stage.on_failure {
                OnFailure::Fail => {
                    return Err(WorkflowError::RunStage {
                        stage: stage.name.clone(),
                        err,
                    });
                }
                OnFailure::Continue => {
                    record_recovered_stage_error(stage, "continue", &err, &current_data, ctx);
                    current_data = restore_data_for_control_flow(
                        stage,
                        &current_data,
                        &mut recovery_input,
                        requires_clone,
                        &err,
                    )?;
                    stage_idx += 1;
                    continue;
                }
                OnFailure::Goto(target) => {
                    if let Some(idx) = script.stages.iter().position(|s| &s.name == target) {
                        record_recovered_stage_error(stage, "goto", &err, &current_data, ctx);
                        current_data = restore_data_for_control_flow(
                            stage,
                            &current_data,
                            &mut recovery_input,
                            requires_clone,
                            &err,
                        )?;
                        stage_idx = idx;
                        continue;
                    } else {
                        return Err(WorkflowError::RunStage {
                            stage: stage.name.clone(),
                            err: CtDiagnosticError::simple(format!(
                                "goto target '{target}' not found"
                            )),
                        });
                    }
                }
            }
        } else {
            // Apply variable binding if any
            if let Some(var_name) = &stage.var {
                final_result = final_result.collect_values();
                if let CtPipelineData::Value(ref v, _) = final_result {
                    vars.set(var_name.clone(), v.clone());
                }
            }
            current_data = final_result;
            stage_idx += 1;
        }
    }

    Ok(current_data)
}

fn estimate_rows(data: &CtPipelineData) -> usize {
    match data {
        CtPipelineData::Empty => 0,
        CtPipelineData::Value(ctpipeline::CtValue::List(items), _) => items.len(),
        CtPipelineData::Value(ctpipeline::CtValue::Nothing, _) => 0,
        CtPipelineData::Value(_, _) => 1,
        CtPipelineData::ListStream(_) => 0,
        CtPipelineData::ByteStream(_) => 0,
    }
}

fn record_recovered_stage_error(
    stage: &WorkflowStage,
    policy: &str,
    err: &CtDiagnosticError,
    input: &CtPipelineData,
    ctx: &crate::context::DataEngineContext,
) {
    ctx.record_stage_trace(StageTrace {
        name: Some(stage.name.clone()),
        cmd: format!("workflow:{} (on_failure={policy})", stage.name),
        duration_ms: 0,
        rows_in: estimate_rows(input),
        rows_out: 0,
        status: TraceStatus::Error(format!("{err} (recovered by workflow policy)")),
    });
}

fn clone_in_memory(data: &CtPipelineData) -> Result<CtPipelineData, CtDiagnosticError> {
    match data {
        CtPipelineData::Empty => Ok(CtPipelineData::Empty),
        CtPipelineData::Value(v, m) => Ok(CtPipelineData::Value(v.clone(), m.clone())),
        _ => Err(CtDiagnosticError::simple(
            "Cannot clone streaming data for workflow control flow (use collect or parse first)",
        )),
    }
}

fn restore_data_for_control_flow(
    stage: &WorkflowStage,
    current_data: &CtPipelineData,
    recovery_input: &mut Option<CtPipelineData>,
    requires_clone: bool,
    stage_err: &CtDiagnosticError,
) -> Result<CtPipelineData, WorkflowError> {
    if requires_clone {
        return clone_in_memory(current_data).map_err(|e| WorkflowError::RunStage {
            stage: stage.name.clone(),
            err: CtDiagnosticError::simple(format!(
                "failed to preserve input for on_failure handling: {e}"
            )),
        });
    }
    recovery_input
        .take()
        .ok_or_else(|| WorkflowError::RunStage {
            stage: stage.name.clone(),
            err: CtDiagnosticError::simple(format!(
                "failed to preserve input for on_failure handling after stage error ({stage_err}); \
                 streaming input cannot be replayed without buffering, add `collect` before this stage"
            )),
        })
}

fn split_pipeline_data_for_recovery(
    data: CtPipelineData,
) -> (CtPipelineData, Option<CtPipelineData>) {
    match data {
        CtPipelineData::Empty => (CtPipelineData::Empty, Some(CtPipelineData::Empty)),
        CtPipelineData::Value(v, m) => (
            CtPipelineData::Value(v.clone(), m.clone()),
            Some(CtPipelineData::Value(v, m)),
        ),
        CtPipelineData::ListStream(stream) => (CtPipelineData::ListStream(stream), None),
        CtPipelineData::ByteStream(stream) => (CtPipelineData::ByteStream(stream), None),
    }
}

fn materialize_timeout_error(started: &Instant, timeout_ms: u64) -> CtDiagnosticError {
    CtDiagnosticError::simple(format!(
        "stage timed out after {} ms (elapsed {} ms)",
        timeout_ms,
        started.elapsed().as_millis()
    ))
    .with_code(124)
}

fn timeout_exceeded(started: &Instant, timeout_ms: u64) -> bool {
    started.elapsed() > Duration::from_millis(timeout_ms)
}

fn materialize_pipeline_data_with_timeout(
    data: CtPipelineData,
    started: &Instant,
    timeout_ms: u64,
) -> Result<CtPipelineData, CtDiagnosticError> {
    match data {
        CtPipelineData::ListStream(stream) => {
            let meta = stream.metadata.clone();
            let mut values = Vec::new();
            for value in stream {
                if timeout_exceeded(started, timeout_ms) {
                    return Err(materialize_timeout_error(started, timeout_ms));
                }
                values.push(value);
            }
            if timeout_exceeded(started, timeout_ms) {
                return Err(materialize_timeout_error(started, timeout_ms));
            }
            Ok(CtPipelineData::ListStream(ctpipeline::CtListStream::new(
                values.into_iter(),
                meta,
            )))
        }
        CtPipelineData::ByteStream(stream) => {
            let meta = stream.metadata.clone();
            let buf = read_bytestream_with_timeout(stream, started, timeout_ms)?;
            if timeout_exceeded(started, timeout_ms) {
                return Err(materialize_timeout_error(started, timeout_ms));
            }
            Ok(CtPipelineData::ByteStream(ctpipeline::CtByteStream::new(
                std::io::Cursor::new(buf),
                meta,
            )))
        }
        other => Ok(other),
    }
}

enum ByteStreamReadOutcome {
    Chunk(Vec<u8>),
    Eof,
    ReadError(std::io::ErrorKind, String),
}

fn remaining_timeout(started: &Instant, timeout_ms: u64) -> Option<Duration> {
    Duration::from_millis(timeout_ms).checked_sub(started.elapsed())
}

fn remaining_timeout_ms(started: &Instant, timeout_ms: u64) -> Option<u64> {
    let remaining = remaining_timeout(started, timeout_ms)?;
    if remaining.is_zero() {
        return None;
    }
    Some(remaining.as_millis().max(1).min(u64::MAX as u128) as u64)
}

fn read_bytestream_with_timeout(
    mut stream: ctpipeline::CtByteStream,
    started: &Instant,
    timeout_ms: u64,
) -> Result<Vec<u8>, CtDiagnosticError> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        use std::io::Read;
        let mut chunk = [0_u8; 8192];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => {
                    let _ = tx.send(ByteStreamReadOutcome::Eof);
                    break;
                }
                Ok(n) => {
                    if tx
                        .send(ByteStreamReadOutcome::Chunk(chunk[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(ByteStreamReadOutcome::ReadError(e.kind(), e.to_string()));
                    break;
                }
            }
        }
    });

    let mut buf = Vec::new();
    loop {
        let remaining = remaining_timeout(started, timeout_ms)
            .ok_or_else(|| materialize_timeout_error(started, timeout_ms))?;
        match rx.recv_timeout(remaining) {
            Ok(ByteStreamReadOutcome::Chunk(chunk)) => buf.extend_from_slice(&chunk),
            Ok(ByteStreamReadOutcome::Eof) => return Ok(buf),
            Ok(ByteStreamReadOutcome::ReadError(std::io::ErrorKind::TimedOut, _)) => {
                return Err(materialize_timeout_error(started, timeout_ms));
            }
            Ok(ByteStreamReadOutcome::ReadError(_, message)) => {
                return Err(CtDiagnosticError::simple(format!(
                    "read stage output failed: {message}"
                )));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err(materialize_timeout_error(started, timeout_ms));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(CtDiagnosticError::simple(
                    "read stage output failed: reader thread disconnected",
                ));
            }
        }
    }
}

fn persist_checkpoint(
    stage: &WorkflowStage,
    stage_idx: usize,
    data: &CtPipelineData,
    run_id: &str,
) -> Result<(), CtDiagnosticError> {
    let collected = match data {
        CtPipelineData::Empty => ctpipeline::CtValue::Nothing,
        CtPipelineData::Value(v, _) => v.clone(),
        _ => {
            return Err(CtDiagnosticError::simple(
                "checkpoint only supports in-memory pipeline data",
            ));
        }
    };

    let payload = serde_json::json!({
        "stage": stage.name,
        "stage_index": stage_idx,
        "data": serde_json::to_value(collected)
            .map_err(|e| CtDiagnosticError::simple(format!("checkpoint serialize failed: {e}")))?,
    });
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|e| CtDiagnosticError::simple(format!("checkpoint encode failed: {e}")))?;
    let path = checkpoint_file_path(&stage.name, stage_idx, run_id);

    if let Some(parent) = path.parent() {
        ensure_checkpoint_dir(parent)?;
    }
    write_checkpoint_file(&path, &bytes).map_err(|e| {
        CtDiagnosticError::simple(format!(
            "failed to write checkpoint '{}': {}",
            path.display(),
            e
        ))
    })?;
    Ok(())
}

fn new_checkpoint_run_id() -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut random = [0u8; 8];
    let mut rng = rand::rngs::OsRng;
    rand::RngCore::fill_bytes(&mut rng, &mut random);
    format!(
        "{}-{}-{:016x}",
        std::process::id(),
        timestamp,
        u64::from_ne_bytes(random)
    )
}

fn checkpoint_file_path(stage_name: &str, stage_idx: usize, run_id: &str) -> std::path::PathBuf {
    let dir = checkpoint_base_dir();
    dir.join(format!(
        "syskits-checkpoint-{}-{}-{}.json",
        run_id,
        stage_idx,
        sanitize_stage_name(stage_name)
    ))
}

fn checkpoint_base_dir() -> std::path::PathBuf {
    std::env::var("SYSKITS_WORKFLOW_CHECKPOINT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| default_checkpoint_base_dir())
}

fn default_checkpoint_base_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("syskits-workflow-checkpoints")
}

fn ensure_checkpoint_dir(dir: &std::path::Path) -> Result<(), CtDiagnosticError> {
    if dir == default_checkpoint_base_dir() {
        ensure_private_checkpoint_dir(dir)
    } else {
        std::fs::create_dir_all(dir).map_err(|e| {
            CtDiagnosticError::simple(format!(
                "failed to create checkpoint dir '{}': {}",
                dir.display(),
                e
            ))
        })
    }
}

fn ensure_private_checkpoint_dir(dir: &std::path::Path) -> Result<(), CtDiagnosticError> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(CtDiagnosticError::simple(format!(
                    "checkpoint dir '{}' must be a real directory",
                    dir.display()
                )));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(dir).map_err(|e| {
                CtDiagnosticError::simple(format!(
                    "failed to create checkpoint dir '{}': {}",
                    dir.display(),
                    e
                ))
            })?;
        }
        Err(e) => {
            return Err(CtDiagnosticError::simple(format!(
                "failed to inspect checkpoint dir '{}': {}",
                dir.display(),
                e
            )));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| {
            CtDiagnosticError::simple(format!(
                "failed to secure checkpoint dir '{}': {}",
                dir.display(),
                e
            ))
        })?;
    }

    Ok(())
}

fn write_checkpoint_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.flush()
}

fn sanitize_stage_name(stage_name: &str) -> String {
    let mut out = String::with_capacity(stage_name.len());
    for ch in stage_name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "stage".to_string()
    } else {
        out
    }
}

fn execute_stage(
    stage: &WorkflowStage,
    input: CtPipelineData,
    vars: &mut crate::workflow_vars::WorkflowVars,
    ctx: &crate::context::DataEngineContext,
    started: &Instant,
) -> Result<CtPipelineData, CtDiagnosticError> {
    let mut actual_input = input;

    // 1. evaluate if_cond
    if let Some(cond_expr) = &stage.if_cond {
        let cond_str = vars.expand_in_expr(cond_expr);
        let ast = parse_stage_expr(&cond_str, started, stage.timeout_ms)?;

        // We must clone input to pass to the condition check
        let cond_input = clone_in_memory(&actual_input)?;
        let cond_res = crate::interpreter::eval_pipeline(&ast, cond_input, ctx)?;

        // Evaluate truthiness: explicitly check for Bool, then fallback to non-empty
        let is_true = match cond_res {
            CtPipelineData::Value(ctpipeline::value::CtValue::Bool(b), _) => b,
            res => !res.is_empty(),
        };

        if is_true {
            // proceed to execute main expr
        } else {
            // execute else_expr if present, otherwise return empty
            if let Some(else_expr) = &stage.else_expr {
                let else_str = vars.expand_in_expr(else_expr);
                let ast = parse_stage_expr(&else_str, started, stage.timeout_ms)?;
                return crate::interpreter::eval_pipeline(&ast, actual_input, ctx);
            } else {
                return Ok(CtPipelineData::Empty);
            }
        }
    }

    // 2. foreach
    if let Some(foreach_var) = &stage.foreach {
        // Implement loop iteration conceptually
        // For simplicity in L2, if foreach is present, we expect input to be a list
        actual_input = actual_input.collect_values();
        if let CtPipelineData::Value(ctpipeline::value::CtValue::List(items), meta) = actual_input {
            let previous_value = vars.get(foreach_var).cloned();
            let result = (|| {
                let mut results = Vec::new();
                for item in items {
                    vars.set(foreach_var.clone(), item.clone());
                    if let Some(expr) = &stage.expr {
                        let expr_str = vars.expand_in_expr(expr);
                        let ast = parse_stage_expr(&expr_str, started, stage.timeout_ms)?;
                        let res = crate::interpreter::eval_pipeline(
                            &ast,
                            CtPipelineData::Value(item, meta.clone()),
                            ctx,
                        )?;
                        if let CtPipelineData::Value(v, _) = res.collect_values() {
                            results.push(v);
                        }
                    }
                }
                Ok(CtPipelineData::Value(
                    ctpipeline::value::CtValue::List(results),
                    meta,
                ))
            })();
            restore_workflow_var(vars, foreach_var, previous_value);
            return result;
        } else {
            return Err(CtDiagnosticError::simple("foreach requires a List input"));
        }
    }

    // 3. Normal execution
    if let Some(expr) = &stage.expr {
        let expr_str = vars.expand_in_expr(expr);
        let ast = parse_stage_expr(&expr_str, started, stage.timeout_ms)?;
        crate::interpreter::eval_pipeline(&ast, actual_input, ctx)
    } else {
        Ok(actual_input)
    }
}

fn restore_workflow_var(
    vars: &mut crate::workflow_vars::WorkflowVars,
    name: &str,
    previous_value: Option<ctpipeline::value::CtValue>,
) {
    if let Some(value) = previous_value {
        vars.set(name.to_string(), value);
    } else {
        vars.remove(name);
    }
}

fn parse_stage_expr(
    expr: &str,
    started: &Instant,
    timeout_ms: Option<u64>,
) -> Result<ctdsl::Expr, CtDiagnosticError> {
    let mut ast = ctdsl::parse(expr).map_err(|e| CtDiagnosticError::simple(e.to_string()))?;
    if let Some(timeout_ms) = timeout_ms {
        let remaining = remaining_timeout_ms(started, timeout_ms)
            .ok_or_else(|| materialize_timeout_error(started, timeout_ms))?;
        apply_timeout_to_run_external(&mut ast, remaining);
    }
    Ok(ast)
}

fn apply_timeout_to_run_external(ast: &mut ctdsl::Expr, timeout_ms: u64) {
    let ctdsl::Expr::Pipeline(calls) = ast;
    for call in calls {
        if call.name != "run-external" || call_has_timeout_ms(call) {
            continue;
        }
        call.args.push(ctdsl::Arg::LongFlagValue {
            name: "timeout-ms".to_string(),
            value: ctdsl::Lit::Int(timeout_ms.min(i64::MAX as u64) as i64),
            value_span: call.span.clone(),
            span: call.span.clone(),
        });
    }
}

fn call_has_timeout_ms(call: &ctdsl::Call) -> bool {
    call.args.iter().any(|arg| match arg {
        ctdsl::Arg::LongFlag { name, .. } | ctdsl::Arg::LongFlagValue { name, .. } => {
            name == "timeout-ms"
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::DataCommand;
    use crate::context::CommandRegistry;
    use ctpipeline::CtType;
    use ctpipeline::metadata::CtPipelineMetadata;
    use ctpipeline::value::CtValue;
    use ctsig::{CtPositionalArg, DataCall, DataSignature};

    fn get_test_ctx() -> crate::context::DataEngineContext {
        crate::context::DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    struct WorkflowListInts;
    impl DataCommand for WorkflowListInts {
        fn signature(&self) -> DataSignature {
            DataSignature::new("wf-list-ints", "list ints")
        }

        fn run(
            &self,
            _call: &DataCall,
            _input: CtPipelineData,
            _ctx: &crate::context::DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            Ok(CtPipelineData::Value(
                CtValue::List(vec![CtValue::Int(1), CtValue::Int(2)]),
                CtPipelineMetadata::default(),
            ))
        }
    }

    struct WorkflowEmitInt;
    impl DataCommand for WorkflowEmitInt {
        fn signature(&self) -> DataSignature {
            DataSignature::new("wf-emit-int", "emit int").positional(CtPositionalArg::required(
                "value",
                "value",
                CtType::Int,
            ))
        }

        fn run(
            &self,
            call: &DataCall,
            _input: CtPipelineData,
            _ctx: &crate::context::DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            let value = call.positionals[0]
                .value
                .as_int()
                .map_err(|e| CtDiagnosticError::simple(e.to_string()))?;
            Ok(CtPipelineData::Value(
                CtValue::Int(value),
                CtPipelineMetadata::default(),
            ))
        }
    }

    struct WorkflowConstNinetyNine;
    impl DataCommand for WorkflowConstNinetyNine {
        fn signature(&self) -> DataSignature {
            DataSignature::new("wf-const-99", "const 99")
        }

        fn run(
            &self,
            _call: &DataCall,
            _input: CtPipelineData,
            _ctx: &crate::context::DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            Ok(CtPipelineData::Value(
                CtValue::Int(99),
                CtPipelineMetadata::default(),
            ))
        }
    }

    struct WorkflowAppendInt;
    impl DataCommand for WorkflowAppendInt {
        fn signature(&self) -> DataSignature {
            DataSignature::new("wf-append-int", "append int").positional(CtPositionalArg::required(
                "value",
                "value",
                CtType::Int,
            ))
        }

        fn run(
            &self,
            call: &DataCall,
            input: CtPipelineData,
            _ctx: &crate::context::DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            let value = call.positionals[0]
                .value
                .as_int()
                .map_err(|e| CtDiagnosticError::simple(e.to_string()))?;
            match input {
                CtPipelineData::Value(CtValue::List(mut items), meta) => {
                    items.push(CtValue::Int(value));
                    Ok(CtPipelineData::Value(CtValue::List(items), meta))
                }
                _ => Err(CtDiagnosticError::simple("expected list input")),
            }
        }
    }

    struct WorkflowEchoString;
    impl DataCommand for WorkflowEchoString {
        fn signature(&self) -> DataSignature {
            DataSignature::new("wf-echo-string", "echo string")
                .positional(CtPositionalArg::required("value", "value", CtType::String))
        }

        fn run(
            &self,
            call: &DataCall,
            _input: CtPipelineData,
            _ctx: &crate::context::DataEngineContext,
        ) -> Result<CtPipelineData, CtDiagnosticError> {
            let value = call.positionals[0]
                .value
                .as_str()
                .map_err(|e| CtDiagnosticError::simple(e.to_string()))?;
            Ok(CtPipelineData::Value(
                CtValue::String(value.to_string()),
                CtPipelineMetadata::default(),
            ))
        }
    }

    fn workflow_var_test_ctx() -> crate::context::DataEngineContext {
        let registry = CommandRegistry::from_factories(&[
            ("wf-list-ints", || Box::new(WorkflowListInts)),
            ("wf-emit-int", || Box::new(WorkflowEmitInt)),
            ("wf-const-99", || Box::new(WorkflowConstNinetyNine)),
            ("wf-append-int", || Box::new(WorkflowAppendInt)),
            ("wf-echo-string", || Box::new(WorkflowEchoString)),
        ]);
        crate::context::DataEngineContext::new(registry, None, None)
    }

    fn unique_stage_name(prefix: &str) -> String {
        format!(
            "{}_{}_{}",
            prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    }

    fn matching_checkpoint_paths(stage_name: &str) -> Vec<std::path::PathBuf> {
        let suffix = format!("-{}.json", sanitize_stage_name(stage_name));
        let Ok(entries) = std::fs::read_dir(checkpoint_base_dir()) else {
            return Vec::new();
        };
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with("syskits-checkpoint-") && name.ends_with(&suffix)
                    })
            })
            .collect();
        paths.sort();
        paths
    }

    fn remove_matching_checkpoints(stage_name: &str) {
        for path in matching_checkpoint_paths(stage_name) {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn checkpoint_default_path_uses_private_temp_subdir() {
        let path = default_checkpoint_base_dir().join("probe.json");
        assert_ne!(path.parent(), Some(std::path::Path::new("/tmp")));
        assert!(
            path.starts_with(std::env::temp_dir().join("syskits-workflow-checkpoints")),
            "checkpoint path should use a private temp subdir by default: {}",
            path.display()
        );
    }

    #[cfg(unix)]
    #[test]
    fn persist_checkpoint_does_not_follow_existing_symlink() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!(
            "syskits-checkpoint-symlink-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("checkpoint.json");
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, "safe").unwrap();
        symlink(&victim, &path).unwrap();

        let result = write_checkpoint_file(&path, b"checkpoint");
        assert!(result.is_err(), "checkpoint write should reject symlinks");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "safe");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_workflow_empty_script_errors() {
        let script = WorkflowScript { stages: vec![] };
        let ctx = get_test_ctx();
        let result = run_workflow(&script, CtPipelineData::Empty, &ctx);
        assert!(matches!(result, Err(WorkflowError::EmptyScript)));
    }

    #[test]
    fn test_workflow_parse_error_stage() {
        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "bad-stage".into(),
                expr: Some("from json 'missing quote".into()),
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: None,
                retry: None,
                on_failure: Default::default(),
                checkpoint: false,
            }],
        };
        let ctx = get_test_ctx();
        let result = run_workflow(&script, CtPipelineData::Empty, &ctx);
        match result {
            Err(WorkflowError::RunStage { stage, .. }) => {
                assert_eq!(stage, "bad-stage");
            }
            _ => panic!("Expected RunStage error"),
        }
    }

    #[test]
    fn test_workflow_run_error_stage() {
        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "bad-run".into(),
                expr: Some("non_existent_command".into()),
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: None,
                retry: None,
                on_failure: Default::default(),
                checkpoint: false,
            }],
        };
        let ctx = get_test_ctx();
        let input = CtPipelineData::Empty;

        let result = run_workflow(&script, input, &ctx);
        match result {
            Err(WorkflowError::RunStage { stage, .. }) => {
                assert_eq!(stage, "bad-run");
            }
            _ => panic!("Expected RunStage error"),
        }
    }

    #[test]
    fn test_workflow_success_chaining() {
        use crate::command::DataCommand;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctpipeline::value::CtValue;
        use ctsig::DataSignature;

        struct DummyCmd;
        impl DataCommand for DummyCmd {
            fn signature(&self) -> DataSignature {
                DataSignature::new("dummy", "dummy desc")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _input: CtPipelineData,
                _ctx: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::Value(
                    CtValue::Int(42),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        struct DummyCmd2;
        impl DataCommand for DummyCmd2 {
            fn signature(&self) -> DataSignature {
                DataSignature::new("dummy2", "dummy2 desc")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                input: CtPipelineData,
                _ctx: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                if let CtPipelineData::Value(CtValue::Int(val), _) = input {
                    Ok(CtPipelineData::Value(
                        CtValue::Int(val + 1),
                        CtPipelineMetadata::default(),
                    ))
                } else {
                    Err(CtDiagnosticError::simple("expected int 42"))
                }
            }
        }

        fn dummy_factory() -> Box<dyn DataCommand> {
            Box::new(DummyCmd)
        }
        fn dummy2_factory() -> Box<dyn DataCommand> {
            Box::new(DummyCmd2)
        }

        let reg = CommandRegistry::from_factories(&[
            ("dummy", dummy_factory),
            ("dummy2", dummy2_factory),
        ]);

        let ctx = crate::context::DataEngineContext::new(reg, None, None);

        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "stage1".into(),
                    expr: Some("dummy".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "stage2".into(),
                    expr: Some("dummy2".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
            ],
        };

        let result = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        if let CtPipelineData::Value(CtValue::Int(val), _) = result {
            assert_eq!(val, 43);
        } else {
            panic!("Expected Int(43)");
        }
    }

    #[test]
    fn test_workflow_l2_if_else() {
        use crate::command::DataCommand;
        use ctpipeline::CtType;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctpipeline::value::CtValue;
        use ctsig::{CtPositionalArg, DataSignature};

        struct DummyCond;
        impl DataCommand for DummyCond {
            fn signature(&self) -> DataSignature {
                DataSignature::new("cond", "cond").positional(CtPositionalArg::required(
                    "value",
                    "condition selector",
                    CtType::String,
                ))
            }
            fn run(
                &self,
                call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                let arg = match &call.positionals[0].value {
                    ctpipeline::value::CtValue::String(s) => s.clone(),
                    _ => String::new(),
                };
                if arg == "true_val" {
                    Ok(CtPipelineData::Value(
                        CtValue::Int(1),
                        CtPipelineMetadata::default(),
                    ))
                } else {
                    Ok(CtPipelineData::Empty)
                }
            }
        }

        struct DummyAction;
        impl DataCommand for DummyAction {
            fn signature(&self) -> DataSignature {
                DataSignature::new("act", "act").positional(CtPositionalArg::required(
                    "value",
                    "action value",
                    CtType::String,
                ))
            }
            fn run(
                &self,
                call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                let v = match &call.positionals[0].value {
                    ctpipeline::value::CtValue::String(s) => s.clone(),
                    _ => String::new(),
                };
                Ok(CtPipelineData::Value(
                    CtValue::String(v),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        let reg = CommandRegistry::from_factories(&[
            ("cond", || Box::new(DummyCond)),
            ("act", || Box::new(DummyAction)),
        ]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);

        // true branch
        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "stage1".into(),
                expr: Some("act true_branch".into()),
                if_cond: Some("cond true_val".into()),
                else_expr: Some("act else_branch".into()),
                foreach: None,
                var: None,
                timeout_ms: None,
                retry: None,
                on_failure: Default::default(),
                checkpoint: false,
            }],
        };

        let res = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        if let CtPipelineData::Value(CtValue::String(s), _) = res {
            assert_eq!(s, "true_branch");
        } else {
            panic!("Expected true_branch");
        }

        // false branch
        let script_false = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "stage1".into(),
                expr: Some("act true_branch".into()),
                if_cond: Some("cond false_val".into()),
                else_expr: Some("act else_branch".into()),
                foreach: None,
                var: None,
                timeout_ms: None,
                retry: None,
                on_failure: Default::default(),
                checkpoint: false,
            }],
        };
        let res_false = run_workflow(&script_false, CtPipelineData::Empty, &ctx).unwrap();
        if let CtPipelineData::Value(CtValue::String(s), _) = res_false {
            assert_eq!(s, "else_branch");
        } else {
            panic!("Expected else_branch");
        }
    }

    #[test]
    fn test_workflow_l2_var_and_foreach() {
        use crate::command::DataCommand;
        use ctpipeline::CtType;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctpipeline::value::CtValue;
        use ctsig::{CtPositionalArg, DataSignature};

        struct SetFactor;
        impl DataCommand for SetFactor {
            fn signature(&self) -> DataSignature {
                DataSignature::new("factor", "factor")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::Value(
                    CtValue::Int(2),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        struct DummyList;
        impl DataCommand for DummyList {
            fn signature(&self) -> DataSignature {
                DataSignature::new("list", "list")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::Value(
                    CtValue::List(vec![CtValue::Int(1), CtValue::Int(2), CtValue::Int(3)]),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        struct DummyMult;
        impl DataCommand for DummyMult {
            fn signature(&self) -> DataSignature {
                DataSignature::new("mult", "mult").positional(CtPositionalArg::required(
                    "factor",
                    "multiplication factor",
                    CtType::Int,
                ))
            }
            fn run(
                &self,
                call: &ctsig::DataCall,
                i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                let factor = if let CtValue::Int(n) = call.positionals[0].value {
                    n
                } else {
                    return Err(CtDiagnosticError::simple("factor should be int"));
                };
                match i {
                    CtPipelineData::Value(CtValue::Int(val), m) => {
                        Ok(CtPipelineData::Value(CtValue::Int(val * factor), m))
                    }
                    _ => Err(CtDiagnosticError::simple("input should be int")),
                }
            }
        }

        let reg = CommandRegistry::from_factories(&[
            ("factor", || Box::new(SetFactor)),
            ("list", || Box::new(DummyList)),
            ("mult", || Box::new(DummyMult)),
        ]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);

        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "stage_factor".into(),
                    expr: Some("factor".into()),
                    var: Some("factor".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "stage_list".into(),
                    expr: Some("list".into()),
                    var: None,
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "stage_foreach".into(),
                    expr: Some("mult $factor".into()),
                    var: None,
                    if_cond: None,
                    else_expr: None,
                    foreach: Some("item".into()),
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
            ],
        };
        let res = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        match res.collect_values() {
            CtPipelineData::Value(CtValue::List(items), _) => {
                assert_eq!(items.len(), 3);
                assert!(matches!(items[0], CtValue::Int(2)));
                assert!(matches!(items[1], CtValue::Int(4)));
                assert!(matches!(items[2], CtValue::Int(6)));
            }
            _ => panic!("Expected list output"),
        }
    }

    #[test]
    fn test_workflow_foreach_expands_current_item_variable() {
        let ctx = workflow_var_test_ctx();
        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "list".into(),
                    expr: Some("wf-list-ints".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "foreach".into(),
                    expr: Some("wf-emit-int $item".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: Some("item".into()),
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
            ],
        };

        let res = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        assert!(matches!(
            res.collect_values(),
            CtPipelineData::Value(CtValue::List(items), _)
                if items == vec![CtValue::Int(1), CtValue::Int(2)]
        ));
    }

    #[test]
    fn test_workflow_foreach_restores_existing_variable() {
        let ctx = workflow_var_test_ctx();
        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "seed".into(),
                    expr: Some("wf-const-99".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: Some("item".into()),
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "list".into(),
                    expr: Some("wf-list-ints".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "foreach".into(),
                    expr: Some("wf-emit-int $item".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: Some("item".into()),
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "append-restored".into(),
                    expr: Some("wf-append-int $item".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
            ],
        };

        let res = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        assert!(matches!(
            res.collect_values(),
            CtPipelineData::Value(CtValue::List(items), _)
                if items == vec![CtValue::Int(1), CtValue::Int(2), CtValue::Int(99)]
        ));
    }

    #[test]
    fn test_workflow_foreach_removes_new_variable_after_loop() {
        let ctx = workflow_var_test_ctx();
        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "list".into(),
                    expr: Some("wf-list-ints".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "foreach".into(),
                    expr: Some("wf-emit-int $item".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: Some("item".into()),
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "after".into(),
                    expr: Some("wf-echo-string $item".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: Default::default(),
                    checkpoint: false,
                },
            ],
        };

        let res = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        assert!(matches!(
            res,
            CtPipelineData::Value(CtValue::String(value), _) if value == "$item"
        ));
    }

    #[test]
    fn test_workflow_l3_retry_and_goto() {
        use crate::command::DataCommand;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctpipeline::value::CtValue;
        use ctsig::DataSignature;
        use std::sync::atomic::Ordering;

        static ATTEMPT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

        struct DummyFail;
        impl DataCommand for DummyFail {
            fn signature(&self) -> DataSignature {
                DataSignature::new("fail", "fail")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                let current = ATTEMPT.fetch_add(1, Ordering::SeqCst);
                if current < 2 {
                    Err(CtDiagnosticError::simple("Simulated failure"))
                } else {
                    Ok(CtPipelineData::Value(
                        CtValue::Int(42),
                        CtPipelineMetadata::default(),
                    ))
                }
            }
        }

        fn fail_factory() -> Box<dyn DataCommand> {
            Box::new(DummyFail)
        }

        struct DummyPass;
        impl DataCommand for DummyPass {
            fn signature(&self) -> DataSignature {
                DataSignature::new("pass", "pass")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::Value(
                    CtValue::String("passed".into()),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        let reg = CommandRegistry::from_factories(&[
            ("fail", fail_factory),
            ("pass", || Box::new(DummyPass)),
        ]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);

        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "stage_fail".into(),
                    expr: Some("fail".into()),
                    retry: Some(3),
                    on_failure: crate::workflow::OnFailure::Fail,
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "stage_skip".into(),
                    expr: Some("fail".into()), // will fail immediately, but we continue
                    retry: None,
                    on_failure: crate::workflow::OnFailure::Continue,
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "stage_goto_trigger".into(),
                    expr: Some("fail".into()), // will fail immediately, triggers goto
                    retry: None,
                    on_failure: crate::workflow::OnFailure::Goto("stage_end".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "stage_unreachable".into(),
                    expr: Some("pass".into()),
                    retry: None,
                    on_failure: crate::workflow::OnFailure::Fail,
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "stage_end".into(),
                    expr: Some("pass".into()),
                    retry: None,
                    on_failure: crate::workflow::OnFailure::Fail,
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    checkpoint: false,
                },
            ],
        };

        ATTEMPT.store(0, Ordering::SeqCst);
        let res = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        if let CtPipelineData::Value(CtValue::String(s), _) = res {
            assert_eq!(s, "passed");
        } else {
            panic!("Expected 'passed'");
        }
    }

    #[test]
    fn test_workflow_l3_timeout() {
        use crate::command::DataCommand;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctpipeline::value::CtValue;
        use ctsig::DataSignature;

        struct SlowCmd;
        impl DataCommand for SlowCmd {
            fn signature(&self) -> DataSignature {
                DataSignature::new("slow", "slow")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                std::thread::sleep(Duration::from_millis(30));
                Ok(CtPipelineData::Value(
                    CtValue::Int(1),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        let reg = CommandRegistry::from_factories(&[("slow", || Box::new(SlowCmd))]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);
        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "timeout_stage".into(),
                expr: Some("slow".into()),
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: Some(5),
                retry: None,
                on_failure: OnFailure::Fail,
                checkpoint: false,
            }],
        };

        let err = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap_err();
        match err {
            WorkflowError::RunStage { err, .. } => {
                assert!(err.to_string().contains("timed out"));
                assert_eq!(err.code, 124);
            }
            _ => panic!("Expected RunStage timeout"),
        }
    }

    #[test]
    fn test_workflow_on_failure_continue_preserves_input() {
        use crate::command::DataCommand;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctpipeline::value::CtValue;
        use ctsig::DataSignature;

        struct Seed;
        impl DataCommand for Seed {
            fn signature(&self) -> DataSignature {
                DataSignature::new("seed", "seed")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::Value(
                    CtValue::Int(7),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        struct AlwaysFail;
        impl DataCommand for AlwaysFail {
            fn signature(&self) -> DataSignature {
                DataSignature::new("boom", "boom")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Err(CtDiagnosticError::simple("boom"))
            }
        }

        struct AddOne;
        impl DataCommand for AddOne {
            fn signature(&self) -> DataSignature {
                DataSignature::new("add1", "add1")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                match i {
                    CtPipelineData::Value(CtValue::Int(n), m) => {
                        Ok(CtPipelineData::Value(CtValue::Int(n + 1), m))
                    }
                    _ => Err(CtDiagnosticError::simple("expected int input")),
                }
            }
        }

        let reg = CommandRegistry::from_factories(&[
            ("seed", || Box::new(Seed)),
            ("boom", || Box::new(AlwaysFail)),
            ("add1", || Box::new(AddOne)),
        ]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);
        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "seed".into(),
                    expr: Some("seed".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: OnFailure::Fail,
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "boom".into(),
                    expr: Some("boom".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: OnFailure::Continue,
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "add".into(),
                    expr: Some("add1".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: OnFailure::Fail,
                    checkpoint: false,
                },
            ],
        };

        let out = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        match out {
            CtPipelineData::Value(CtValue::Int(n), _) => assert_eq!(n, 8),
            _ => panic!("expected Int(8)"),
        }
    }

    #[test]
    fn test_workflow_on_failure_continue_does_not_eagerly_buffer_stream_input() {
        use ctpipeline::metadata::CtPipelineMetadata;
        use std::io::Read;

        struct PanicReader;
        impl Read for PanicReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                panic!("stream input must not be eagerly read");
            }
        }

        let ctx = get_test_ctx();
        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "passthrough".into(),
                expr: None,
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: None,
                retry: None,
                on_failure: OnFailure::Continue,
                checkpoint: false,
            }],
        };

        let input = CtPipelineData::ByteStream(ctpipeline::CtByteStream::new(
            PanicReader,
            CtPipelineMetadata::default(),
        ));
        let out = run_workflow(&script, input, &ctx).unwrap();
        assert!(
            matches!(out, CtPipelineData::ByteStream(_)),
            "passthrough stage should keep stream input untouched"
        );
    }

    #[test]
    fn test_workflow_on_failure_continue_errors_when_recovery_input_missing() {
        use crate::command::DataCommand;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctsig::DataSignature;
        use std::io::{self, Read};

        struct BrokenReader;
        impl Read for BrokenReader {
            fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("broken"))
            }
        }

        struct AlwaysFail;
        impl DataCommand for AlwaysFail {
            fn signature(&self) -> DataSignature {
                DataSignature::new("boom", "boom")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _input: CtPipelineData,
                _ctx: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Err(CtDiagnosticError::simple("boom"))
            }
        }

        let reg = CommandRegistry::from_factories(&[("boom", || Box::new(AlwaysFail))]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);
        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "broken".into(),
                expr: Some("boom".into()),
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: None,
                retry: None,
                on_failure: OnFailure::Continue,
                checkpoint: false,
            }],
        };

        let input = CtPipelineData::ByteStream(ctpipeline::CtByteStream::new(
            BrokenReader,
            CtPipelineMetadata::default(),
        ));
        let err = run_workflow(&script, input, &ctx).unwrap_err();
        match err {
            WorkflowError::RunStage { stage, err } => {
                assert_eq!(stage, "broken");
                assert!(err.to_string().contains("failed to preserve input"));
                assert!(
                    err.to_string()
                        .contains("streaming input cannot be replayed")
                );
            }
            _ => panic!("Expected RunStage error"),
        }
    }

    #[test]
    fn test_workflow_on_failure_continue_records_recovered_error_in_trace() {
        use crate::command::DataCommand;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctpipeline::value::CtValue;
        use ctsig::DataSignature;

        struct Seed;
        impl DataCommand for Seed {
            fn signature(&self) -> DataSignature {
                DataSignature::new("seed", "seed")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _input: CtPipelineData,
                _ctx: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::Value(
                    CtValue::Int(1),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        struct AlwaysFail;
        impl DataCommand for AlwaysFail {
            fn signature(&self) -> DataSignature {
                DataSignature::new("boom", "boom")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _input: CtPipelineData,
                _ctx: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Err(CtDiagnosticError::simple("boom"))
            }
        }

        let reg = CommandRegistry::from_factories(&[
            ("seed", || Box::new(Seed)),
            ("boom", || Box::new(AlwaysFail)),
        ]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None).enable_trace();
        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "seed".into(),
                    expr: Some("seed".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: OnFailure::Fail,
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "boom".into(),
                    expr: Some("boom".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: OnFailure::Continue,
                    checkpoint: false,
                },
            ],
        };

        let out = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        assert!(matches!(out, CtPipelineData::Value(CtValue::Int(1), _)));

        let trace = ctx.trace_snapshot().expect("trace should be enabled");
        assert!(
            trace.stages.iter().any(|s| {
                s.cmd.contains("workflow:boom")
                    && matches!(s.status, crate::trace::TraceStatus::Error(_))
            }),
            "workflow recovery should leave an error breadcrumb in trace"
        );
    }

    #[test]
    fn test_workflow_interrupt_remains_latched_with_on_failure_continue() {
        use crate::command::DataCommand;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctsig::DataSignature;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static TOUCH_RUNS: AtomicUsize = AtomicUsize::new(0);

        struct Touch;
        impl DataCommand for Touch {
            fn signature(&self) -> DataSignature {
                DataSignature::new("touch", "touch")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                input: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                TOUCH_RUNS.fetch_add(1, Ordering::SeqCst);
                Ok(match input {
                    CtPipelineData::Empty => CtPipelineData::Value(
                        ctpipeline::value::CtValue::Int(1),
                        CtPipelineMetadata::default(),
                    ),
                    other => other,
                })
            }
        }

        TOUCH_RUNS.store(0, Ordering::SeqCst);
        let reg = CommandRegistry::from_factories(&[("touch", || Box::new(Touch))]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);
        ctx.signal.trigger();

        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "stage_interrupt_continue".into(),
                    expr: Some("touch".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: OnFailure::Continue,
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "stage_must_not_run".into(),
                    expr: Some("touch".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: OnFailure::Fail,
                    checkpoint: false,
                },
            ],
        };

        let err = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap_err();
        match err {
            WorkflowError::RunStage { stage, err } => {
                assert_eq!(stage, "stage_must_not_run");
                assert!(err.to_string().contains("interrupted"));
            }
            _ => panic!("expected RunStage error"),
        }
        assert_eq!(TOUCH_RUNS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_workflow_timeout_interrupts_materialization_early() {
        use crate::command::DataCommand;
        use ctpipeline::CtListStream;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctpipeline::value::CtValue;
        use ctsig::DataSignature;

        struct SlowIter {
            remain: usize,
        }

        impl Iterator for SlowIter {
            type Item = CtValue;

            fn next(&mut self) -> Option<Self::Item> {
                if self.remain == 0 {
                    return None;
                }
                self.remain -= 1;
                std::thread::sleep(Duration::from_millis(10));
                Some(CtValue::Int(self.remain as i64))
            }
        }

        struct SlowStream;
        impl DataCommand for SlowStream {
            fn signature(&self) -> DataSignature {
                DataSignature::new("slow-stream", "slow-stream")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::ListStream(CtListStream::new(
                    SlowIter { remain: 100 },
                    CtPipelineMetadata::default(),
                )))
            }
        }

        let reg = CommandRegistry::from_factories(&[("slow-stream", || Box::new(SlowStream))]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);
        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "timeout_materialize".into(),
                expr: Some("slow-stream".into()),
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: Some(50),
                retry: None,
                on_failure: OnFailure::Fail,
                checkpoint: false,
            }],
        };

        let started = Instant::now();
        let err = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap_err();
        let elapsed = started.elapsed();

        match err {
            WorkflowError::RunStage { err, .. } => {
                assert!(err.to_string().contains("timed out"));
                assert_eq!(err.code, 124);
            }
            _ => panic!("Expected RunStage timeout"),
        }
        assert!(
            elapsed < Duration::from_millis(400),
            "timeout should stop materialization early, elapsed={elapsed:?}"
        );
    }

    #[test]
    fn test_workflow_timeout_interrupts_blocking_bytestream_read() {
        use crate::command::DataCommand;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctsig::DataSignature;
        use std::io::{self, Read};

        struct SlowEofReader {
            slept: bool,
        }

        impl Read for SlowEofReader {
            fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
                if !self.slept {
                    self.slept = true;
                    std::thread::sleep(Duration::from_millis(300));
                }
                Ok(0)
            }
        }

        struct SlowByteStream;
        impl DataCommand for SlowByteStream {
            fn signature(&self) -> DataSignature {
                DataSignature::new("slow-bytes", "slow bytes")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::ByteStream(ctpipeline::CtByteStream::new(
                    SlowEofReader { slept: false },
                    CtPipelineMetadata::default(),
                )))
            }
        }

        let reg = CommandRegistry::from_factories(&[("slow-bytes", || Box::new(SlowByteStream))]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);
        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "timeout_bytes".into(),
                expr: Some("slow-bytes".into()),
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: Some(30),
                retry: None,
                on_failure: OnFailure::Fail,
                checkpoint: false,
            }],
        };

        let started = Instant::now();
        let err = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap_err();
        let elapsed = started.elapsed();

        match err {
            WorkflowError::RunStage { err, .. } => {
                assert!(err.to_string().contains("timed out"));
                assert_eq!(err.code, 124);
            }
            _ => panic!("Expected RunStage timeout"),
        }
        assert!(
            elapsed < Duration::from_millis(150),
            "timeout should not wait for blocking read to finish, elapsed={elapsed:?}"
        );
    }

    #[test]
    fn test_workflow_l3_checkpoint() {
        use crate::command::DataCommand;
        use ctpipeline::metadata::CtPipelineMetadata;
        use ctpipeline::value::CtValue;
        use ctsig::DataSignature;

        struct CheckpointCmd;
        impl DataCommand for CheckpointCmd {
            fn signature(&self) -> DataSignature {
                DataSignature::new("checkpoint-cmd", "checkpoint-cmd")
            }
            fn run(
                &self,
                _call: &ctsig::DataCall,
                _i: CtPipelineData,
                _c: &crate::context::DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::Value(
                    CtValue::Int(7),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        let reg =
            CommandRegistry::from_factories(&[("checkpoint-cmd", || Box::new(CheckpointCmd))]);
        let ctx = crate::context::DataEngineContext::new(reg, None, None);

        let stage_name = unique_stage_name("checkpoint_stage");
        remove_matching_checkpoints(&stage_name);

        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: stage_name.clone(),
                expr: Some("checkpoint-cmd".into()),
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: None,
                retry: None,
                on_failure: OnFailure::Fail,
                checkpoint: true,
            }],
        };

        let _ = run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        let paths = matching_checkpoint_paths(&stage_name);
        assert_eq!(paths.len(), 1, "checkpoint file should exist: {paths:?}");

        let raw = std::fs::read_to_string(&paths[0]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["stage"], stage_name);
        assert_eq!(parsed["stage_index"], 0);
        assert_eq!(parsed["data"], 7);

        remove_matching_checkpoints(&stage_name);
    }

    #[test]
    fn test_workflow_checkpoint_paths_include_run_identity() {
        let ctx = workflow_var_test_ctx();
        let stage_name = unique_stage_name("checkpoint_same_stage");
        remove_matching_checkpoints(&stage_name);

        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: stage_name.clone(),
                expr: Some("wf-const-99".into()),
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: None,
                retry: None,
                on_failure: OnFailure::Fail,
                checkpoint: true,
            }],
        };

        run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
        run_workflow(&script, CtPipelineData::Empty, &ctx).unwrap();

        let paths = matching_checkpoint_paths(&stage_name);
        assert_eq!(
            paths.len(),
            2,
            "two workflow runs of the same checkpoint stage should not overwrite each other: {paths:?}"
        );
        assert_ne!(paths[0], paths[1]);

        remove_matching_checkpoints(&stage_name);
    }

    #[test]
    fn test_goto_self_loop_is_terminated_by_transition_limit() {
        let ctx = workflow_var_test_ctx();
        // expr 引用不存在的命令，stage 每次都会报错并 goto 自身 → 无限循环。
        let script = WorkflowScript {
            stages: vec![WorkflowStage {
                name: "looping-stage".into(),
                expr: Some("wf-nonexistent-cmd".into()),
                if_cond: None,
                else_expr: None,
                foreach: None,
                var: None,
                timeout_ms: None,
                retry: None,
                on_failure: OnFailure::Goto("looping-stage".into()),
                checkpoint: false,
            }],
        };

        let result = run_workflow(&script, CtPipelineData::Empty, &ctx);
        assert!(result.is_err(), "expected error from infinite goto loop");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("maximum stage transitions"),
            "error message should mention transition limit: {msg}"
        );
    }

    #[test]
    fn test_goto_back_to_earlier_stage_is_terminated_by_transition_limit() {
        let ctx = workflow_var_test_ctx();
        // 两个阶段互相 goto，均执行失败的命令，形成死循环。
        let script = WorkflowScript {
            stages: vec![
                WorkflowStage {
                    name: "stage-a".into(),
                    expr: Some("wf-nonexistent-cmd".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: OnFailure::Goto("stage-b".into()),
                    checkpoint: false,
                },
                WorkflowStage {
                    name: "stage-b".into(),
                    expr: Some("wf-nonexistent-cmd".into()),
                    if_cond: None,
                    else_expr: None,
                    foreach: None,
                    var: None,
                    timeout_ms: None,
                    retry: None,
                    on_failure: OnFailure::Goto("stage-a".into()),
                    checkpoint: false,
                },
            ],
        };

        let result = run_workflow(&script, CtPipelineData::Empty, &ctx);
        assert!(result.is_err(), "expected error from cycle goto");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("maximum stage transitions"),
            "error message should mention transition limit: {msg}"
        );
    }
}
