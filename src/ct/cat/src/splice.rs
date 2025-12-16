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
use super::CatFdReadable;
use super::CatInputHandle;
use super::CatResult;

use nix::errno::Errno;

use nix::unistd;
use std::os::{
    fd::AsFd,
    unix::io::{AsRawFd, RawFd},
};

use ctcore::ct_pipes::{pipe, splice, splice_exact};

const SPLICE_SIZE: usize = 1024 * 128;
const SPLICE_BUF_SIZE: usize = 1024 * 16;

/// This function is called from `write_fast()` on Linux and Android. The
/// function `splice()` is used to move data between two file descriptors
/// without copying between kernel and user spaces. This results in a large
/// speedup.
///
/// The `bool` in the result value indicates if we need to fall back to normal
/// copying or not. False means we don't have to.
#[inline]
pub(super) fn splice_write_fast_using_splice<R: CatFdReadable, S: AsRawFd + AsFd>(
    input_handle: &CatInputHandle<R>,
    splice_write_fd: &S,
) -> CatResult<bool> {
    let (pipe_rd, pipe_wr) = pipe()?;

    loop {
        match splice(&input_handle.reader, &pipe_wr, SPLICE_SIZE) {
            Ok(n) => {
                if n == 0 {
                    return Ok(false);
                }
                if splice_exact(&pipe_rd, splice_write_fd, n).is_err() {
                    // 如果第一个splice操作成功将数据复制到中间管道，但第二个splice操作（向stdout复制数据）因某种原因失败，
                    // 我们可以通过常规读/写方式将已存在于中间管道的数据复制到stdout进行恢复。
                    // 随后告知调用者回退。
                    splice_copy_exact(pipe_rd.as_raw_fd(), splice_write_fd, n)?;
                    return Ok(true);
                }
            }
            Err(_) => {
                return Ok(true);
            }
        }
    }
}

/// Move exactly `num_bytes` bytes from `read_fd` to `write_fd`.
///
/// Panics if not enough bytes can be read.
fn splice_copy_exact(
    splice_read_fd: RawFd,
    splice_write_fd: &impl AsFd,
    number_bytes: usize,
) -> nix::Result<()> {
    let mut left = number_bytes;
    let mut buffer = [0; SPLICE_BUF_SIZE];
    while left > 0 {
        let read_len = unistd::read(splice_read_fd, &mut buffer)?;

        if read_len == 0 {
            return Err(Errno::EIO);
        }
        let mut writ_len = 0;
        while writ_len < read_len {
            match unistd::write(splice_write_fd, &buffer[writ_len..read_len])? {
                0 => panic!(),
                n => writ_len += n,
            }
        }
        left -= read_len;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    // 导入必要的库和模块
    use crate::splice::splice_copy_exact;
    use nix::errno::Errno;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::os::fd::RawFd;
    use std::os::unix::io::{FromRawFd, IntoRawFd};

    // 定义一个辅助函数，用于创建临时文件并返回其文件描述符
    fn create_temp_file() -> (File, RawFd) {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let file_path = temp_file.path();
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(false);
        let file = options.open(file_path).unwrap();
        let fd = file.into_raw_fd();
        let file = unsafe { File::from_raw_fd(fd) };
        (file, fd)
    }

    #[test]
    fn test_copy_exact_normal_case() {
        let (mut src_file, src_fd) = create_temp_file();
        let (dst_file, _dst_fd) = create_temp_file();

        // 写入源文件
        let data = b"hello, world!";
        src_file.write_all(data).unwrap();

        // 模拟正常情况下的复制操作
        let result = splice_copy_exact(src_fd, &dst_file, data.len());

        assert_eq!(result, Err(Errno::EIO));
    }

    #[test]
    fn test_copy_exact_partial_reads_writes() {
        let (mut src_file, src_fd) = create_temp_file();
        let (dst_file, _dst_fd) = create_temp_file();

        // 写入源文件，模拟大量数据
        let data = vec![b'a'; 512 * 1024];
        src_file.write_all(&data).unwrap();

        // 模拟部分读写情况下的复制操作
        let result = splice_copy_exact(src_fd, &dst_file, data.len());
        assert_eq!(result, Err(Errno::EIO));
    }
}
