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

//! 统一执行契约：输出视图策略与退出码策略。

use crate::context::DataEngineContext;
use crate::error::CtDiagnosticError;
use ctpipeline::CtPipelineData;
use ctsig::DataCall;

/// 统一退出码常量。
pub mod exit_code {
    pub const SUCCESS: i32 = 0;
    pub const RUNTIME_ERROR: i32 = 1;
    pub const USAGE_ERROR: i32 = 2;
    pub const TIMEOUT: i32 = 124;
    pub const INTERRUPTED: i32 = 130;
}

/// 输出格式策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// 传统命令行 / GNU 风格兼容视图。
    Classic,
    /// 自动模式（根据 TTY 与数据形态选择）
    #[default]
    Auto,
    /// 文本模式（兼容流式脚本）
    Text,
    /// 表格模式（Record/List<Record> 优先渲染表格）
    Table,
    /// JSON 模式（结构化序列化）
    Json,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "classic" => Some(Self::Classic),
            "auto" => Some(Self::Auto),
            "text" => Some(Self::Text),
            "table" => Some(Self::Table),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// 运行时输出视图配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputProfile {
    pub format: OutputFormat,
    pub stdout_is_tty: bool,
    pub use_pager: bool,
}

impl Default for OutputProfile {
    fn default() -> Self {
        Self::text_stream()
    }
}

impl OutputProfile {
    /// 稳定文本流输出（适配非 TTY/脚本场景）
    pub const fn text_stream() -> Self {
        Self {
            format: OutputFormat::Text,
            stdout_is_tty: false,
            use_pager: false,
        }
    }

    /// `syskits data` CLI 默认视图。
    pub const fn for_data_cli(stdout_is_tty: bool) -> Self {
        Self {
            format: OutputFormat::Auto,
            stdout_is_tty,
            use_pager: false,
        }
    }

    /// REPL 友好视图。
    pub const fn for_repl() -> Self {
        Self {
            format: OutputFormat::Auto,
            stdout_is_tty: true,
            use_pager: true,
        }
    }

    /// classic 兼容视图。
    pub const fn for_legacy(stdout_is_tty: bool) -> Self {
        Self {
            format: OutputFormat::Classic,
            stdout_is_tty,
            use_pager: false,
        }
    }

    /// 过渡别名：保留给仍未迁移的 legacy 命令调用点。
    pub const fn for_coreutils_text(stdout_is_tty: bool) -> Self {
        Self::for_legacy(stdout_is_tty)
    }
}

/// 退出码归一化策略。
#[derive(Debug, Clone, Copy, Default)]
pub struct ExitPolicy;

impl ExitPolicy {
    /// 诊断错误映射为进程退出码。
    pub fn from_diagnostic(err: &CtDiagnosticError) -> i32 {
        Self::normalize(err.code)
    }

    /// 用法错误返回码。
    pub const fn usage_error() -> i32 {
        exit_code::USAGE_ERROR
    }

    /// 运行成功返回码。
    pub const fn success() -> i32 {
        exit_code::SUCCESS
    }

    /// 归一化外部输入退出码，避免出现 0 或负值。
    pub fn normalize(code: i32) -> i32 {
        match code {
            exit_code::SUCCESS => exit_code::RUNTIME_ERROR,
            exit_code::USAGE_ERROR => exit_code::USAGE_ERROR,
            exit_code::TIMEOUT => exit_code::TIMEOUT,
            exit_code::INTERRUPTED => exit_code::INTERRUPTED,
            c if c > 0 => c,
            _ => exit_code::RUNTIME_ERROR,
        }
    }
}

/// 命令唯一核心逻辑接口（与具体输出视图无关）。
pub trait CommandCore: Send + Sync {
    fn run_core(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError>;
}

/// 通用核心执行器（后续可插入统一前后置钩子）。
pub struct CommandRunner;

impl CommandRunner {
    pub fn run(
        core: &dyn CommandCore,
        call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        core.run_core(call, input, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctpipeline::{CtPipelineMetadata, CtValue};

    #[test]
    fn output_format_parse_supports_single_axis_public_values() {
        assert_eq!(OutputFormat::parse("classic"), Some(OutputFormat::Classic));
        assert_eq!(OutputFormat::parse("auto"), Some(OutputFormat::Auto));
        assert_eq!(OutputFormat::parse("text"), Some(OutputFormat::Text));
        assert_eq!(OutputFormat::parse("table"), Some(OutputFormat::Table));
        assert_eq!(OutputFormat::parse("json"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::parse("raw"), None);
        assert_eq!(OutputFormat::parse("legacy"), None);
        assert_eq!(OutputFormat::parse("native"), None);
        assert_eq!(OutputFormat::parse("coreutils"), None);
    }

    #[test]
    fn output_profile_presets_follow_single_axis_contract() {
        assert_eq!(OutputProfile::text_stream().format, OutputFormat::Text);
        assert_eq!(OutputProfile::for_data_cli(true).format, OutputFormat::Auto);
        assert_eq!(OutputProfile::for_repl().format, OutputFormat::Auto);
        assert_eq!(
            OutputProfile::for_legacy(false).format,
            OutputFormat::Classic
        );
    }

    #[test]
    fn repl_profile_defaults_to_auto_not_table() {
        assert_eq!(OutputProfile::for_repl().format, OutputFormat::Auto);
    }

    #[test]
    fn exit_policy_normalize() {
        assert_eq!(ExitPolicy::normalize(0), exit_code::RUNTIME_ERROR);
        assert_eq!(ExitPolicy::normalize(2), exit_code::USAGE_ERROR);
        assert_eq!(ExitPolicy::normalize(124), exit_code::TIMEOUT);
        assert_eq!(ExitPolicy::normalize(130), exit_code::INTERRUPTED);
        assert_eq!(ExitPolicy::normalize(7), 7);
        assert_eq!(ExitPolicy::normalize(-1), exit_code::RUNTIME_ERROR);
    }

    #[test]
    fn command_runner_dispatches_core() {
        struct EchoCore;
        impl CommandCore for EchoCore {
            fn run_core(
                &self,
                _call: &DataCall,
                _input: CtPipelineData,
                _ctx: &DataEngineContext,
            ) -> Result<CtPipelineData, CtDiagnosticError> {
                Ok(CtPipelineData::Value(
                    CtValue::String("ok".to_string()),
                    CtPipelineMetadata::default(),
                ))
            }
        }

        let call = DataCall::named("echo");
        let ctx = DataEngineContext::empty_for_test();
        let out = CommandRunner::run(&EchoCore, &call, CtPipelineData::Empty, &ctx).unwrap();
        assert!(matches!(
            out,
            CtPipelineData::Value(CtValue::String(s), _) if s == "ok"
        ));
    }
}
