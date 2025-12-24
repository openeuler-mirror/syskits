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
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tempfile::TempDir;

use ctcore::{
    ct_error::{CTResult, CtSimpleError},
    ct_show_error,
};

use crate::SortError;

/// TempDir 的封装器，在一个进程中可能只存在一次。
///
/// `TmpDirWrapper`负责在此临时目录中分配新的临时文件，并在收到 `SIGINT` 时删除整个目录。
/// 当收到 `SIGINT` 时删除整个目录。创建第二个 `TmpDirWrapper` 将会
/// 失败，因为当已有处理程序时，`ctrlc::set_handler()` 会失败。
/// 只有在请求第一个文件后才会创建目录。
pub struct TmpDirWrapper {
    temp_dir: Option<TempDir>,
    parent_path: PathBuf,
    size: usize,
    lock: Arc<Mutex<()>>,
}

impl TmpDirWrapper {
    pub fn new(path: PathBuf) -> Self {
        Self {
            parent_path: path,
            size: 0,
            temp_dir: None,
            lock: Arc::default(),
        }
    }

    fn init_tmp_dir(&mut self) -> CTResult<()> {
        assert!(self.temp_dir.is_none());
        assert_eq!(self.size, 0);
        self.temp_dir = Some(
            tempfile::Builder::new()
                .prefix("cttils_sort")
                .tempdir_in(&self.parent_path)
                .map_err(|_| SortError::SortTmpDirCreationFailed)?,
        );

        let tmp_dir = self.temp_dir.as_ref().unwrap();
        let path_buf = tmp_dir.path().to_owned();
        let lock = self.lock.clone();
        ctrlc::set_handler(move || {
            // 加锁，这样 `next_file_path` 就不会返回新的文件路径、
            // 并且程序不会在处理程序结束前终止
            let _lock = lock.lock().unwrap();
            if let Err(e) = tmp_dir_remove_tmp_dir(&path_buf) {
                ct_show_error!("failed to delete temporary directory: {}", e);
            }
            std::process::exit(2)
        })
        .map_err(|e| CtSimpleError::new(2, format!("failed to set up signal handler: {e}")))
    }

    pub fn next_file(&mut self) -> CTResult<(File, PathBuf)> {
        if self.temp_dir.is_none() {
            self.init_tmp_dir()?;
        }

        let _lock = self.lock.lock().unwrap();
        let file_name_string = self.size.to_string();
        self.size += 1;
        let tmp_dir = self.temp_dir.as_ref().unwrap();
        let path = tmp_dir.path().join(file_name_string);
        Ok((
            File::create(&path).map_err(|error| SortError::SortOpenTmpFileFailed { error })?,
            path,
        ))
    }

    /// 如果信号处理器被调用，函数只是等待
    pub fn wait_if_signal(&self) {
        let _lock = self.lock.lock().unwrap();
    }
}

/// 删除位于 `path` 的目录，先删除其子文件，然后再删除自身。
/// 删除子文件时的错误将被忽略。
fn tmp_dir_remove_tmp_dir(path: &Path) -> std::io::Result<()> {
    if let Ok(read_dir) = std::fs::read_dir(path) {
        for file in read_dir.flatten() {
            // 如果我们没能在这里删除文件，它可能已经被另一个线程删除了。
            // 在此期间被另一个线程删除了，不过没关系。
            let _ = std::fs::remove_file(file.path());
        }
    }
    std::fs::remove_dir(path)
}

