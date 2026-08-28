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

//! Normalized Intermediate Representation (NIR) prototype for SysKits.

use ctdsl::ast::Call;

#[derive(Debug, Clone)]
pub enum NirNode<'a> {
    Command(&'a Call),
}

#[derive(Debug, Clone)]
pub struct NirStage<'a> {
    pub node: NirNode<'a>,
}

#[derive(Debug, Clone)]
pub struct NirPipeline<'a> {
    pub stages: Vec<NirStage<'a>>,
}
