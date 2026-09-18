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
use crate::split_quote_path;
use ctcore::ct_error::CTError;
use ctcore::ct_fs;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::io::{BufWriter, Error, ErrorKind};
use std::path::Path;

pub fn reset_filter_failure() {}

pub fn filter_failure_recorded() -> bool {
    false
}

pub fn take_filter_failure() -> Option<Box<dyn CTError>> {
    None
}

pub fn reset_output_failure() {}

pub fn take_output_failure() -> Option<Box<dyn CTError>> {
    None
}

/// Get a file writer
///
/// Unlike the unix version of this function, this _always_ returns
/// a file writer
pub fn instantiate_current_writer(
    _filter: &Option<OsString>,
    file_name: impl AsRef<OsStr>,
    is_new: bool,
) -> Result<BufWriter<Box<dyn Write>>> {
    let file_name = file_name.as_ref();
    let file = if is_new {
        // 创建新文件
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(std::path::Path::new(file_name))
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    format!(
                        "unable to open '{}'; aborting",
                        split_quote_path(file_name, false)
                    ),
                )
            })?
    } else {
        // 重新打开之前创建的文件以便追加写入
        std::fs::OpenOptions::new()
            .append(true)
            .open(std::path::Path::new(file_name))
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    format!(
                        "unable to re-open '{}'; aborting",
                        split_quote_path(file_name, false)
                    ),
                )
            })?
    };
    Ok(BufWriter::new(Box::new(file) as Box<dyn Write>))
}

pub fn paths_refer_to_same_file(p1: impl AsRef<OsStr>, p2: impl AsRef<OsStr>) -> bool {
    let p1 = p1.as_ref();
    let p2 = p2.as_ref();
    ct_fs::paths_refer_to_same_file(Path::new(p1), Path::new(p2), true)
}
