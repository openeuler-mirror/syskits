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
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::CtSimpleError;
use ctcore::{ct_error::CTResult, ct_show_error};
use tempfile::TempDir;

use crate::SortError;

/// TempDir 的封装器，在一个进程中可能只存在一次。
///
/// `TmpDirWrapper`负责在此临时目录中分配新的临时文件，并在收到 `SIGINT` 时删除整个目录。
/// 当收到 `SIGINT` 时删除整个目录。创建第二个 `TmpDirWrapper` 将会
/// 失败，因为当已有处理程序时，`ctrlc::set_handler()` 会失败。
/// 只有在请求第一个文件后才会创建目录。
pub struct TmpDirWrapper {
    temp_dirs: Vec<Option<TempDir>>,
    parent_paths: Vec<PathBuf>,
    next_parent: usize,
    size: usize,
    lock: Arc<Mutex<()>>,
    cleanup_paths: Arc<Mutex<Vec<PathBuf>>>,
}

impl TmpDirWrapper {
    pub fn new(path: PathBuf) -> Self {
        Self::new_many(vec![path])
    }

    pub fn new_many(parent_paths: Vec<PathBuf>) -> Self {
        assert!(!parent_paths.is_empty());
        Self {
            temp_dirs: parent_paths.iter().map(|_| None).collect(),
            parent_paths,
            next_parent: 0,
            size: 0,
            lock: Arc::default(),
            cleanup_paths: Arc::default(),
        }
    }

    fn init_tmp_dir(&mut self, index: usize) -> CTResult<()> {
        assert!(self.temp_dirs[index].is_none());
        self.temp_dirs[index] = Some(
            tempfile::Builder::new()
                .prefix("cttils_sort")
                .tempdir_in(&self.parent_paths[index])
                .map_err(|_| SortError::SortTmpDirCreationFailed)?,
        );

        let tmp_dir = self.temp_dirs[index].as_ref().unwrap();
        let path_buf = tmp_dir.path().to_owned();
        self.cleanup_paths.lock().unwrap().push(path_buf);
        let lock = self.lock.clone();
        let cleanup_paths = self.cleanup_paths.clone();
        if let Err(e) = ctrlc::set_handler(move || {
            // 加锁，这样 `next_file_path` 就不会返回新的文件路径、
            // 并且程序不会在处理程序结束前终止
            let _lock = lock.lock().unwrap();
            for path in cleanup_paths.lock().unwrap().iter() {
                if let Err(e) = tmp_dir_remove_tmp_dir(path) {
                    ct_show_error!("failed to delete temporary directory: {}", e);
                }
            }
            std::process::exit(2)
        }) {
            if !matches!(e, ctrlc::Error::MultipleHandlers) {
                return Err(CtSimpleError::new(
                    2,
                    format!("failed to set up signal handler: {e}"),
                ));
            }
        }
        Ok(())
    }

