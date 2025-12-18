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

#[cfg(any(target_os = "linux", target_os = "android"))]
use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Read};
#[cfg(unix)]
use std::io::{Seek, SeekFrom};
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::os::unix::io::AsRawFd;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[cfg(any(target_os = "linux", target_os = "android"))]
use libc::S_IFIFO;
#[cfg(unix)]
use libc::{sysconf, S_IFREG, _SC_PAGESIZE};
#[cfg(unix)]
use nix::sys::stat;

#[cfg(any(target_os = "linux", target_os = "android"))]
use ctcore::ct_pipes::{pipe, splice, splice_exact};

// cSpell:ignore sysconf
use crate::word_count::WcWordCount;

use super::WcWordCountable;

#[cfg(windows)]
const FILE_ATTRIBUTE_ARCHIVE: u32 = 32;
#[cfg(windows)]
const FILE_ATTRIBUTE_NORMAL: u32 = 128;

const COUNT_FAST_BUF_SIZE: usize = 16 * 1024;
#[cfg(target_os = "linux")]
const COUNT_FAST_SPLICE_SIZE: usize = 128 * 1024;

/// 这是一个 Linux 专用函数，用于使用 `splice` 系统调用来计算字节数，这比使用 `read` 更快。
/// 如果出错，它会返回读取到的字节数，因为调用者会返回到更简单的方法。
#[inline]
#[cfg(target_os = "linux")]
fn count_bytes_with_splice(fd: &impl AsRawFd) -> Result<usize, usize> {
    let null_file = OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .map_err(|_| 0_usize)?;
    let null_rdev = stat::fstat(null_file.as_raw_fd())
        .map_err(|_| 0_usize)?
        .st_rdev as libc::dev_t;
    if unsafe { (libc::major(null_rdev), libc::minor(null_rdev)) } != (1, 3) {
        // This is not a proper /dev/null, writing to it is probably bad
        // Bit of an edge case, but it has been known to happen
        return Err(0);
    }
    let (pipe_rd, pipe_wr) = pipe().map_err(|_| 0_usize)?;

    let mut byte_count = 0;
    loop {
        match splice(fd, &pipe_wr, COUNT_FAST_SPLICE_SIZE) {
            Ok(0) => break,
            Ok(res) => {
                byte_count += res;
                // Silent the warning as we want to the error message
                #[allow(clippy::question_mark)]
                if splice_exact(&pipe_rd, &null_file, res).is_err() {
                    return Err(byte_count);
                }
            }
            Err(_) => return Err(byte_count),
        };
    }

    Ok(byte_count)
}

