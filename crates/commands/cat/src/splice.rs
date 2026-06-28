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

/// This function is called from `write_fast()` on Linux. The
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
                if let Err(err) = splice_exact(&pipe_rd, splice_write_fd, n) {
                    // 如果第一个splice操作成功将数据复制到中间管道，但第二个splice操作（向stdout复制数据）因某种原因失败，
                    // 我们可以通过常规读/写方式将已存在于中间管道的数据复制到stdout进行恢复。
                    // 随后告知调用者是“回退”还是直接报错。
                    splice_copy_exact(pipe_rd.as_raw_fd(), splice_write_fd, n)?;
                    if is_splice_unsupported(err) {
                        return Ok(true);
                    }
                    return Err(err.into());
                }
            }
            Err(err) => {
                if is_splice_retryable(err) {
                    continue;
                }
                if is_splice_unsupported(err) {
                    return Ok(true);
                }
                return Err(err.into());
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

/// 判断给定的 splice 错误是否可以重试（例如被中断）。
fn is_splice_retryable(err: Errno) -> bool {
    matches!(err, Errno::EINTR)
}

/// 判断给定的 splice 错误是否表示"该场景不支持 splice"。
fn is_splice_unsupported(err: Errno) -> bool {
    // 注意: EOPNOTSUPP 和 ENOTSUP 在某些平台上是相同的值，
    // 所以我们只匹配 EOPNOTSUPP 来避免不可达模式警告
    matches!(err, Errno::EINVAL | Errno::EOPNOTSUPP | Errno::ENOSYS)
}
#[cfg(test)]
mod tests {
    // 导入必要的库和模块
    use crate::{
        CatInputHandle,
        splice::{is_splice_retryable, is_splice_unsupported, splice_copy_exact},
    };
    use nix::errno::Errno;
    use nix::unistd::{pipe, read, write};
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd, RawFd};
    use std::os::unix::io::FromRawFd;
    use tempfile::tempfile;

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

    #[test]
    fn test_splice_copy_exact_successful_transfer() {
        let (read_fd, write_fd) = pipe().unwrap();
        let (dest_read, dest_write) = pipe().unwrap();
        let payload = b"splice-data";

        write(&write_fd, payload).unwrap();
        drop(write_fd);

        let dest_writer = unsafe { File::from_raw_fd(dest_write.into_raw_fd()) };
        splice_copy_exact(read_fd.as_raw_fd(), &dest_writer, payload.len()).unwrap();

        let mut reader = unsafe { File::from_raw_fd(dest_read.into_raw_fd()) };
        let mut buffer = vec![0_u8; payload.len()];
        reader.read_exact(&mut buffer).unwrap();
        assert_eq!(&buffer, payload);

        drop(reader);
        drop(dest_writer);
    }

    #[test]
    fn test_is_splice_retryable_and_unsupported() {
        assert!(is_splice_retryable(Errno::EINTR));
        assert!(!is_splice_retryable(Errno::EINVAL));

        assert!(is_splice_unsupported(Errno::EINVAL));
        assert!(is_splice_unsupported(Errno::ENOSYS));
        assert!(!is_splice_unsupported(Errno::EPIPE));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_splice_write_fast_completes_without_fallback() {
        use super::splice_write_fast_using_splice;
        use ctcore::ct_pipes::pipe;

        let (input_reader, mut input_writer) = pipe().unwrap();
        let payload = b"hello splice";
        input_writer.write_all(payload).unwrap();
        drop(input_writer);

        let mut output_file = tempfile().unwrap();
        let handle = CatInputHandle {
            reader: input_reader,
            is_interactive: false,
        };

        let need_fallback = splice_write_fast_using_splice(&handle, &output_file).unwrap();
        assert!(!need_fallback, "正常情况下不应触发回退逻辑");

        output_file.seek(SeekFrom::Start(0)).unwrap();
        let mut buffer = Vec::new();
        output_file.read_to_end(&mut buffer).unwrap();
        assert_eq!(buffer, payload);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_splice_write_fast_triggers_fallback_on_unsupported() {
        use super::splice_write_fast_using_splice;
        use ctcore::ct_pipes::pipe;

        let (input_reader, mut input_writer) = pipe().unwrap();
        let value: u64 = 42;
        input_writer
            .write_all(&value.to_ne_bytes())
            .expect("写入管道失败");
        drop(input_writer);

        let handle = CatInputHandle {
            reader: input_reader,
            is_interactive: false,
        };

        let fd = unsafe { nix::libc::eventfd(0, nix::libc::EFD_NONBLOCK) };
        assert!(fd >= 0, "eventfd 创建失败: {}", fd);
        let event_fd = unsafe { OwnedFd::from_raw_fd(fd) };

        let need_fallback =
            splice_write_fast_using_splice(&handle, &event_fd).expect("调用应成功返回");
        let mut buffer = [0u8; 8];
        let read_len = read(event_fd.as_raw_fd(), &mut buffer).expect("读取 eventfd 失败");
        assert_eq!(read_len, 8);
        assert_eq!(u64::from_ne_bytes(buffer), value);
        if !need_fallback {
            // 某些内核支持直接 splice 到 eventfd，此时无需回退。
            return;
        }
    }
}
