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

use crate::ct_error::CTResult;
use clap::Command;
use std::ffi::OsString;

/// 统一的工具接口，所有工具都必须实现此接口
pub trait Tool: Send + Sync {
    /// 获取工具名称
    fn name(&self) -> &'static str;

    /// 创建 clap 命令
    fn command(&self) -> Command;

    /// 执行工具功能
    fn execute(&self, args: &[OsString]) -> CTResult<()>;
}
