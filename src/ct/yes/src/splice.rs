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

