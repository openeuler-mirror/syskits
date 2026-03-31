/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! Thin pipe-related wrappers around functions from the `nix` crate.
use std::fs::File;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::io::IoSlice;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::os::unix::io::AsRawFd;

#[cfg(any(target_os = "linux", target_os = "android"))]
use nix::fcntl::SpliceFFlags;

pub use nix::{Error, Result};

/// A wrapper around [`nix::unistd::pipe`] that ensures the pipe is cleaned up.
///
/// Returns two `File` objects: everything written to the second can be read
/// from the first.
pub fn pipe() -> Result<(File, File)> {
    let (read, write) = nix::unistd::pipe()?;
    Ok((File::from(read), File::from(write)))
}

/// Less noisy wrapper around [`nix::fcntl::splice`].
///
/// Up to `len` bytes are moved from `source` to `target`. Returns the number
/// of successfully moved bytes.
///
/// At least one of `source` and `target` must be some sort of pipe.
/// To get around this requirement, consider splicing from your source into
/// a [`pipe`] and then from the pipe into your target (with `splice_exact`):
/// this is still very efficient.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn splice(source: &impl AsRawFd, target: &impl AsRawFd, len: usize) -> Result<usize> {
    nix::fcntl::splice(
        source.as_raw_fd(),
        None,
        target.as_raw_fd(),
        None,
        len,
        SpliceFFlags::empty(),
    )
}

/// Splice wrapper which fully finishes the write.
///
/// Exactly `len` bytes are moved from `source` into `target`.
///
/// Panics if `source` runs out of data before `len` bytes have been moved.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn splice_exact(source: &impl AsRawFd, target: &impl AsRawFd, len: usize) -> Result<()> {
    let mut left = len;
    while left != 0 {
        let written = splice(source, target, left)?;
        assert_ne!(written, 0, "unexpected end of data");
        left -= written;
    }
    Ok(())
}

/// Copy data from `bytes` into `target`, which must be a pipe.
///
/// Returns the number of successfully copied bytes.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn vmsplice(target: &impl AsRawFd, bytes: &[u8]) -> Result<usize> {
    nix::fcntl::vmsplice(
        target.as_raw_fd(),
        &[IoSlice::new(bytes)],
        SpliceFFlags::empty(),
    )
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use std::io::{Read, Write};
//     use tempfile::tempfile;
//
//     #[test]
//     fn test_pipe() {
//         // 创建管道
//         let (mut read_pipe, mut write_pipe) = pipe().expect("Failed to create pipe");
//
//         // 写入数据到管道
//         const DATA: &[u8] = b"Hello, world!";
//         write_pipe.write_all(DATA).expect("Failed to write to pipe");
//
//         // 从管道读取数据
//         let mut read_buffer = Vec::new();
//         read_pipe
//             .read_to_end(&mut read_buffer)
//             .expect("Failed to read from pipe");
//
//         // 验证读取的数据与写入的数据一致
//         assert_eq!(read_buffer, DATA);
//     }
// }
