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
use std::ffi::{OsStr, OsString};
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};

use ctcore::ct_display::Quotable;
#[cfg(all(test, target_os = "linux"))]
use ctcore::ct_error::CtSimpleError;
use ctcore::ct_error::{CTError, CTResult, FromIo};

#[cfg(target_os = "linux")]
pub unsafe fn do_sync() -> isize {
    unsafe {
        libc::sync();
        0
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum SyncOperation {
    File,
    Data,
    FileSystem,
}

#[cfg(target_os = "linux")]
fn open_sync_file(path: &OsStr) -> CTResult<File> {
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
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(target_os = "linux")]
fn reset_nonblocking_mode(file: &File, path: &OsStr) -> CTResult<()> {
    let flags = fcntl(file.as_raw_fd(), FcntlArg::F_GETFL)
        .map(OFlag::from_bits_truncate)
        .map_err_context(|| format!("couldn't reset non-blocking mode {}", path.quote()))?;
    let mut blocking_flags = flags;
    blocking_flags.remove(OFlag::O_NONBLOCK);
    fcntl(file.as_raw_fd(), FcntlArg::F_SETFL(blocking_flags))
        .map_err_context(|| format!("couldn't reset non-blocking mode {}", path.quote()))?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn close_sync_file(file: File, path: &OsStr) -> CTResult<()> {
    let fd = file.into_raw_fd();
    if unsafe { libc::close(fd) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
            .map_err_context(|| format!("failed to close {}", path.quote()))
    }
}

#[cfg(target_os = "linux")]
fn sync_paths(files: &[OsString], operation: SyncOperation) -> CTResult<()> {
    sync_all_paths(files, |path| sync_path(path, operation))
}

#[cfg(target_os = "linux")]
fn sync_all_paths<F>(files: &[OsString], mut sync_path: F) -> CTResult<()>
where
    F: FnMut(&OsStr) -> Vec<Box<dyn CTError>>,
{
    let mut failed = false;

    for path in files {
        for error in sync_path(path) {
            ctcore::ct_show!(error);
            failed = true;
        }
    }

    if failed {
        ctcore::ct_error::set_ct_exit_code(1);
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn sync_path(path: &OsStr, operation: SyncOperation) -> Vec<Box<dyn CTError>> {
    let file = match open_sync_file(path) {
        Ok(file) => file,
        Err(error) => return vec![error],
    };
    let mut errors: Vec<Box<dyn CTError>> = Vec::new();

    match reset_nonblocking_mode(&file, path) {
        Ok(()) => {
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
            if let Err(error) = result {
                errors.push(error.map_err_context(|| format!("error syncing {}", path.quote())));
            }
        }
        Err(error) => errors.push(error),
    }

    if let Err(error) = close_sync_file(file, path) {
        errors.push(error);
    }

    errors
}

#[cfg(target_os = "linux")]
pub fn sync_files(files: &[OsString]) -> CTResult<()> {
    sync_paths(files, SyncOperation::File)
}

#[cfg(target_os = "linux")]
pub fn sync_data(files: &[OsString]) -> CTResult<()> {
    sync_paths(files, SyncOperation::Data)
}

#[cfg(target_os = "linux")]
pub fn sync_file_systems(files: &[OsString]) -> CTResult<()> {
    sync_paths(files, SyncOperation::FileSystem)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::os::fd::AsRawFd;
    use std::sync::Mutex;

    #[cfg(target_os = "linux")]
    static EXIT_CODE_LOCK: Mutex<()> = Mutex::new(());

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
        let files = vec![file_path.into_os_string()];

        assert!(sync_files(&files).is_ok());
        assert!(sync_data(&files).is_ok());
        assert!(sync_file_systems(&files).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_close_sync_file_reports_close_failure() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("file");
        let file = File::create(&file_path).unwrap();

        assert_eq!(unsafe { libc::close(file.as_raw_fd()) }, 0);

        let error = close_sync_file(file, file_path.as_os_str()).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("failed to close {}: Bad file descriptor", file_path.quote())
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_sync_missing_file() {
        let _exit_code_guard = EXIT_CODE_LOCK.lock().unwrap();
        ctcore::ct_error::set_ct_exit_code(0);
        let temp_dir = tempfile::tempdir().unwrap();
        let missing = temp_dir.path().join("missing").into_os_string();

        assert!(sync_files(&[missing]).is_ok());
        assert_eq!(ctcore::ct_error::get_ct_exit_code(), 1);
        ctcore::ct_error::set_ct_exit_code(0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_sync_all_paths_reports_each_error_without_aborting() {
        let _exit_code_guard = EXIT_CODE_LOCK.lock().unwrap();
        ctcore::ct_error::set_ct_exit_code(0);
        let files = vec![OsString::from("missing-one"), OsString::from("missing-two")];
        let mut calls = Vec::new();

        let result = sync_all_paths(&files, |path| {
            calls.push(path.to_os_string());
            vec![CtSimpleError::new(1, "sync failed")]
        });

        assert!(result.is_ok());
        assert_eq!(
            calls,
            [OsString::from("missing-one"), OsString::from("missing-two")]
        );
        assert_eq!(ctcore::ct_error::get_ct_exit_code(), 1);
        ctcore::ct_error::set_ct_exit_code(0);
    }
}
