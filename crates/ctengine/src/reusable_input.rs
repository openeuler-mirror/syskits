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

//! 可重放输入快照：用于 retry/wait-until 等需要多次执行表达式的命令。

use crate::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtValue};

#[derive(Debug, Clone)]
pub enum ReusableInput {
    Empty,
    Value(CtValue, CtPipelineMetadata),
}

impl ReusableInput {
    /// 将非流式输入快照化为可重放数据。
    pub fn from_pipeline(
        input: &CtPipelineData,
        streaming_error: &str,
    ) -> Result<Self, CtDiagnosticError> {
        match input {
            CtPipelineData::Empty => Ok(Self::Empty),
            CtPipelineData::Value(v, m) => Ok(Self::Value(v.clone(), m.clone())),
            CtPipelineData::ListStream(_) | CtPipelineData::ByteStream(_) => {
                Err(CtDiagnosticError::simple(streaming_error))
            }
        }
    }

    /// 还原为新的管线数据实例，供一次执行消费。
    pub fn to_pipeline_data(&self) -> CtPipelineData {
        match self {
            Self::Empty => CtPipelineData::Empty,
            Self::Value(v, m) => CtPipelineData::Value(v.clone(), m.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reusable_input_empty_roundtrip() {
        let snap = ReusableInput::from_pipeline(&CtPipelineData::Empty, "stream not supported")
            .expect("empty should be supported");
        assert!(matches!(snap.to_pipeline_data(), CtPipelineData::Empty));
    }

    #[test]
    fn reusable_input_rejects_stream() {
        let stream = ctpipeline::CtListStream::new(
            vec![CtValue::Int(1)].into_iter(),
            CtPipelineMetadata::default(),
        );
        let input = CtPipelineData::ListStream(stream);
        let err = ReusableInput::from_pipeline(&input, "stream not supported")
            .expect_err("stream should be rejected");
        assert!(err.to_string().contains("stream not supported"));
    }
}
