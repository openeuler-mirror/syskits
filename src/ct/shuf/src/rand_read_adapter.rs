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

//! 随机数读取适配器
//!
//! # 功能概述
//! 该模块提供了一个包装器，可以将任何实现了 Read 特征的类型转换为随机数生成器。
//!
//! # 主要组件
//! - `ReadRng`: 从 Read 实现读取随机字节的 RNG 适配器
//! - `ReadError`: 读取错误的封装类型
//!
//! # 使用场景
//! - 从文件读取随机数据
//! - 将任意输入流用作随机源
//! - 在需要 RNG 接口的地方复用现有的 Read 实现

/// 一个包装器，可以将任何实现了 [`std::io::Read`] 的类型转换为随机数生成器，
/// 例如文件。
///
/// 这个适配器最适合用于无限读取器，但这不是必需的。
///
/// 虽然可以在 Unix 系统上用于 `/dev/urandom`，但建议使用 [`OsRng`] 代替。
///
/// # 可能的异常
///
/// `ReadRng` 使用 [`std::io::Read::read_exact`]，它会在中断时重试。
/// 底层读取器的所有其他错误（包括数据不足）只会通过 [`try_fill_bytes`] 报告。
/// 其他 [`RngCore`] 方法在出错时会触发 panic。
///
/// [`OsRng`]: rand::rngs::OsRng
/// [`try_fill_bytes`]: RngCore::try_fill_bytes

use std::fmt;
use std::io::Read;

use rand_core::{Error, RngCore, impls};

#[derive(Debug)]
pub struct ReadRng<R> {
    reader: R,
}

impl<R: Read> ReadRng<R> {
    /// 从一个 `Read` 实现创建新的 `ReadRng`
    pub fn new(r: R) -> Self {
        Self { reader: r }
    }
}

impl<R: Read> RngCore for ReadRng<R> {
    fn next_u32(&mut self) -> u32 {
        impls::next_u32_via_fill(self)
    }

    fn next_u64(&mut self) -> u64 {
        impls::next_u64_via_fill(self)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.try_fill_bytes(dest).unwrap_or_else(|err| {
            panic!("reading random bytes from Read implementation failed; error: {err}");
        });
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Error> {
        if dest.is_empty() {
            return Ok(());
        }
        // Use `std::io::read_exact`, which retries on `ErrorKind::Interrupted`.
        self.reader
            .read_exact(dest)
            .map_err(|e| Error::new(ReadError(e)))
    }
}

/// `ReadRng` 的错误类型
#[derive(Debug)]
pub struct ReadError(std::io::Error);

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ReadError: {}", self.0)
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[cfg(test)]
mod test {
    use std::println;

    use super::ReadRng;
    use rand::RngCore;

    #[test]
    fn test_reader_rng_u64() {
        // transmute from the target to avoid endianness concerns.
        #[rustfmt::skip]
        let v = [0u8, 0, 0, 0, 0, 0, 0, 1,
                 0,   4, 0, 0, 3, 0, 0, 2,
                 5,   0, 0, 0, 0, 0, 0, 0];
        let mut rng = ReadRng::new(&v[..]);

        assert_eq!(rng.next_u64(), 1 << 56);
        assert_eq!(rng.next_u64(), (2 << 56) + (3 << 32) + (4 << 8));
        assert_eq!(rng.next_u64(), 5);
    }

    #[test]
    fn test_reader_rng_u32() {
        let v = [0u8, 0, 0, 1, 0, 0, 2, 0, 3, 0, 0, 0];
        let mut rng = ReadRng::new(&v[..]);

        assert_eq!(rng.next_u32(), 1 << 24);
        assert_eq!(rng.next_u32(), 2 << 16);
        assert_eq!(rng.next_u32(), 3);
    }

    #[test]
    fn test_reader_rng_fill_bytes() {
        let v = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut w = [0u8; 8];

        let mut rng = ReadRng::new(&v[..]);
        rng.fill_bytes(&mut w);

        assert!(v == w);
    }

    #[test]
    fn test_reader_rng_insufficient_bytes() {
        let v = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut w = [0u8; 9];

        let mut rng = ReadRng::new(&v[..]);

        let result = rng.try_fill_bytes(&mut w);
        assert!(result.is_err());
        println!("Error: {}", result.unwrap_err());
    }
}
