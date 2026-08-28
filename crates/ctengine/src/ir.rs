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

//! Intermediate Representation (IR) for pipeline optimization.

use ctdsl::ast::Call;

/// IR operation types
#[derive(Debug, Clone)]
pub enum IrOp<'a> {
    /// A standalone command execution
    Command(&'a Call),
    /// An optimized merged operation (e.g., where + select)
    MergedWhereSelect {
        where_call: &'a Call,
        select_call: &'a Call,
    },
}

/// A pipeline represented as a sequence of IR operations
#[derive(Debug, Clone)]
pub struct IrPipeline<'a> {
    pub ops: Vec<IrOp<'a>>,
}
