/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */
use std::fs;
use std::path::Path;

use quick_error::ResultExt;

use crate::{
    CopyDebug, CopyResult, CpOffloadReflinkDebug, CpReflinkMode, CpSparseDebug, CpSparseMode,
};

/// Copies `source` to `dest` for systems without copy-on-write
pub(crate) fn copy_on_write(
    source: &Path,
    dest: &Path,
    reflink_mode: CpReflinkMode,
    sparse_mode: CpSparseMode,
    context: &str,
) -> CopyResult<CopyDebug> {
    if reflink_mode != CpReflinkMode::Never {
        return Err("--reflink is only supported on linux and macOS"
            .to_string()
            .into());
    }
    if sparse_mode != CpSparseMode::Auto {
        return Err("--sparse is only supported on linux".to_string().into());
    }
    let copy_debug = CopyDebug {
        offload: CpOffloadReflinkDebug::Unsupported,
        reflink: CpOffloadReflinkDebug::Unsupported,
        sparse_detection: CpSparseDebug::Unsupported,
    };
    fs::copy(source, dest).context(context)?;

    Ok(copy_debug)
}

