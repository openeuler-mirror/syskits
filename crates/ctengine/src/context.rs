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

//! `DataEngineContext` — 引擎执行上下文，传递给每个 DataCommand。
//!
//! M1a 骨架：`SignalHandle` 为 no-op 存根，`CommandRegistry` 为空注册表。
//! M1b/M1c 阶段扩充解释器循环和实际命令填充。

use crate::command::{DataCommand, DataCommandFactory};
use crate::execution::OutputFormat;
use crate::legacy_adapter::{LegacyToolAdapter, LegacyToolResolver};
use crate::trace::{PipelineTrace, StageTrace};
use ctsig::DataSignature;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// 环境变量存储（有序 BTreeMap，使 `env` 命令输出按字母序）
pub type EnvStore = BTreeMap<String, String>;

/// 插件提供者 trait（抽象接口以打破 ctplugin 和 ctengine 循环依赖）
pub trait PluginProvider: Send + Sync + std::fmt::Debug {
    fn get_command(&self, name: &str) -> Option<Box<dyn DataCommand>>;
}

/// 标准输出目标
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutDest {
    /// 写入真实 stdout
    Stdout,
    /// 写入命名文件
    File(String),
    /// 丢弃输出（`null`）
    Null,
    /// 捕获到内存缓冲区（用于管线传递）
    Capture,
}

/// 错误输出目标
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrDest {
    /// 写入真实 stderr
    Stderr,
    /// 合并到 stdout
    Stdout,
    /// 丢弃
    Null,
}

/// 信号句柄 — 支持 no-op 与真实 SIGINT 注册两种模式。
///
/// - `noop()`：始终未中断（测试用）
/// - `register_sigint()`：安装 SIGINT 处理器，Ctrl-C 时设置标志
///
/// 设计为 `Clone + Send + Sync`，可在多线程中共享。
/// 对应 LLD §12：`signals` feature 时使用系统信号适配，否则 no-op。
#[derive(Clone, Debug)]
pub struct SignalHandle {
    interrupted: Arc<AtomicBool>,
    seen_global_epoch: Arc<AtomicU64>,
}

/// 全局中断代次计数，供 C 信号处理器写入。
/// 每次收到 SIGINT 时递增，避免并发上下文通过 reset 彼此干扰。
static GLOBAL_INTERRUPT_EPOCH: AtomicU64 = AtomicU64::new(0);

impl SignalHandle {
    /// 创建 no-op 句柄（始终未中断，用于测试与非 signals 场景）
    pub fn noop() -> Self {
        Self {
            interrupted: Arc::new(AtomicBool::new(false)),
            seen_global_epoch: Arc::new(AtomicU64::new(
                GLOBAL_INTERRUPT_EPOCH.load(Ordering::SeqCst),
            )),
        }
    }

    /// 注册真实 SIGINT 处理器。
    ///
    /// 调用后，Ctrl-C 会将 `interrupted()` 置为 `true`。
    /// 安全限制：使用全局中断代次计数，进程级单例。
    /// 仅在 `unix` 平台可用；非 unix 回落 no-op。
    pub fn register_sigint() -> Self {
        #[cfg(unix)]
        {
            // 安全性：信号处理器仅操作原子计数，不调用非 signal-safe 函数。
            unsafe {
                libc::signal(libc::SIGINT, sigint_handler as libc::sighandler_t);
            }
        }

        Self {
            interrupted: Arc::new(AtomicBool::new(false)),
            seen_global_epoch: Arc::new(AtomicU64::new(
                GLOBAL_INTERRUPT_EPOCH.load(Ordering::SeqCst),
            )),
        }
    }

    /// 检查是否收到中断信号
    pub fn interrupted(&self) -> bool {
        let local = self.interrupted.load(Ordering::Relaxed);
        let global = GLOBAL_INTERRUPT_EPOCH.load(Ordering::Relaxed);
        let seen = self.seen_global_epoch.load(Ordering::Relaxed);
        local || global != seen
    }

    /// 消费一次中断信号并清除标记。
    ///
    /// 适用于 REPL 等复用上下文场景，避免一次 Ctrl-C 影响后续命令。
    pub fn take_interrupted(&self) -> bool {
        let local = self.interrupted.swap(false, Ordering::Relaxed);
        let global = GLOBAL_INTERRUPT_EPOCH.load(Ordering::SeqCst);
        let seen = self.seen_global_epoch.swap(global, Ordering::SeqCst);
        local || global != seen
    }

    /// 触发中断（用于测试或外部信号处理器回调）
    pub fn trigger(&self) {
        self.interrupted.store(true, Ordering::Relaxed);
    }
}

