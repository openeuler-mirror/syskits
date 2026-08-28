/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_ps` — 输出结构化进程列表。

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctengine::execution::{CommandCore, CommandRunner};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};

#[derive(Default)]
pub struct CmdPs;

struct PsCore;

impl DataCommand for CmdPs {
    fn signature(&self) -> DataSignature {
        DataSignature::new("ps", "structured process list (linux only for now)")
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        _call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        CommandRunner::run(&PsCore, _call, input, ctx)
    }
}

impl CommandCore for PsCore {
    fn run_core(
        &self,
        _call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        ps_core_pipeline()
    }
}

/// 统一核心输出：供 DataCommand 与 Legacy Tool 共同复用。
pub fn ps_core_pipeline() -> Result<CtPipelineData, CtDiagnosticError> {
    let rows = collect_processes()?;
    Ok(CtPipelineData::Value(
        CtValue::List(rows),
        CtPipelineMetadata::default(),
    ))
}

#[cfg(target_os = "linux")]
fn collect_processes() -> Result<Vec<CtValue>, CtDiagnosticError> {
    use std::fs;

    let page_size = page_size_bytes();
    let ticks_per_sec = clock_ticks_per_second();
    let uptime_seconds = system_uptime_seconds()?;
    let mut rows = Vec::new();

    let entries = fs::read_dir("/proc")
        .map_err(|e| CtDiagnosticError::simple(format!("ps: cannot read /proc: {e}")))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid_str = name.to_string_lossy();
        if !pid_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(pid) = pid_str.parse::<i64>() else {
            continue;
        };
        if let Some(row) = read_process_row(pid, page_size, ticks_per_sec, uptime_seconds) {
            rows.push(row);
        }
    }

    rows.sort_by(|a, b| {
        let pid_a = extract_pid(a).unwrap_or(i64::MAX);
        let pid_b = extract_pid(b).unwrap_or(i64::MAX);
        pid_a.cmp(&pid_b)
    });

    Ok(rows)
}

#[cfg(not(target_os = "linux"))]
fn collect_processes() -> Result<Vec<CtValue>, CtDiagnosticError> {
    Err(CtDiagnosticError::simple(
        "ps: structured ps is currently supported on linux only",
    ))
}

#[cfg(target_os = "linux")]
fn read_process_row(
    pid: i64,
    page_size: u64,
    ticks_per_sec: u64,
    uptime_seconds: f64,
) -> Option<CtValue> {
    let stat_path = format!("/proc/{pid}/stat");
    let stat = std::fs::read_to_string(&stat_path).ok()?;
    let (name, status, cpu_percent) = parse_stat(&stat, ticks_per_sec, uptime_seconds)?;

    let statm_path = format!("/proc/{pid}/statm");
    let mem = parse_mem_from_statm(&statm_path, page_size).unwrap_or(0);

    Some(CtValue::Record(vec![
        ("pid".to_string(), CtValue::Int(pid)),
        ("name".to_string(), CtValue::String(name)),
        // `cpu` 使用当前进程累计 CPU 占比（%）语义。
        ("cpu".to_string(), CtValue::Float(cpu_percent)),
        ("mem".to_string(), CtValue::Size(mem)),
        ("status".to_string(), CtValue::String(status)),
    ]))
}

#[cfg(target_os = "linux")]
fn parse_stat(
    stat_line: &str,
    ticks_per_sec: u64,
    uptime_seconds: f64,
) -> Option<(String, String, f64)> {
    // /proc/<pid>/stat: pid (comm) state ...
    let l = stat_line.find('(')?;
    let r = stat_line.rfind(')')?;
    if r <= l {
        return None;
    }
    let name = stat_line[l + 1..r].to_string();
    let tail = stat_line[r + 1..].trim();
    let mut parts = tail.split_whitespace();
    let status = parts.next()?.to_string();

    // utime/stime 在原始 stat 中是第 14/15 字段；
    // 去掉 pid+comm 后，分别是 tail 的第 12/13 个字段（0-based 11/12）。
    let rest = tail.split_whitespace().collect::<Vec<_>>();
    let utime = rest
        .get(11)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let stime = rest
        .get(12)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let start_ticks = rest
        .get(19)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let hz = ticks_per_sec.max(1) as f64;

    let total_cpu_seconds = (utime.saturating_add(stime) as f64) / hz;
    let start_seconds = (start_ticks as f64) / hz;
    let elapsed_seconds = (uptime_seconds - start_seconds).max(1.0 / hz);
    let cpu_percent = (total_cpu_seconds / elapsed_seconds) * 100.0;
    let cpu_percent = if cpu_percent.is_finite() && cpu_percent >= 0.0 {
        cpu_percent
    } else {
        0.0
    };
    Some((name, status, cpu_percent))
}

#[cfg(target_os = "linux")]
fn parse_mem_from_statm(path: &str, page_size: u64) -> Option<u64> {
    let statm = std::fs::read_to_string(path).ok()?;
    let fields = statm.split_whitespace().collect::<Vec<_>>();
    let resident_pages = fields.get(1)?.parse::<u64>().ok()?;
    Some(resident_pages.saturating_mul(page_size))
}

#[cfg(target_os = "linux")]
fn page_size_bytes() -> u64 {
    let sz = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if sz > 0 { sz as u64 } else { 4096 }
}

#[cfg(target_os = "linux")]
fn clock_ticks_per_second() -> u64 {
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz > 0 { hz as u64 } else { 100 }
}

#[cfg(target_os = "linux")]
fn system_uptime_seconds() -> Result<f64, CtDiagnosticError> {
    let raw = std::fs::read_to_string("/proc/uptime")
        .map_err(|e| CtDiagnosticError::simple(format!("ps: cannot read /proc/uptime: {e}")))?;
    let first = raw
        .split_whitespace()
        .next()
        .ok_or_else(|| CtDiagnosticError::simple("ps: unexpected /proc/uptime format"))?;
    let uptime = first.parse::<f64>().map_err(|e| {
        CtDiagnosticError::simple(format!("ps: invalid uptime value `{first}`: {e}"))
    })?;
    Ok(uptime.max(0.0))
}

fn extract_pid(v: &CtValue) -> Option<i64> {
    let CtValue::Record(fields) = v else {
        return None;
    };
    fields.iter().find_map(|(k, v)| {
        if k == "pid" {
            if let CtValue::Int(n) = v {
                Some(*n)
            } else {
                None
            }
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn ps_returns_records() {
        let out = CmdPs.run(&DataCall::named("ps"), CtPipelineData::Empty, &ctx());
        let CtPipelineData::Value(CtValue::List(rows), _) = out.expect("ps should run") else {
            panic!("expected list");
        };
        assert!(!rows.is_empty());
        if let Some(CtValue::Record(fields)) = rows.first() {
            assert!(fields.iter().any(|(k, _)| k == "pid"));
            assert!(fields.iter().any(|(k, _)| k == "name"));
            assert!(fields.iter().any(|(k, _)| k == "cpu"));
            assert!(fields.iter().any(|(k, _)| k == "mem"));
            assert!(fields.iter().any(|(k, _)| k == "status"));
            assert!(matches!(
                fields.iter().find(|(k, _)| k == "mem").map(|(_, v)| v),
                Some(CtValue::Size(_))
            ));
            let cpu_value = fields.iter().find(|(k, _)| k == "cpu").map(|(_, v)| v);
            assert!(matches!(cpu_value, Some(CtValue::Float(v)) if *v >= 0.0));
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn parse_stat_cpu_is_percent() {
        // utime+stime = 300 ticks => 3s; starttime = 1000 ticks => 10s;
        // uptime = 40s => elapsed = 30s => cpu = 10%
        let line = "123 (proc) S 0 0 0 0 0 0 0 0 0 0 200 100 0 0 0 0 0 0 1000";
        let (_, _, cpu_percent) = parse_stat(line, 100, 40.0).expect("parse stat");
        assert!((cpu_percent - 10.0).abs() < f64::EPSILON);
    }
}
