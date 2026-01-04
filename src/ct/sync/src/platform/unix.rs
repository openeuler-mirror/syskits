/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO NON-INFRINGEMENT,
 * MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

#[cfg(target_os = "linux")]
use nix::errno::Errno;
#[cfg(target_os = "linux")]
use nix::fcntl::{open, OFlag};
#[cfg(target_os = "linux")]
use nix::sys::stat::Mode;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::unix::io::AsRawFd;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, FromIo};
use std::path::Path;

#[cfg(target_os = "linux")]
pub unsafe fn do_sync() -> isize {
    libc::sync();
    0
}

#[cfg(target_os = "linux")]
pub unsafe fn do_syncfs(files: Vec<String>) -> isize {
    for path in files {
        let f = File::open(path).unwrap();
        let fd = f.as_raw_fd();
        libc::syscall(libc::SYS_syncfs, fd);
    }
    0
}

#[cfg(target_os = "linux")]
pub unsafe fn do_fdatasync(files: Vec<String>) -> isize {
    for path in files {
        let f = File::open(path).unwrap();
        let fd = f.as_raw_fd();
        libc::syscall(libc::SYS_fdatasync, fd);
    }
    0
}

// 使用 Nix 打开文件，以便为 FIFO 文件设置 NONBLOCK 标志
#[cfg(target_os = "linux")]
pub fn check_files(f: &String) -> CTResult<()> {
    let path = Path::new(&f);
    if let Err(e) = open(path, OFlag::O_NONBLOCK, Mode::empty()) {
        if e != Errno::EACCES || (e == Errno::EACCES && path.is_dir()) {
            e.map_err_context(|| format!("error opening {}", f.quote()))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn fdatasync(files: Vec<String>) -> isize {
    unsafe { do_fdatasync(files) }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(target_os = "linux")]
    #[test]
    fn test_check_files_directory() {
        let dir_path = "/tmp/testdir";
        fs::create_dir(dir_path).unwrap();
        let result = check_files(&dir_path.to_string());
        assert!(result.is_ok());
        fs::remove_dir(dir_path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_check_files_permission_denied() {
        let file_path = "/root/testfile"; // Assuming the test is run by a non-root user
        let result = check_files(&file_path.to_string());
        assert!(result.is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_do_fdatasync() {
        let file_path = "/tmp/testfile";
        File::create(file_path).unwrap();
        let result = unsafe { do_fdatasync(vec![file_path.to_string()]) };
        assert_eq!(result, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_do_sync() {
        let result = unsafe { do_sync() };
        assert_eq!(result, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_do_syncfs() {
        let file_path = "/tmp/testfile";
        File::create(file_path).unwrap();
        let result = unsafe { do_syncfs(vec![file_path.to_string()]) };
        assert_eq!(result, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_check_files() {
        let file_path = "/tmp/testfile";
        File::create(file_path).unwrap();
        let result = check_files(&file_path.to_string());
        assert!(result.is_ok());

        // Test with non-existing file
        let result = check_files(&"/tmp/non_existing_file".to_string());
        assert!(result.is_err());
    }
}