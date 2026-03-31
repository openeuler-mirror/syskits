/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use crate::conversion_tables::*;

type Cbs = usize;

/// How to apply conversion, blocking, and/or unblocking.
///
/// Certain settings of the `conv` parameter to `dd` require a
/// combination of conversion, blocking, or unblocking, applied in a
/// certain order. The variants of this enumeration give the different
/// ways of combining those three operations.
#[derive(Debug, PartialEq)]
pub(crate) enum ConversionMode {
    ConvertOnly(&'static ConversionTable),
    BlockOnly(Cbs, bool),
    UnblockOnly(Cbs),
    BlockThenConvert(&'static ConversionTable, Cbs, bool),
    ConvertThenBlock(&'static ConversionTable, Cbs, bool),
    UnblockThenConvert(&'static ConversionTable, Cbs),
    ConvertThenUnblock(&'static ConversionTable, Cbs),
}

/// Stores all Conv Flags that apply to the input
#[derive(Debug, Default, PartialEq)]
pub(crate) struct IConvFlags {
    pub mode: Option<ConversionMode>,
    pub swab: bool,
    pub sync: Option<u8>,
    pub noerror: bool,
}

/// Stores all Conv Flags that apply to the output
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OConvFlags {
    pub sparse: bool,
    pub excl: bool,
    pub nocreat: bool,
    pub notrunc: bool,
    pub fdatasync: bool,
    pub fsync: bool,
}

/// Stores all Flags that apply to the input
#[derive(Debug, Default, PartialEq, Eq)]
pub struct IFlags {
    pub cio: bool,
    pub direct: bool,
    pub directory: bool,
    pub dsync: bool,
    pub sync: bool,
    pub nocache: bool,
    pub nonblock: bool,
    pub noatime: bool,
    pub noctty: bool,
    pub nofollow: bool,
    pub nolinks: bool,
    pub binary: bool,
    pub text: bool,
    pub fullblock: bool,
    pub count_bytes: bool,
    pub skip_bytes: bool,
}

/// Stores all Flags that apply to the output
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OFlags {
    pub append: bool,
    pub cio: bool,
    pub direct: bool,
    pub directory: bool,
    pub dsync: bool,
    pub sync: bool,
    pub nocache: bool,
    pub nonblock: bool,
    pub noatime: bool,
    pub noctty: bool,
    pub nofollow: bool,
    pub nolinks: bool,
    pub binary: bool,
    pub text: bool,
    pub seek_bytes: bool,
}

pub mod options {
    pub const OPERANDS: &str = "operands";
}
