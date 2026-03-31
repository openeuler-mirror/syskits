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

    #[test]
    fn test_from_nix_error_to_io_error_eexist() {
        let nix_error = nix::Error::EEXIST;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::AlreadyExists);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_exdev() {
        let nix_error = nix::Error::EXDEV;
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
    fn test_from_nix_error_to_io_error_enodev() {
        let nix_error = nix::Error::ENODEV;
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
    fn test_from_nix_error_to_io_error_enotdir() {
        let nix_error = nix::Error::ENOTDIR;
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
    fn test_from_nix_error_to_io_error_eisdir() {
        let nix_error = nix::Error::EISDIR;
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
    fn test_from_nix_error_to_io_error_einval() {
        let nix_error = nix::Error::EINVAL;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::InvalidInput);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_enfile() {
        let nix_error = nix::Error::ENFILE;
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
    fn test_from_nix_error_to_io_error_emfile() {
        let nix_error = nix::Error::EMFILE;
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
    fn test_from_nix_error_to_io_error_enotty() {
        let nix_error = nix::Error::ENOTTY;
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
    fn test_from_nix_error_to_io_error_etxtbsy() {
        let nix_error = nix::Error::ETXTBSY;
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
    fn test_from_nix_error_to_io_error_efbig() {
        let nix_error = nix::Error::EFBIG;
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
    fn test_from_nix_error_to_io_error_enospc() {
        let nix_error = nix::Error::ENOSPC;
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
    fn test_from_nix_error_to_io_error_espipe() {
        let nix_error = nix::Error::ESPIPE;
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
    fn test_from_nix_error_to_io_error_erofs() {
        let nix_error = nix::Error::EROFS;
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
    fn test_from_nix_error_to_io_error_emlink() {
        let nix_error = nix::Error::EMLINK;
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
    fn test_from_nix_error_to_io_error_epipe() {
        let nix_error = nix::Error::EPIPE;
        let custom_error: SpliceError = nix_error.into();
        match custom_error {
            SpliceError::Io(inner_io_error) => {
                assert_eq!(inner_io_error.kind(), io::ErrorKind::BrokenPipe);
            }
            _ => {}
        }
    }

    #[test]
    fn test_from_nix_error_to_io_error_edom() {
        let nix_error = nix::Error::EDOM;
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
    fn test_from_nix_error_to_io_error_erange() {
        let nix_error = nix::Error::ERANGE;
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
    fn test_from_nix_error_to_io_error_edeadlk() {
        let nix_error = nix::Error::EDEADLK;
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
    fn test_from_nix_error_to_io_error_enametoolong() {
        let nix_error = nix::Error::ENAMETOOLONG;
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
    fn test_from_nix_error_to_io_error_enolck() {
        let nix_error = nix::Error::ENOLCK;
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
    fn test_from_nix_error_to_io_error_enosys() {
        let nix_error = nix::Error::ENOSYS;
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
    fn test_from_nix_error_to_io_error_enotempty() {
        let nix_error = nix::Error::ENOTEMPTY;
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
    fn test_from_nix_error_to_io_error_eloop() {
        let nix_error = nix::Error::ELOOP;
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
    fn test_from_nix_error_to_io_error_enomsg() {
        let nix_error = nix::Error::ENOMSG;
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
    fn test_from_nix_error_to_io_error_eidrm() {
        let nix_error = nix::Error::EIDRM;
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
    fn test_from_nix_error_to_io_error_echrng() {
        let nix_error = nix::Error::ECHRNG;
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
    fn test_from_nix_error_to_io_error_el2nsync() {
        let nix_error = nix::Error::EL2NSYNC;
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
    fn test_from_nix_error_to_io_error_el3hlt() {
        let nix_error = nix::Error::EL3HLT;
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
    fn test_from_nix_error_to_io_error_el3rst() {
        let nix_error = nix::Error::EL3RST;
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
    fn test_from_nix_error_to_io_error_elnrng() {
        let nix_error = nix::Error::ELNRNG;
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
    fn test_from_nix_error_to_io_error_eunatch() {
        let nix_error = nix::Error::EUNATCH;
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
    fn test_from_nix_error_to_io_error_enocsi() {
        let nix_error = nix::Error::ENOCSI;
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
    fn test_from_nix_error_to_io_error_el2hlt() {
        let nix_error = nix::Error::EL2HLT;
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
    fn test_from_nix_error_to_io_error_ebade() {
        let nix_error = nix::Error::EBADE;
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
    fn test_from_nix_error_to_io_error_ebadr() {
        let nix_error = nix::Error::EBADR;
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
    fn test_from_nix_error_to_io_error_exfull() {
        let nix_error = nix::Error::EXFULL;
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
    fn test_from_nix_error_to_io_error_enoano() {
        let nix_error = nix::Error::ENOANO;
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
    fn test_from_nix_error_to_io_error_ebadrqc() {
        let nix_error = nix::Error::EBADRQC;
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
    fn test_from_nix_error_to_io_error_ebadslt() {
        let nix_error = nix::Error::EBADSLT;
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
    fn test_from_nix_error_to_io_error_ebfont() {
        let nix_error = nix::Error::EBFONT;
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
    fn test_from_nix_error_to_io_error_enostr() {
        let nix_error = nix::Error::ENOSTR;
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
    fn test_from_nix_error_to_io_error_enodata() {
        let nix_error = nix::Error::ENODATA;
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
    fn test_from_nix_error_to_io_error_etime() {
        let nix_error = nix::Error::ETIME;
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
    fn test_from_nix_error_to_io_error_enosr() {
        let nix_error = nix::Error::ENOSR;
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
    fn test_from_nix_error_to_io_error_enonet() {
        let nix_error = nix::Error::ENONET;
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
    fn test_from_nix_error_to_io_error_enopkg() {
        let nix_error = nix::Error::ENOPKG;
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
    fn test_from_nix_error_to_io_error_eremote() {
        let nix_error = nix::Error::EREMOTE;
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
    fn test_from_nix_error_to_io_error_enolink() {
        let nix_error = nix::Error::ENOLINK;
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
    fn test_from_nix_error_to_io_error_eadv() {
        let nix_error = nix::Error::EADV;
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
    fn test_from_nix_error_to_io_error_esrmnt() {
        let nix_error = nix::Error::ESRMNT;
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
    fn test_from_nix_error_to_io_error_ecomm() {
        let nix_error = nix::Error::ECOMM;
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
    fn test_from_nix_error_to_io_error_eproto() {
        let nix_error = nix::Error::EPROTO;
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
    fn test_from_nix_error_to_io_error_emultihop() {
        let nix_error = nix::Error::EMULTIHOP;
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
    fn test_from_nix_error_to_io_error_edotdot() {
        let nix_error = nix::Error::EDOTDOT;
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
    fn test_from_nix_error_to_io_error_ebadmsg() {
        let nix_error = nix::Error::EBADMSG;
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
    fn test_from_nix_error_to_io_error_eoverflow() {
        let nix_error = nix::Error::EOVERFLOW;
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
    fn test_from_nix_error_to_io_error_enotuniq() {
        let nix_error = nix::Error::ENOTUNIQ;
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
    fn test_from_nix_error_to_io_error_ebadfd() {
        let nix_error = nix::Error::EBADFD;
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
    fn test_from_nix_error_to_io_error_eremchg() {
        let nix_error = nix::Error::EREMCHG;
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
    fn test_from_nix_error_to_io_error_elibacc() {
        let nix_error = nix::Error::ELIBACC;
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
    fn test_from_nix_error_to_io_error_elibbad() {
        let nix_error = nix::Error::ELIBBAD;
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
    fn test_from_nix_error_to_io_error_elibscn() {
        let nix_error = nix::Error::ELIBSCN;
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
    fn test_from_nix_error_to_io_error_elibmax() {
        let nix_error = nix::Error::ELIBMAX;
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
    fn test_from_nix_error_to_io_error_elibexec() {
        let nix_error = nix::Error::ELIBEXEC;
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
    fn test_from_nix_error_to_io_error_eilseq() {
        let nix_error = nix::Error::EILSEQ;
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
    fn test_from_nix_error_to_io_error_erestart() {
        let nix_error = nix::Error::ERESTART;
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
    fn test_from_nix_error_to_io_error_emsgsize() {
        let nix_error = nix::Error::EMSGSIZE;
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
    fn test_from_nix_error_to_io_error_eprototype() {
        let nix_error = nix::Error::EPROTOTYPE;
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
    fn test_from_nix_error_to_io_error_enoprotoopt() {
        let nix_error = nix::Error::ENOPROTOOPT;
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
    fn test_from_nix_error_to_io_error_eprotonosupport() {
        let nix_error = nix::Error::EPROTONOSUPPORT;
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
    fn test_from_nix_error_to_io_error_esocktnosupport() {
        let nix_error = nix::Error::ESOCKTNOSUPPORT;
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
    fn test_from_nix_error_to_io_error_eopnotsupp() {
        let nix_error = nix::Error::EOPNOTSUPP;
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
    fn test_maybe_unsupported_with_einval() {
        let error = Errno::EINVAL.into();
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Unsupported
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_enosys() {
        let error = Errno::ENOSYS.into();
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Unsupported
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_ebadf() {
        let error = Errno::EBADF.into();
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Unsupported
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_other_error() {
        let error = nix::Error::E2BIG.into(); // Random error
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Io(_)
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_enomem() {
        let error = Errno::ENOMEM.into(); // Out of memory error
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Io(_)
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_eperm() {
        let error = Errno::EPERM.into(); // Permission denied error
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Io(_)
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_esrch() {
        let error = Errno::ESRCH.into(); // No such process error
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Io(_)
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_efault() {
        let error = Errno::EFAULT.into(); // Bad address error
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Io(_)
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_econnreset() {
        let error = Errno::ECONNRESET.into(); // Connection reset by peer error
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Io(_)
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_eaddrinuse() {
        let error = Errno::EADDRINUSE.into(); // Address already in use error
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Io(_)
        ));
    }

    #[test]
    fn test_maybe_unsupported_with_eintr() {
        let error = Errno::EINTR.into();
        assert!(matches!(
            splice_maybe_unsupported(error),
            SpliceError::Io(_)
        ));
    }
}
