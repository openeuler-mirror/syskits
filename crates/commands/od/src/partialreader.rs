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

use std::cmp;
use std::io;
use std::io::Read;

use crate::multifilereader::HasError;

/// 当需要跳过大量字节时，会将数据读入一个动态分配的缓冲区。
/// 该缓冲区的大小会被限制在这个值以内。
const MAX_SKIP_BUFFER: usize = 16 * 1024;

/// `std::io::Read` 的包装器，可以：
/// 1. 在读取输入开始时跳过指定数量的字节
/// 2. 限制返回的字节数到特定值
pub struct PartialReader<R> {
    inner: R,
    skip: u64,
    limit: Option<u64>,
}

impl<R> PartialReader<R> {
    /// 创建一个新的 `PartialReader` 实例
    ///
    /// # 参数
    /// * `inner` - 被包装的读取器
    /// * `skip` - 需要跳过的字节数
    /// * `limit` - 限制读取的字节数，设置为 `None` 表示无限制
    pub fn new(inner: R, skip: u64, limit: Option<u64>) -> Self {
        Self { inner, skip, limit }
    }
}

impl<R: Read> Read for PartialReader<R> {
    /// 从输入流中读取数据
    ///
    /// 该函数会：
    /// 1. 如果需要，先跳过指定数量的字节
    /// 2. 如果设置了限制，只读取限制内的字节数
    /// 3. 否则读取请求的所有字节
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // 如果需要跳过字节，先处理跳过
        if self.skip > 0 {
            let mut bytes = [0; MAX_SKIP_BUFFER];
            while self.skip > 0 {
                let skip_count = cmp::min(self.skip as usize, MAX_SKIP_BUFFER);
                match self.inner.read(&mut bytes[..skip_count])? {
                    0 => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "tried to skip past end of input",
                        ));
                    }
                    n => self.skip -= n as u64,
                }
            }
        }

        // 根据是否有限制选择读取方式
        match self.limit {
            None => self.inner.read(out),
            Some(0) => Ok(0),
            Some(ref mut limit) => {
                let slice = if *limit > (out.len() as u64) {
                    out
                } else {
                    &mut out[0..(*limit as usize)]
                };
                match self.inner.read(slice) {
                    Err(e) => Err(e),
                    Ok(r) => {
                        *limit -= r as u64;
                        Ok(r)
                    }
                }
            }
        }
    }
}

impl<R: HasError> HasError for PartialReader<R> {
    fn has_error(&self) -> bool {
        self.inner.has_error()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mockstream::*;
    use std::io::{Cursor, ErrorKind, Read};

    #[test]
    fn test_read_without_limits() {
        let mut v = [0; 10];
        let mut sut = PartialReader::new(Cursor::new(&b"abcdefgh"[..]), 0, None);

        assert_eq!(sut.read(v.as_mut()).unwrap(), 8);
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0, 0]);
    }

    #[test]
    fn test_read_without_limits_with_error() {
        let mut v = [0; 10];
        let f = OdFailingMockStream::new(ErrorKind::PermissionDenied, "No access", 3);
        let mut sut = PartialReader::new(f, 0, None);

        let error = sut.read(v.as_mut()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "No access");
    }

    #[test]
    fn test_read_skipping_bytes() {
        let mut v = [0; 10];
        let mut sut = PartialReader::new(Cursor::new(&b"abcdefgh"[..]), 2, None);

        assert_eq!(sut.read(v.as_mut()).unwrap(), 6);
        assert_eq!(v, [0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0, 0, 0, 0]);
    }

    #[test]
    fn test_read_skipping_all() {
        let mut v = [0; 10];
        let mut sut = PartialReader::new(Cursor::new(&b"abcdefgh"[..]), 20, None);

        let error = sut.read(v.as_mut()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn test_read_skipping_with_error() {
        let mut v = [0; 10];
        let f = OdFailingMockStream::new(ErrorKind::PermissionDenied, "No access", 3);
        let mut sut = PartialReader::new(f, 2, None);

        let error = sut.read(v.as_mut()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "No access");
    }

    #[test]
    fn test_read_skipping_with_two_reads_during_skip() {
        let mut v = [0; 10];
        let c = Cursor::new(&b"a"[..]).chain(Cursor::new(&b"bcdefgh"[..]));
        let mut sut = PartialReader::new(c, 2, None);

        assert_eq!(sut.read(v.as_mut()).unwrap(), 6);
        assert_eq!(v, [0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0, 0, 0, 0]);
    }

    #[test]
    fn test_read_skipping_huge_number() {
        let mut v = [0; 10];
        // test if it does not eat all memory....
        let mut sut = PartialReader::new(
            Cursor::new(&b"abcdefgh"[..]),
            usize::max_value() as u64,
            None,
        );

        sut.read(v.as_mut()).unwrap_err();
    }

    #[test]
    fn test_read_limiting_all() {
        let mut v = [0; 10];
        let mut sut = PartialReader::new(Cursor::new(&b"abcdefgh"[..]), 0, Some(0));

        assert_eq!(sut.read(v.as_mut()).unwrap(), 0);
    }

    #[test]
    fn test_read_limiting() {
        let mut v = [0; 10];
        let mut sut = PartialReader::new(Cursor::new(&b"abcdefgh"[..]), 0, Some(6));

        assert_eq!(sut.read(v.as_mut()).unwrap(), 6);
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0, 0, 0, 0]);
    }

    #[test]
    fn test_read_limiting_with_error() {
        let mut v = [0; 10];
        let f = OdFailingMockStream::new(ErrorKind::PermissionDenied, "No access", 3);
        let mut sut = PartialReader::new(f, 0, Some(6));

        let error = sut.read(v.as_mut()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "No access");
    }

    #[test]
    fn test_read_limiting_with_large_limit() {
        let mut v = [0; 10];
        let mut sut = PartialReader::new(Cursor::new(&b"abcdefgh"[..]), 0, Some(20));

        assert_eq!(sut.read(v.as_mut()).unwrap(), 8);
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0, 0]);
    }

    #[test]
    fn test_read_limiting_with_multiple_reads() {
        let mut v = [0; 3];
        let mut sut = PartialReader::new(Cursor::new(&b"abcdefgh"[..]), 0, Some(6));

        assert_eq!(sut.read(v.as_mut()).unwrap(), 3);
        assert_eq!(v, [0x61, 0x62, 0x63]);
        assert_eq!(sut.read(v.as_mut()).unwrap(), 3);
        assert_eq!(v, [0x64, 0x65, 0x66]);
        assert_eq!(sut.read(v.as_mut()).unwrap(), 0);
    }

    #[test]
    fn test_read_skipping_and_limiting() {
        let mut v = [0; 10];
        let mut sut = PartialReader::new(Cursor::new(&b"abcdefgh"[..]), 2, Some(4));

        assert_eq!(sut.read(v.as_mut()).unwrap(), 4);
        assert_eq!(v, [0x63, 0x64, 0x65, 0x66, 0, 0, 0, 0, 0, 0]);
    }
}
