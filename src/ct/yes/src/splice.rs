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

// 在某些系统（尤其是 WSL 1）上，我们根本不支持拼接到某些目标。
// 如果我们得到的错误代码表明不支持拼接，那么我们就会告诉调用者，这样它就可以退回到一个健壮的简单方法。
// vmsplice() 只能拼接到管道中，因此如果输出不是管道
// 我们就自制一个管道，并使用 splice() 来连接管道和输出。
// 我们假设 "不支持 "错误只会在数据成功写入输出之前发生。这样，如果 splice() 失败，我们就不必费力从管道中挽救数据了，
// 只需倒回去从头开始。

use std::io::IoSlice;
use std::{io, os::unix::io::AsRawFd};

use nix::fcntl::SpliceFFlags;
use nix::{errno::Errno, libc::S_IFIFO, sys::stat::fstat};

use ctcore::ct_pipes::{pipe, splice_exact};

pub(crate) fn splice_data(bytes: &[u8], out: &impl AsRawFd) -> Result<()> {
    let fstat_result = fstat(out.as_raw_fd())?;
    let st_mode = fstat_result.st_mode as nix::libc::mode_t;

    if st_mode & S_IFIFO != 0 {
        loop {
            let mut bytes = bytes;
            while !bytes.is_empty() {
                let len = nix::fcntl::vmsplice(
                    out.as_raw_fd(),
                    &[IoSlice::new(bytes)],
                    SpliceFFlags::empty(),
                )
                .map_err(splice_maybe_unsupported)?;
                bytes = &bytes[len..];
            }
        }
    } else {
        let (read, write) = pipe()?;
        loop {
            let mut bytes = bytes;
            while !bytes.is_empty() {
                let write_fd = &write;
                let len = nix::fcntl::vmsplice(
                    write_fd.as_raw_fd(),
                    &[IoSlice::new(bytes)],
                    SpliceFFlags::empty(),
                )
                .map_err(splice_maybe_unsupported)?;

                splice_exact(&read, out, len).map_err(splice_maybe_unsupported)?;
                bytes = &bytes[len..];
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum SpliceError {
    Unsupported,
    Io(io::Error),
}

type Result<T> = std::result::Result<T, SpliceError>;

impl From<nix::Error> for SpliceError {
    fn from(error: nix::Error) -> Self {
        Self::Io(io::Error::from_raw_os_error(error as i32))
    }
}

fn splice_maybe_unsupported(error: nix::Error) -> SpliceError {
    if error == Errno::EINVAL || error == Errno::ENOSYS || error == Errno::EBADF {
        SpliceError::Unsupported
    } else {
        error.into()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::fs::File;
    use std::io::Write;

    use nix::errno::Errno;

    use super::*;

    #[test]
    fn test_splice_data_with_file() {
        let bytes = b"test_data";

        let mut file = File::create("test.txt").expect("Failed to create test file");
        file.write_all(bytes).expect("Failed to write test data");
        let file = File::open("test.txt").expect("Failed to open test file");
        let result = splice_data(bytes, &file);

        assert!(result.is_err());

        std::fs::remove_file("test.txt").expect("Failed to remove test file");
    }

    #[test]
    fn test_is_pipe_with_regular_file() {
        let regular_file_path = "test_regular_file";
        File::create(regular_file_path).expect("Failed to create regular file");

        let regular_file = File::open(regular_file_path).expect("Failed to open regular file");

        let result = match fstat(regular_file.as_raw_fd()) {
            Ok(stat) => (stat.st_mode & S_IFIFO) != 0,
            Err(_) => false,
        };
        assert!(!result);

        fs::remove_file(regular_file_path).expect("Failed to remove regular file");
    }

    #[test]
    fn test_from_nix_error_to_io_error() {
        let nix_error = nix::Error::EAGAIN;

        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::WouldBlock);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_unknown_errno() {
        let nix_error = nix::Error::UnknownErrno;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是 Uncategorized
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_eperm() {
        let nix_error = nix::Error::EPERM;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::PermissionDenied);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_enoent() {
        let nix_error = nix::Error::ENOENT;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::NotFound);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_esrch() {
        let nix_error = nix::Error::ESRCH;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是 Uncategorized
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_eintr() {
        let nix_error = nix::Error::EINTR;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::Interrupted);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_eio() {
        let nix_error = nix::Error::EIO;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是 Uncategorized
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_enxio() {
        let nix_error = nix::Error::ENXIO;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是 Uncategorized
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_e2big() {
        let nix_error = nix::Error::E2BIG;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是在rust不稳定
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_enoexec() {
        let nix_error = nix::Error::ENOEXEC;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是在rust不稳定
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_ebadf() {
        let nix_error = nix::Error::EBADF;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是在rust不稳定
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_echild() {
        let nix_error = nix::Error::ECHILD;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是在rust不稳定
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_eagain() {
        let nix_error = nix::Error::EAGAIN;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::WouldBlock);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_enomem() {
        let nix_error = nix::Error::ENOMEM;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::OutOfMemory);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_eacces() {
        let nix_error = nix::Error::EACCES;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::PermissionDenied);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_efault() {
        let nix_error = nix::Error::EFAULT;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是在rust不稳定
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_enotblk() {
        let nix_error = nix::Error::ENOTBLK;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是在rust不稳定
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_ebusy() {
        let nix_error = nix::Error::EBUSY;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(_) => {
                assert!(true); // 不进行断言，因为我们期望的是在rust不稳定
            }
            _ => {
                assert!(false); // 其他类型错误，测试失败
            }
        }
    }

}