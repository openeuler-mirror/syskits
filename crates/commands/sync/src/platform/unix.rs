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

#[cfg(test)]
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError};
#[cfg(target_os = "linux")]
use ctcore::ct_quoting_style::escape_shell_bytes_with_classifier;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;

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
unsafe extern "C" {
    fn mbrtowc(
        wide: *mut libc::wchar_t,
        bytes: *const libc::c_char,
        length: usize,
        state: *mut libc::mbstate_t,
    ) -> usize;
    fn iswprint(wide: libc::c_uint) -> libc::c_int;
}

#[cfg(target_os = "linux")]
fn sync_quote_path(path: &OsStr) -> String {
    let bytes = path.as_bytes();
    let mut quoted = escape_shell_bytes_with_classifier(bytes, |remaining| unsafe {
        let mut state: libc::mbstate_t = std::mem::zeroed();
        let mut wide = 0 as libc::wchar_t;
        let length = mbrtowc(
            &mut wide,
            remaining.as_ptr().cast(),
            remaining.len(),
            &mut state,
        );
        if length == usize::MAX {
            return (1, false);
        }
        if length == usize::MAX - 1 {
            return (remaining.len(), false);
        }

        let length = if length == 0 { 1 } else { length };
        let is_utf8 = std::str::from_utf8(&remaining[..length]).is_ok();
        (length, is_utf8 && iswprint(wide as libc::c_uint) != 0)
    });

    if quoted.as_slice() == bytes {
        quoted.insert(0, b'\'');
        quoted.push(b'\'');
    }

    String::from_utf8(quoted).expect("shell-escaped file names are valid UTF-8")
}

#[cfg(target_os = "linux")]
fn sync_errno_text(error: &std::io::Error) -> String {
    if error.raw_os_error().is_some() {
        ctcore::ct_error::strip_errno(error)
    } else {
        error.to_string()
    }
}

#[cfg(target_os = "linux")]
fn sync_io_error(error: std::io::Error, context: String) -> Box<dyn CTError> {
    CtSimpleError::new(1, format!("{context}: {}", sync_errno_text(&error)))
}

#[cfg(target_os = "linux")]
fn sync_nix_error(error: nix::errno::Errno, context: String) -> Box<dyn CTError> {
    sync_io_error(std::io::Error::from_raw_os_error(error as i32), context)
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
                return Err(sync_nix_error(
                    read_error,
                    format!("error opening {}", sync_quote_path(path)),
                ));
            }
        },
    };
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(target_os = "linux")]
fn reset_nonblocking_mode(file: &File, path: &OsStr) -> CTResult<()> {
    let flags = fcntl(file.as_raw_fd(), FcntlArg::F_GETFL)
        .map(OFlag::from_bits_truncate)
        .map_err(|error| {
            sync_nix_error(
                error,
                format!("couldn't reset non-blocking mode {}", sync_quote_path(path)),
            )
        })?;
    let mut blocking_flags = flags;
    blocking_flags.remove(OFlag::O_NONBLOCK);
    fcntl(file.as_raw_fd(), FcntlArg::F_SETFL(blocking_flags)).map_err(|error| {
        sync_nix_error(
            error,
            format!("couldn't reset non-blocking mode {}", sync_quote_path(path)),
        )
    })?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn close_sync_file(file: File, path: &OsStr) -> CTResult<()> {
    let fd = file.into_raw_fd();
    if unsafe { libc::close(fd) } == 0 {
        Ok(())
    } else {
        Err(sync_io_error(
            std::io::Error::last_os_error(),
            format!("failed to close {}", sync_quote_path(path)),
        ))
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
                errors.push(sync_io_error(
                    error,
                    format!("error syncing {}", sync_quote_path(path)),
                ));
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
    fn test_sync_errno_text_preserves_linux_einval_message() {
        let error = std::io::Error::from_raw_os_error(libc::EINVAL);

        assert_eq!(sync_errno_text(&error), "Invalid argument");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_sync_quote_path_uses_gnu_shell_quote_for_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let path = OsString::from_vec(b"missing-\xff".to_vec());

        assert_eq!(sync_quote_path(&path), "'missing-'$'\\377'");
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