/// 在特殊情况下，我们只需要计算字节数。我们可以做几种优化：
/// 1.在 Unix 上，如果文件是正常的，我们可以简单地 "统计 "文件。
/// 2. 在 Linux 上 -- 如果上述方法不起作用 -- 我们可以使用 splice 来计算字节数。
/// 如果文件是 FIFO 文件，我们可以使用 splice 来计算字节数。
/// 3.在 windows 上，我们可以使用`std::os::windows::fs::MetadataExt`来获取文件大小。
/// 对于普通文件
/// 4. 否则，我们只是正常读取，但不需要计算开销，无需计算行数和字数等其他开销。
#[inline]
pub(crate) fn count_bytes_handle<T: WcWordCountable>(handle: &mut T) -> (usize, Option<io::Error>) {
    #[allow(unused_mut)]
    let mut byte_count = 0;

    #[cfg(unix)]
    {
        let fd = handle.as_raw_fd();
        if let Ok(stat) = stat::fstat(fd) {
            if fd > 0 && (stat.st_mode as libc::mode_t & S_IFREG) != 0 && stat.st_size > 0 {
                let sys_page_size = unsafe { sysconf(_SC_PAGESIZE) as usize };
                if stat.st_size as usize % sys_page_size > 0 {
                    // 常规文件或来自 /proc、/sys 和类似伪文件系统的文件
                    // 大小不是系统页面大小倍数的文件
                    return (stat.st_size as usize, None);
                } else if let Some(file) = handle.inner_file() {
                    // 在某些平台上，"stat.st_blksize "和 "stat.st_size "属于不同类型：i64 与 i32，
                    // 例如苹果硅平台上的 MacOS (aarch64-apple-darwin)、ARM 平台上的 Debian Linux
                    // (aarch64-unknown-linux-gnu)、32 位 i686 目标机等。 [...]
                    #[allow(clippy::unnecessary_cast)]
                    let offset =
                        stat.st_size as i64 - stat.st_size as i64 % (stat.st_blksize as i64 + 1);

                    if let Ok(n) = file.seek(SeekFrom::Start(offset as u64)) {
                        byte_count = n as usize;
                    }
                }
            }
            #[cfg(target_os = "linux")]
            {
                // 否则，如果我们使用的是 Linux 系统，并且我们的文件是一个 FIFO 管道（或 stdin），
                //我们就会使用 splice 来计算字节数。
                if (stat.st_mode as libc::mode_t & S_IFIFO) != 0 {
                    match count_bytes_with_splice(handle) {
                        Ok(n) => return (n, None),
                        Err(n) => byte_count = n,
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    {
        if let Some(file) = handle.inner_file() {
            if let Ok(metadata) = file.metadata() {
                let attributes = metadata.file_attributes();

                if (attributes & FILE_ATTRIBUTE_ARCHIVE) != 0
                    || (attributes & FILE_ATTRIBUTE_NORMAL) != 0
                {
                    return (metadata.file_size() as usize, None);
                }
            }
        }
    }

    // 使用 "read"，但无需计算字数和行数。
    let mut total_bytes_read = byte_count;
    let mut buffer = [0_u8; COUNT_FAST_BUF_SIZE]; // Define BUF_SIZE appropriately elsewhere

    loop {
        let read_result = handle.read(&mut buffer);

        if let Ok(bytes_read) = read_result {
            if bytes_read == 0 {
                return (total_bytes_read, None); // End of stream
            }
            total_bytes_read += bytes_read; // Increment total read bytes
        } else if let Err(error) = read_result {
            if error.kind() == ErrorKind::Interrupted {
                continue; // Interrupted, so continue without exiting
            }
            return (total_bytes_read, Some(error)); // Other errors, return with error
        }
    }
}

/// 返回一个 WordCount，计算通过阅读器读取的字节数、行数和/或以 UTF-8 编码的 Unicode 字符数。
/// 与 wc 的 `-c`、`-l` 和 `-m` 命令行标志相对应。
///
/// # 参数
/// * `R` - 读取 UTF-8 数据流的读取器。
pub(crate) fn count_bytes_chars_lines_from_stream<
    R: Read,
    const COUNT_BYTES: bool,
    const COUNT_CHARS: bool,
    const COUNT_LINES: bool,
>(
    handle: &mut R,
) -> (WcWordCount, Option<io::Error>) {
    /// Mask of the value bits of a continuation byte
    const CONT_MASK: u8 = 0b0011_1111u8;
    /// Value of the tag bits (tag mask is !CONT_MASK) of a continuation byte
    const TAG_CONT_U8: u8 = 0b1000_0000u8;

    let mut total = WcWordCount::default();
    let mut buf = [0; COUNT_FAST_BUF_SIZE];
    loop {
        let read_cnt = handle.read(&mut buf);
        match read_cnt {
            Ok(0) => return (total, None),
            Ok(n) => {
                if COUNT_BYTES {
                    total.bytes += n;
                }
                if COUNT_CHARS {
                    total.chars += buf[..n]
                        .iter()
                        .filter(|&&byte| (byte & !CONT_MASK) != TAG_CONT_U8)
                        .count();
                }
                if COUNT_LINES {
                    total.lines += bytecount::count(&buf[..n], b'\n');
                }
            }
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return (total, Some(e)),
        }
    }
}