/// C 级 SIGINT 处理器 — 仅设置全局原子标志，signal-safe。
#[cfg(unix)]
extern "C" fn sigint_handler(_sig: libc::c_int) {
    GLOBAL_INTERRUPT_EPOCH.fetch_add(1, Ordering::SeqCst);
}

/// DataCommand 注册表
///
/// 从 `DataTools` 宏生成的工厂函数列表构建，
/// 支持按名称查找命令实例。
pub struct CommandRegistry {
    factories: HashMap<&'static str, DataCommandFactory>,
}

impl CommandRegistry {
    /// 从工厂列表构建注册表
    pub fn from_factories(factories: &[(&'static str, DataCommandFactory)]) -> Self {
        Self {
            factories: factories.iter().copied().collect(),
        }
    }

    /// 创建空注册表（M1a 骨架使用）
    pub fn empty() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// 按名称查找并实例化命令
    pub fn get(&self, name: &str) -> Option<Box<dyn DataCommand>> {
        self.factories.get(name).map(|f| f())
    }

    /// 返回所有已注册命令名
    pub fn command_names(&self) -> Vec<&'static str> {
        let mut names: Vec<_> = self.factories.keys().copied().collect();
        names.sort_unstable();
        names
    }

    /// 导出命令签名表（供 REPL precheck 使用）
    pub fn command_signatures(&self) -> HashMap<String, DataSignature> {
        let mut map = HashMap::new();
        for name in self.command_names() {
            if let Some(cmd) = self.get(name) {
                map.insert(name.to_string(), cmd.signature());
            }
        }
        map
    }
}

impl std::fmt::Debug for CommandRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandRegistry")
            .field("count", &self.factories.len())
            .finish()
    }
}

/// 引擎执行上下文（传递给每个 `DataCommand::run()`）
#[derive(Debug)]
pub struct DataEngineContext {
    /// 环境变量（可变，命令可设置/删除）
    pub env: EnvStore,
    /// 当前标准输出目标
    pub out_dest: OutDest,
    /// 当前标准错误目标
    pub err_dest: ErrDest,
    /// 信号句柄
    pub signal: SignalHandle,
    /// 命令注册表
    pub registry: CommandRegistry,
    /// legacy Tool 适配器（可选）
    pub legacy_adapter: Option<LegacyToolAdapter>,
    /// 是否启用 legacy 适配器
    pub use_legacy_tools: bool,
    /// 插件提供者
    pub plugin_registry: Option<Arc<dyn PluginProvider>>,
    /// 可选 pipeline trace（默认由环境变量控制）
    pub trace: Option<Arc<Mutex<PipelineTrace>>>,
    /// 当前 data 入口输出格式，供少数命令区分 classic 与 native 副作用策略。
    pub output_format: OutputFormat,
}

impl DataEngineContext {
    /// 创建最简上下文（继承当前进程环境变量）
    pub fn new(
        registry: CommandRegistry,
        legacy_resolver: Option<LegacyToolResolver>,
        plugin_registry: Option<Arc<dyn PluginProvider>>,
    ) -> Self {
        let enable_legacy = legacy_resolver.is_some();
        Self {
            env: EnvStore::new(),
            out_dest: OutDest::Stdout,
            err_dest: ErrDest::Stderr,
            signal: SignalHandle::noop(), // Changed from new() to noop() to match existing SignalHandle API
            registry,                     // Changed from commands to registry
            legacy_adapter: legacy_resolver.map(LegacyToolAdapter::new),
            use_legacy_tools: enable_legacy,
            plugin_registry,
            trace: trace_from_env().then(|| Arc::new(Mutex::new(PipelineTrace::default()))),
            output_format: OutputFormat::Auto,
        }
    }

    /// M1a 测试辅助方法（仅提供极简空上下文，无 resolver 和 plugins）
    pub fn empty_for_test() -> Self {
        Self::new(CommandRegistry::empty(), None, None)
    }

    /// 强制更新信号句柄（仅在 ctrlc 初始化后调用）
    pub fn with_signal(mut self, signal: SignalHandle) -> Self {
        self.signal = signal;
        self
    }

    /// 设置当前 data 入口输出格式。
    pub fn with_output_format(mut self, output_format: OutputFormat) -> Self {
        self.output_format = output_format;
        self
    }

    /// 显式启用 trace（主要用于测试）
    pub fn enable_trace(mut self) -> Self {
        if self.trace.is_none() {
            self.trace = Some(Arc::new(Mutex::new(PipelineTrace::default())));
        }
        self
    }

