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
use ctcore::ct_fs;
use std::io::Write;
use std::io::{BufWriter, Error, ErrorKind, Result};
use std::path::Path;

/// Get a file writer
///
/// Unlike the unix version of this function, this _always_ returns
/// a file writer
pub fn instantiate_current_writer(
    _filter: &Option<String>,
    file_name: &str,
    is_new: bool,
) -> Result<BufWriter<Box<dyn Write>>> {
    let file = if is_new {
        // 创建新文件
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(std::path::Path::new(&file_name))
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    format!("unable to open '{file_name}'; aborting"),
                )
            })?
    } else {
        // 重新打开之前创建的文件以便追加写入
        std::fs::OpenOptions::new()
            .append(true)
            .open(std::path::Path::new(&file_name))
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    format!("unable to re-open '{file_name}'; aborting"),
                )
            })?
    };
    Ok(BufWriter::new(Box::new(file) as Box<dyn Write>))
}

pub fn paths_refer_to_same_file(p1: &str, p2: &str) -> bool {
    ct_fs::paths_refer_to_same_file(Path::new(p1), Path::new(p2), true)
}
