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

#[cfg(target_os = "linux")]
use nix::fcntl::{FcntlArg, OFlag, fcntl, open};
#[cfg(target_os = "linux")]
use nix::sys::stat::Mode;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};

use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, FromIo};

#[cfg(target_os = "linux")]
pub unsafe fn do_sync() -> isize {
    unsafe {
        libc::sync();
        0
    }
}

#[cfg(target_os = "linux")]
enum SyncOperation {
    File,
    Data,
    FileSystem,
}

#[cfg(target_os = "linux")]
fn open_sync_file(path: &str) -> CTResult<File> {
    let read_flags = OFlag::O_RDONLY | OFlag::O_NONBLOCK;
    let write_flags = OFlag::O_WRONLY | OFlag::O_NONBLOCK;
    let fd = match open(path, read_flags, Mode::empty()) {
        Ok(fd) => fd,
        Err(read_error) => match open(path, write_flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(_) => {
                return Err(read_error)
                    .map_err_context(|| format!("error opening {}", path.quote()));
            }
        },
    };
    let file = unsafe { File::from_raw_fd(fd) };

    let flags = fcntl(file.as_raw_fd(), FcntlArg::F_GETFL)
        .map(OFlag::from_bits_truncate)
        .map_err_context(|| format!("couldn't reset non-blocking mode {}", path.quote()))?;
    let mut blocking_flags = flags;
    blocking_flags.remove(OFlag::O_NONBLOCK);
    fcntl(file.as_raw_fd(), FcntlArg::F_SETFL(blocking_flags))
        .map_err_context(|| format!("couldn't reset non-blocking mode {}", path.quote()))?;

    Ok(file)
}

#[cfg(target_os = "linux")]
fn sync_paths(files: &[String], operation: SyncOperation) -> CTResult<()> {
    for path in files {
        let file = open_sync_file(path)?;
        let result = match operation {
            SyncOperation::File => file.sync_all(),
            SyncOperation::Data => file.sync_data(),
            SyncOperation::FileSystem => {
                if unsafe { libc::syncfs(file.as_raw_fd()) } == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            }
        };
        result.map_err_context(|| format!("error syncing {}", path.quote()))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn sync_files(files: &[String]) -> CTResult<()> {
    sync_paths(files, SyncOperation::File)
}

#[cfg(target_os = "linux")]
pub fn sync_data(files: &[String]) -> CTResult<()> {
    sync_paths(files, SyncOperation::Data)
}

#[cfg(target_os = "linux")]
pub fn sync_file_systems(files: &[String]) -> CTResult<()> {
    sync_paths(files, SyncOperation::FileSystem)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn test_do_sync() {
        let result = unsafe { do_sync() };
        assert_eq!(result, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_file_sync_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("file");
        File::create(&file_path).unwrap();
        let files = vec![file_path.to_string_lossy().into_owned()];

        assert!(sync_files(&files).is_ok());
        assert!(sync_data(&files).is_ok());
        assert!(sync_file_systems(&files).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_sync_missing_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let missing = temp_dir
            .path()
            .join("missing")
            .to_string_lossy()
            .into_owned();

        assert!(sync_files(&[missing]).is_err());
    }
}