    pub fn next_file(&mut self) -> CTResult<(File, PathBuf)> {
        let index = self.next_parent;
        self.next_parent = (self.next_parent + 1) % self.parent_paths.len();
        if self.temp_dirs[index].is_none() {
            self.init_tmp_dir(index)?;
        }

        let _lock = self.lock.lock().unwrap();
        let file_name_string = self.size.to_string();
        self.size += 1;
        let tmp_dir = self.temp_dirs[index].as_ref().unwrap();
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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    mod tmp_dir_wrapper_tests {
        use std::path::PathBuf;

        use super::*;

        #[test]
        fn test_tmp_dir_wrapper_new() {
            let path = PathBuf::from("/tmp");
            let wrapper = TmpDirWrapper::new(path.clone());
            assert_eq!(wrapper.parent_paths, vec![path]);
            assert_eq!(wrapper.size, 0);
            assert!(wrapper.temp_dirs.iter().all(Option::is_none));
            // test that lock is initialized
            assert!(wrapper.lock.lock().is_ok());
        }

        #[test]
        fn test_tmp_dir_wrapper_next_file() {
            let mut wrapper = TmpDirWrapper::new(PathBuf::from("/tmp"));
            let _ = wrapper.init_tmp_dir(0);

            let (_file1, path1) = wrapper.next_file().unwrap();
            let (_file2, path2) = wrapper.next_file().unwrap();

            assert_ne!(path1, path2);
            assert!(!path1.starts_with("/tmp/cttils_sort"));
            assert!(!path2.starts_with("/tmp/cttils_sort"));
        }

        #[test]
        fn test_tmp_dir_wrapper_rotates_parent_paths() {
            let first = tempdir().unwrap();
            let second = tempdir().unwrap();
            let first_path = first.path().to_owned();
            let second_path = second.path().to_owned();
            let mut wrapper =
                TmpDirWrapper::new_many(vec![first_path.clone(), second_path.clone()]);

            let (_file1, path1) = wrapper.next_file().unwrap();
            let (_file2, path2) = wrapper.next_file().unwrap();
            let (_file3, path3) = wrapper.next_file().unwrap();

            assert!(path1.starts_with(&first_path));
            assert!(path2.starts_with(&second_path));
            assert!(path3.starts_with(&first_path));
        }

        #[test]
        fn test_tmp_dir_wrapper_wait_if_signal() {
            let wrapper = TmpDirWrapper::new(PathBuf::from("/tmp"));
            wrapper.wait_if_signal();
            // test that lock is acquired and released
            assert!(wrapper.lock.lock().is_ok());
        }
    }

    #[test]
    fn test_remove_tmp_dir_base() {
        let test_dir = tempdir().expect("Failed to create temporary directory");

        let result = tmp_dir_remove_tmp_dir(test_dir.path());

        assert!(result.is_ok(), "remove_tmp_dir failed: {result:?}");
        assert!(
            !test_dir.path().exists(),
            "Temporary directory still exists after removal"
        );
    }

    #[test]
    fn test_remove_tmp_dir_with_files() {
        let test_dir = tempdir().expect("Failed to create temporary directory");

        let file1 = test_dir.path().join("file1.txt");
        let file2 = test_dir.path().join("file2.txt");
        fs::File::create(&file1).expect("Failed to create file1");
        fs::File::create(&file2).expect("Failed to create file2");

        let result = tmp_dir_remove_tmp_dir(test_dir.path());

        assert!(result.is_ok(), "remove_tmp_dir failed: {result:?}");
        assert!(
            !test_dir.path().exists(),
            "Temporary directory still exists after removal"
        );
        assert!(
            !file1.exists(),
            "File1 still exists after removal of temporary directory"
        );
        assert!(
            !file2.exists(),
            "File2 still exists after removal of temporary directory"
        );
    }

    #[test]
    fn test_remove_tmp_dir_with_subdirs() {
        let test_dir = tempdir().expect("Failed to create temporary directory");

        let subdir1 = test_dir.path().join("subdir1");
        let subdir2 = test_dir.path().join("subdir2");
        fs::create_dir(&subdir1).expect("Failed to create subdir1");
        fs::create_dir(&subdir2).expect("Failed to create subdir2");

        let file1 = subdir1.as_path().join("file1.txt");
        fs::File::create(&file1).expect("Failed to create file1");
        let _result1 = tmp_dir_remove_tmp_dir(subdir1.as_path());
        let _result2 = tmp_dir_remove_tmp_dir(subdir2.as_path());

        let result = tmp_dir_remove_tmp_dir(test_dir.path());

        assert!(result.is_ok(), "remove_tmp_dir failed: {result:?}");
        assert!(
            !test_dir.path().exists(),
            "Temporary directory still exists after removal"
        );
        assert!(
            !subdir1.exists(),
            "Subdir1 still exists after removal of temporary directory"
        );
        assert!(
            !subdir2.exists(),
            "Subdir2 still exists after removal of temporary directory"
        );
    }

    #[test]
    fn test_remove_tmp_dir_with_nothing() {
        // Call remove_tmp_dir with a non-existent directory path
        let result = tmp_dir_remove_tmp_dir(Path::new("/non/existent/directory"));

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }
}
