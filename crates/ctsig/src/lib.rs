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

//! `ctsig` — syskits 数据管线命令签名库。
//!
//! 提供：
//! - [`signature`]：命令签名 `DataSignature`、位置参数 `CtPositionalArg`、标志参数 `CtFlag`
//! - [`call`]：调用绑定 `DataCall`、参数提取 trait `TryFromCtValue`

pub mod call;
pub mod signature;

pub use call::{BoundArg, CallError, DataCall, TryFromCtValue};
pub use signature::{CtFlag, CtPositionalArg, DataSignature};
