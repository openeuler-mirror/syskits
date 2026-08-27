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

//! `ctpipeline` — syskits 数据管线核心数据模型库。
//!
//! 提供：
//! - [`value`]：核心值类型 `CtValue`、类型标签 `CtType`、错误 `CtValueError`
//! - [`span`]：源码位置标注 `CtSpan`、`CtSourceRef`
//! - [`metadata`]：元信息 `CtPipelineMetadata`、`CtDataSource`
//! - [`pipeline_data`]：顶层管线数据 `CtPipelineData`、`CtListStream`、`CtByteStream`

pub mod metadata;
pub mod pipeline_data;
pub mod span;
pub mod stream_producer;
pub mod value;

// 顶层重导出，方便调用方直接 `use ctpipeline::*`
pub use metadata::{CtDataSource, CtPipelineMetadata, CtSchemaHint};
pub use pipeline_data::{CtByteStream, CtListStream, CtPipelineData};
pub use span::{CtSourceRef, CtSpan};
pub use stream_producer::{CancelFlag, CtByteStreamWithProducer, new_cancel_flag};
pub use value::{CtType, CtValue, CtValueError};
