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

//! Compiler to lower NIR into IR.

use crate::ir::{IrOp, IrPipeline};
use crate::nir::{NirNode, NirPipeline};

/// Compiles a Normalized Intermediate Representation (NIR) into an optimized IR pipeline.
pub fn compile<'a>(nir: &NirPipeline<'a>) -> IrPipeline<'a> {
    let stages = &nir.stages;
    let mut ops = Vec::new();
    let mut i = 0;

    while i < stages.len() {
        let current = &stages[i];

        let NirNode::Command(current_call) = current.node;
        // Peephole optimization: merge adjacent "where" and "select"
        if current_call.name == "where" && i + 1 < stages.len() {
            let NirNode::Command(next_call) = stages[i + 1].node;
            if next_call.name == "select" {
                ops.push(IrOp::MergedWhereSelect {
                    where_call: current_call,
                    select_call: next_call,
                });
                i += 2;
                continue;
            }
        }

        // Default: just a standard command
        ops.push(IrOp::Command(current_call));
        i += 1;
    }

    IrPipeline { ops }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binder::Binder;
    use ctdsl::parse;

    #[test]
    fn test_compile_no_optimization() {
        let expr = parse("ls | from").unwrap();
        let nir = Binder::bind(&expr).unwrap();
        let pipeline = compile(&nir);
        assert_eq!(pipeline.ops.len(), 2);
        assert!(matches!(pipeline.ops[0], IrOp::Command(c) if c.name == "ls"));
        assert!(matches!(pipeline.ops[1], IrOp::Command(c) if c.name == "from"));
    }

    #[test]
    fn test_compile_merge_where_select() {
        let expr = parse("ls | where size > 100 | select name").unwrap();
        let nir = Binder::bind(&expr).unwrap();
        let pipeline = compile(&nir);
        assert_eq!(pipeline.ops.len(), 2);
        assert!(matches!(pipeline.ops[0], IrOp::Command(c) if c.name == "ls"));
        assert!(
            matches!(&pipeline.ops[1], IrOp::MergedWhereSelect { where_call, select_call } 
            if where_call.name == "where" && select_call.name == "select")
        );
    }
}
