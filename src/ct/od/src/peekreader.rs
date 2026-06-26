/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! 包含 `PeekRead` trait 和实现它的 `PeekReader` 类型。

use std::io;
use std::io::{Read, Write};

use crate::multifilereader::HasError;

/// 提供一个可以预览流数据而不实际读取的 trait
///
/// 类似于 `std::io::Read`，它允许从流中读取数据，
/// 但增加了一个功能：可以保留部分返回的数据，
/// 这些数据在后续调用中仍然可用。
///
pub trait PeekRead {
    /// 将数据读入缓冲区
    ///
    /// 用数据填充 `out`。`out` 的最后 `peek_size` 个字节用于存储
    /// 在后续调用中仍然可用的数据。
    /// `peek_size` 必须小于或等于 `out` 的大小。
    ///
    /// 返回一个元组，第一个数字是从流中读取的字节数，
    /// 第二个数字是额外读取的字节数。这两个数字都可能为零。
    /// 也可能返回一个错误。
    ///
    /// 实现这个 trait 的类型通常也会实现 `std::io::Read`。
    ///
    /// # Panic（异常）
    /// 如果 `peek_size` 大于 `out` 的大小，可能会触发 panic
    fn peek_read(&mut self, out: &mut [u8], peek_size: usize) -> io::Result<(usize, usize)>;
}

/// `std::io::Read` 的包装器，允许预览将要读取的数据
pub struct PeekReader<R> {
    inner: R,
    temp_buffer: Vec<u8>,
}

impl<R> PeekReader<R> {
    /// 创建一个新的 `PeekReader` 包装 `inner`
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            temp_buffer: Vec::new(),
        }
    }
}

impl<R: Read> PeekReader<R> {
    /// 从临时缓冲区读取数据
    fn read_from_tempbuffer(&mut self, mut out: &mut [u8]) -> usize {
        match out.write(self.temp_buffer.as_mut_slice()) {
            Ok(n) => {
                self.temp_buffer.drain(..n);
                n
            }
            Err(_) => 0,
        }
    }

    /// 写入数据到临时缓冲区
    fn write_to_tempbuffer(&mut self, bytes: &[u8]) {
        // 如果临时缓冲区不为空，数据需要插入到前面
        let org_buffer: Vec<_> = self.temp_buffer.drain(..).collect();
        self.temp_buffer.write_all(bytes).unwrap();
        self.temp_buffer.extend(org_buffer);
    }
}

impl<R: Read> Read for PeekReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let start_pos = self.read_from_tempbuffer(out);
        match self.inner.read(&mut out[start_pos..]) {
            Err(e) => Err(e),
            Ok(n) => Ok(n + start_pos),
        }
    }
}

impl<R: Read> PeekRead for PeekReader<R> {
    /// 将数据读入缓冲区
    ///
    /// 参见 `PeekRead::peek_read` 的说明。
    ///
    /// # Panic（异常）
    /// 如果 `peek_size` 大于 `out` 的大小，会触发 panic
    fn peek_read(&mut self, out: &mut [u8], peek_size: usize) -> io::Result<(usize, usize)> {
        assert!(out.len() >= peek_size);
        match self.read(out) {
            Err(e) => Err(e),
            Ok(bytes_in_buffer) => {
                let unused = out.len() - bytes_in_buffer;
                if peek_size <= unused {
                    Ok((bytes_in_buffer, 0))
                } else {
                    let actual_peek_size = peek_size - unused;
                    let real_size = bytes_in_buffer - actual_peek_size;
                    self.write_to_tempbuffer(&out[real_size..bytes_in_buffer]);
                    Ok((real_size, actual_peek_size))
                }
            }
        }
    }
}

