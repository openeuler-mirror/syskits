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

use crate::error::CtDiagnosticError;
use crate::nir::{NirNode, NirPipeline, NirStage};
use ctdsl::Expr;

pub struct Binder;

impl Binder {
    /// Binds an Abstract Syntax Tree (AST) expression into a Normalized Intermediate Representation (NIR) Pipeline.
    pub fn bind(expr: &Expr) -> Result<NirPipeline<'_>, CtDiagnosticError> {
        let mut stages = Vec::new();
        // Since Expr only has a Pipeline(Vec<Call>) variant (or directly returns calls via expr.stages())
        for call in expr.stages() {
            stages.push(NirStage {
                node: NirNode::Command(call),
            });
        }
        Ok(NirPipeline { stages })
    }
}
