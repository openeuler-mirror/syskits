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

//! `DataCommand` trait — 数据管线命令的统一接口。
//!
//! 设计原则：
//! - `DataCommand` 与 Legacy `Tool` 完全独立，不继承、不混用
//! - `run()` 消费管线输入，返回管线输出
//! - 命令工厂类型 `DataCommandFactory` 为 `fn() -> Box<dyn DataCommand>`

use crate::context::DataEngineContext;
use crate::error::CtDiagnosticError;
use ctpipeline::CtPipelineData;
use ctsig::{DataCall, DataSignature};

/// 命令工厂函数类型（供 `DataTools` 宏生成注册表使用）
pub type DataCommandFactory = fn() -> Box<dyn DataCommand>;

/// 数据管线命令 trait
///
/// 所有内建 DataCommand（以及未来的插件命令）均实现此 trait。
pub trait DataCommand: Send + Sync {
    /// 命令签名（名称、参数、IO 类型）
    fn signature(&self) -> DataSignature;

    /// 执行命令
    ///
    /// - `call`：已绑定的参数集合
    /// - `input`：上游管线数据
    /// - `ctx`：引擎上下文（环境变量、注册表、信号等）
    ///
    /// 返回下游管线数据或诊断错误。
    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError>;
}