impl<R: HasError> HasError for PeekReader<R> {
    fn has_error(&self) -> bool {
        self.inner.has_error()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    #[test]
    fn test_read_normal() {
        let mut sut = PeekReader::new(Cursor::new(&b"abcdefgh"[..]));

        let mut v = [0; 10];
        assert_eq!(sut.read(v.as_mut()).unwrap(), 8);
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0, 0]);
    }

    #[test]
    fn test_peek_read_without_buffer() {
        let mut sut = PeekReader::new(Cursor::new(&b"abcdefgh"[..]));

        let mut v = [0; 10];
        assert_eq!(sut.peek_read(v.as_mut(), 0).unwrap(), (8, 0));
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0, 0]);
    }

    #[test]
    fn test_peek_read_and_read() {
        let mut sut = PeekReader::new(Cursor::new(&b"abcdefghij"[..]));

        let mut v = [0; 8];
        assert_eq!(sut.peek_read(v.as_mut(), 4).unwrap(), (4, 4));
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68]);

        let mut v2 = [0; 8];
        assert_eq!(sut.read(v2.as_mut()).unwrap(), 6);
        assert_eq!(v2, [0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0, 0]);
    }

    #[test]
    fn test_peek_read_multiple_times() {
        let mut sut = PeekReader::new(Cursor::new(&b"abcdefghij"[..]));

        let mut s1 = [0; 8];
        assert_eq!(sut.peek_read(s1.as_mut(), 4).unwrap(), (4, 4));
        assert_eq!(s1, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68]);

        let mut s2 = [0; 8];
        assert_eq!(sut.peek_read(s2.as_mut(), 4).unwrap(), (4, 2));
        assert_eq!(s2, [0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0, 0]);

        let mut s3 = [0; 8];
        assert_eq!(sut.peek_read(s3.as_mut(), 4).unwrap(), (2, 0));
        assert_eq!(s3, [0x69, 0x6a, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn test_peek_read_and_read_with_small_buffer() {
        let mut sut = PeekReader::new(Cursor::new(&b"abcdefghij"[..]));

        let mut v = [0; 8];
        assert_eq!(sut.peek_read(v.as_mut(), 4).unwrap(), (4, 4));
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68]);

        let mut v2 = [0; 2];
        assert_eq!(sut.read(v2.as_mut()).unwrap(), 2);
        assert_eq!(v2, [0x65, 0x66]);
        assert_eq!(sut.read(v2.as_mut()).unwrap(), 2);
        assert_eq!(v2, [0x67, 0x68]);
        assert_eq!(sut.read(v2.as_mut()).unwrap(), 2);
        assert_eq!(v2, [0x69, 0x6a]);
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_peek_read_with_smaller_buffer() {
        let mut sut = PeekReader::new(Cursor::new(&b"abcdefghij"[..]));

        let mut v = [0; 8];
        assert_eq!(sut.peek_read(v.as_mut(), 4).unwrap(), (4, 4));
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68]);

        let mut v2 = [0; 2];
        assert_eq!(sut.peek_read(v2.as_mut(), 2).unwrap(), (0, 2));
        assert_eq!(v2, [0x65, 0x66]);
        assert_eq!(sut.peek_read(v2.as_mut(), 0).unwrap(), (2, 0));
        assert_eq!(v2, [0x65, 0x66]);
        assert_eq!(sut.peek_read(v2.as_mut(), 0).unwrap(), (2, 0));
        assert_eq!(v2, [0x67, 0x68]);
        assert_eq!(sut.peek_read(v2.as_mut(), 0).unwrap(), (2, 0));
        assert_eq!(v2, [0x69, 0x6a]);
    }

    #[test]
    fn test_peek_read_peek_with_larger_peek_buffer() {
        let mut sut = PeekReader::new(Cursor::new(&b"abcdefghij"[..]));

        let mut v = [0; 8];
        assert_eq!(sut.peek_read(v.as_mut(), 4).unwrap(), (4, 4));
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68]);

        let mut v2 = [0; 8];
        assert_eq!(sut.peek_read(v2.as_mut(), 8).unwrap(), (0, 6));
        assert_eq!(v2, [0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0, 0]);
    }
}
