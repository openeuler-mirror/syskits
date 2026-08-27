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

//! `CtPipelineMetadata` — 管线数据的元信息容器。
//!
//! M1a 阶段为骨架实现，`CtSchemaHint` 为不透明占位类型，
//! M1b/M1c 阶段再扩展具体字段。

use crate::CtValue;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// 数据来源描述
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CtDataSource {
    /// 未指定来源
    None,
    /// 来自 syskits 内建命令
    BuiltinCommand(String),
    /// 来自外部进程
    ExternalCommand { command: String },
    /// 来自文件读取
    FilePath(PathBuf),
    /// 来自插件
    Plugin(String),
    /// 来自管线拼接（前一个命令的输出）
    Pipeline,
}

/// Schema 提示（M1a 为不透明占位，M1b 扩充）
#[derive(Debug, Clone)]
pub struct CtSchemaHint {
    _opaque: (),
}

impl CtSchemaHint {
    /// 创建空 schema（无类型信息）
    pub fn none() -> Self {
        Self { _opaque: () }
    }
}

/// 管线元信息，随 `CtPipelineData` 一同传递
#[derive(Debug, Clone)]
pub struct CtPipelineMetadata {
    /// 数据来源
    pub data_source: CtDataSource,
    /// 内容类型提示（如 application/json、text/plain）
    pub content_type: Option<String>,
    /// 可选的 schema 提示（用于格式化和类型推导）
    pub schema_hint: Option<CtSchemaHint>,
    /// classic 渲染专用文本。该字段不是 native text alias。
    pub classic_text: Option<String>,
    /// classic 渲染专用原始字节，用于 base64/base32/basenc decode 等非 UTF-8 输出。
    pub classic_bytes: Option<Vec<u8>>,
    /// classic_text 是否需要补尾随换行。
    pub classic_append_newline: bool,
    /// 命令语义层捕获的 stderr 文本。
    pub stderr_text: Option<String>,
    /// 命令语义层捕获的退出码。
    pub exit_code: i32,
    /// 兼容旧命令适配器的来源标签。
    pub source: Option<String>,
    /// 用户自定义 KV 扩展字段（供插件/桥接使用）
    pub custom: Arc<Mutex<BTreeMap<String, CtValue>>>,
}

impl CtPipelineMetadata {
    /// 创建最简元信息（仅指定来源）
    pub fn from_source(source: CtDataSource) -> Self {
        Self {
            data_source: source,
            content_type: None,
            schema_hint: None,
            classic_text: None,
            classic_bytes: None,
            classic_append_newline: true,
            stderr_text: None,
            exit_code: 0,
            source: None,
            custom: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// 创建来自内建命令的元信息
    pub fn builtin(name: &str) -> Self {
        Self::from_source(CtDataSource::BuiltinCommand(name.to_string()))
    }
}

impl Default for CtPipelineMetadata {
    fn default() -> Self {
        Self::from_source(CtDataSource::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_custom_insert() {
        let meta = CtPipelineMetadata::default();
        meta.custom
            .lock()
            .unwrap()
            .insert("format".to_string(), CtValue::String("json".to_string()));
        assert!(matches!(
            meta.custom.lock().unwrap().get("format"),
            Some(CtValue::String(s)) if s == "json"
        ));
    }

    #[test]
    fn test_metadata_builtin() {
        let meta = CtPipelineMetadata::builtin("ls");
        assert!(matches!(
            &meta.data_source,
            CtDataSource::BuiltinCommand(name) if name == "ls"
        ));
        assert!(meta.content_type.is_none());
        assert!(meta.schema_hint.is_none());
        assert!(meta.custom.lock().unwrap().is_empty());
    }

    #[test]
    fn test_schema_hint_none() {
        let hint = CtSchemaHint::none();
        // M1a 仅验证可构造，无 panic
        let _ = hint;
    }

    #[test]
    fn test_metadata_from_source_pipeline() {
        let meta = CtPipelineMetadata::from_source(CtDataSource::Pipeline);
        assert_eq!(meta.data_source, CtDataSource::Pipeline);
    }
}
