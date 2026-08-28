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

//! 命令改造双适配器骨架。
//!
//! - `DataAdapter`: 将 `DataCommand` 统一包装为 `CommandCore`
//! - `LegacyAdapter`: 对 legacy Tool 适配器统一命名，便于后续迁移
//! - `legacy_default_output_profile`: legacy 文本兼容视图缺省配置

use crate::command::DataCommand;
use crate::context::DataEngineContext;
use crate::error::CtDiagnosticError;
use crate::execution::{CommandCore, OutputProfile};
use crate::legacy_adapter::LegacyToolAdapter;
use ctpipeline::CtPipelineData;
use ctsig::DataCall;

/// DataCommand 侧适配器：避免在解释器里重复定义内联 wrapper。
pub struct DataAdapter<'a> {
    cmd: &'a dyn DataCommand,
}

impl<'a> DataAdapter<'a> {
    pub fn new(cmd: &'a dyn DataCommand) -> Self {
        Self { cmd }
    }
}

impl CommandCore for DataAdapter<'_> {
    fn run_core(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        self.cmd.run(call, input, ctx)
    }
}

/// Legacy 侧统一命名（现阶段复用已有实现）。
pub type LegacyAdapter = LegacyToolAdapter;

/// Legacy 命令缺省视图：coreutils 文本兼容优先。
pub const fn legacy_default_output_profile(stdout_is_tty: bool) -> OutputProfile {
    OutputProfile::for_legacy(stdout_is_tty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::CommandRegistry;
    use ctpipeline::{CtPipelineMetadata, CtValue};

    #[derive(Default)]
    struct Echo;

    impl DataCommand for Echo {
        fn signature(&self) -> ctsig::DataSignature {
            ctsig::DataSignature::new("echo", "echo")
                .input(ctpipeline::CtType::Any)
                .output(ctpipeline::CtType::Any)
        }

        fn run(
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

    #[test]
    fn data_adapter_bridges_data_command_to_core() {
        let cmd = Echo;
        let adapter = DataAdapter::new(&cmd);
        let call = DataCall::named("echo");
        let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None);
        let out = adapter
            .run_core(&call, CtPipelineData::Empty, &ctx)
            .expect("adapter should pass through");
        assert!(matches!(
            out,
            CtPipelineData::Value(CtValue::String(s), _) if s == "ok"
        ));
    }

    #[test]
    fn legacy_default_profile_uses_legacy_format() {
        let p = legacy_default_output_profile(false);
        assert_eq!(p.format, crate::execution::OutputFormat::Classic);
    }
}
