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

//! `CtPipelineData` — 管线中流动的顶层数据容器。
//!
//! 设计要点：
//! - `CtListStream` 和 `CtByteStream` 均不可 `Clone`（包含 `Box<dyn ...>`）
//! - 所有变体均携带 `CtPipelineMetadata`
//! - `CtPipelineData::collect_values()` 消费 ListStream 转化为具象 Vec

use crate::metadata::CtPipelineMetadata;
use crate::value::CtValue;
use std::io::Read;

/// 惰性值流（来自管线，不可克隆）
pub struct CtListStream {
    iter: Box<dyn Iterator<Item = CtValue> + Send>,
    pub metadata: CtPipelineMetadata,
}

impl CtListStream {
    /// 从迭代器构造惰性流
    pub fn new(
        iter: impl Iterator<Item = CtValue> + Send + 'static,
        metadata: CtPipelineMetadata,
    ) -> Self {
        Self {
            iter: Box::new(iter),
            metadata,
        }
    }
}

impl Iterator for CtListStream {
    type Item = CtValue;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

impl std::fmt::Debug for CtListStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CtListStream").finish_non_exhaustive()
    }
}

/// 字节流（来自外部命令 stdout 或文件，不可克隆）
pub struct CtByteStream {
    reader: Box<dyn Read + Send>,
    pub metadata: CtPipelineMetadata,
}

impl CtByteStream {
    /// 从任意 `Read` 构造字节流
    pub fn new(reader: impl Read + Send + 'static, metadata: CtPipelineMetadata) -> Self {
        Self {
            reader: Box::new(reader),
            metadata,
        }
    }
}

impl Read for CtByteStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

impl std::fmt::Debug for CtByteStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CtByteStream").finish_non_exhaustive()
    }
}

/// 管线顶层数据容器
///
/// 各变体含义：
/// - `Empty`：管线无数据（相当于 `Nothing`，无元信息）
/// - `Value`：单个具象值（最常见情形）
/// - `ListStream`：惰性值流（来自前一个命令或列表展开）
/// - `ByteStream`：原始字节流（来自外部命令或文件读取）
#[derive(Debug, Default)]
pub enum CtPipelineData {
    /// 无数据（管线起始或 `nothing` 命令输出）
    #[default]
    Empty,
    /// 单个值
    Value(CtValue, CtPipelineMetadata),
    /// 惰性列表流
    ListStream(CtListStream),
    /// 原始字节流
    ByteStream(CtByteStream),
}

impl CtPipelineData {
    /// 消费 `ListStream` 将其收集为具象列表；其他变体原样包装
    pub fn collect_values(self) -> CtPipelineData {
        match self {
            CtPipelineData::ListStream(stream) => {
                let meta = stream.metadata.clone();
                let values: Vec<CtValue> = stream.collect();
                CtPipelineData::Value(CtValue::List(values), meta)
            }
            other => other,
        }
    }

    /// 是否为空管线
    pub fn is_empty(&self) -> bool {
        matches!(self, CtPipelineData::Empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{CtDataSource, CtPipelineMetadata};
    use std::io::Cursor;

    #[test]
    fn test_ctpipelinedata_empty() {
        let data = CtPipelineData::Empty;
        assert!(data.is_empty());
    }

    #[test]
    fn test_ctpipelinedata_value() {
        let meta = CtPipelineMetadata::default();
        let data = CtPipelineData::Value(CtValue::Int(10), meta);
        assert!(!data.is_empty());
    }

    #[test]
    fn test_list_stream_iter() {
        let values = vec![CtValue::Int(1), CtValue::Int(2), CtValue::Int(3)];
        let meta = CtPipelineMetadata::from_source(CtDataSource::Pipeline);
        let stream = CtListStream::new(values.into_iter(), meta);
        let collected: Vec<_> = stream.collect();
        assert_eq!(collected.len(), 3);
        assert!(matches!(collected[0], CtValue::Int(1)));
    }

    #[test]
    fn test_list_stream_collect_values() {
        let values = vec![CtValue::Bool(true), CtValue::Bool(false)];
        let meta = CtPipelineMetadata::default();
        let stream = CtListStream::new(values.into_iter(), meta);
        let data = CtPipelineData::ListStream(stream).collect_values();
        if let CtPipelineData::Value(CtValue::List(items), _) = data {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected Value(List(...))");
        }
    }

    #[test]
    fn test_byte_stream_read() {
        let bytes = b"hello world";
        let cursor = Cursor::new(bytes.to_vec());
        let meta = CtPipelineMetadata::default();
        let mut stream = CtByteStream::new(cursor, meta);
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, b"hello world");
    }

    #[test]
    fn test_ctpipelinedata_default() {
        let data = CtPipelineData::default();
        assert!(data.is_empty());
    }
}