    pub fn record_stage_trace(&self, stage: StageTrace) {
        if let Some(trace) = &self.trace {
            let mut guard = trace.lock().expect("pipeline trace mutex poisoned");
            guard.record(stage);
        }
    }

    pub fn set_trace_total_ms(&self, total_ms: u64) {
        if let Some(trace) = &self.trace {
            let mut guard = trace.lock().expect("pipeline trace mutex poisoned");
            guard.set_total_ms(total_ms);
        }
    }

    pub fn emit_trace_if_enabled(&self) {
        if let Some(trace) = &self.trace {
            let guard = trace.lock().expect("pipeline trace mutex poisoned");
            for line in guard.format_lines() {
                eprintln!("{line}");
            }
        }
    }

    pub fn trace_snapshot(&self) -> Option<PipelineTrace> {
        self.trace
            .as_ref()
            .map(|trace| trace.lock().expect("pipeline trace mutex poisoned").clone())
    }

    pub fn clear_trace(&self) {
        if let Some(trace) = &self.trace {
            let mut guard = trace.lock().expect("pipeline trace mutex poisoned");
            guard.reset();
        }
    }
}

fn trace_from_env() -> bool {
    let value = std::env::var("SYSKITS_DATA_TRACE").ok();
    parse_trace_env_value(value.as_deref())
}

fn parse_trace_env_value(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()),
        Some(v) if v == "1" || v == "true" || v == "yes" || v == "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_handle_noop() {
        let handle = SignalHandle::noop();
        assert!(!handle.interrupted());
    }

    #[test]
    fn test_signal_handle_trigger() {
        let handle = SignalHandle::noop();
        handle.trigger();
        assert!(handle.interrupted());
    }

    #[test]
    fn test_signal_handle_take_interrupted_clears_flag() {
        let handle = SignalHandle::noop();
        handle.trigger();
        assert!(handle.take_interrupted());
        assert!(!handle.take_interrupted());
        assert!(!handle.interrupted());
    }

    #[test]
    fn test_out_dest_variants() {
        let _ = OutDest::Stdout;
        let _ = OutDest::Null;
        let _ = OutDest::Capture;
        let _ = OutDest::File("output.txt".to_string());
    }

    #[test]
    fn test_err_dest_variants() {
        let _ = ErrDest::Stderr;
        let _ = ErrDest::Stdout;
        let _ = ErrDest::Null;
    }

    #[test]
    fn test_command_registry_empty() {
        let reg = CommandRegistry::empty();
        assert!(reg.command_names().is_empty());
        assert!(reg.get("foo").is_none());
    }

    #[test]
    fn test_engine_context_new() {
        let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None);
        assert_eq!(ctx.out_dest, OutDest::Stdout);
        assert_eq!(ctx.err_dest, ErrDest::Stderr);
        assert!(!ctx.signal.interrupted());
        assert!(ctx.legacy_adapter.is_none());
    }

    #[test]
    fn test_engine_context_with_legacy() {
        fn resolver(_name: &str) -> Option<Box<dyn ctcore::Tool>> {
            None
        }
        let ctx = DataEngineContext::new(CommandRegistry::empty(), Some(resolver), None);
        assert!(ctx.legacy_adapter.is_some());
    }

    #[test]
    fn test_parse_trace_env_value() {
        assert!(parse_trace_env_value(Some("1")));
        assert!(parse_trace_env_value(Some("true")));
        assert!(parse_trace_env_value(Some("YES")));
        assert!(!parse_trace_env_value(Some("0")));
        assert!(!parse_trace_env_value(Some("")));
        assert!(!parse_trace_env_value(None));
    }

    #[test]
    fn test_enable_trace_and_snapshot() {
        let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None).enable_trace();
        assert!(ctx.trace.is_some());
        let snap = ctx.trace_snapshot().unwrap();
        assert_eq!(snap.stages.len(), 0);
        assert_eq!(snap.total_ms, 0);
    }

    #[test]
    fn test_clear_trace_resets_previous_stages() {
        let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None).enable_trace();
        ctx.record_stage_trace(StageTrace {
            name: Some("s0".to_string()),
            cmd: "from".to_string(),
            duration_ms: 1,
            rows_in: 0,
            rows_out: 1,
            status: crate::trace::TraceStatus::Ok,
        });
        ctx.set_trace_total_ms(1);
        let snap_before = ctx.trace_snapshot().expect("trace should exist");
        assert_eq!(snap_before.stages.len(), 1);
        assert_eq!(snap_before.total_ms, 1);

        ctx.clear_trace();
        let snap_after = ctx.trace_snapshot().expect("trace should exist");
        assert!(snap_after.stages.is_empty());
        assert_eq!(snap_after.total_ms, 0);
    }
}
